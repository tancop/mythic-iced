use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use iced::Task;
use isahc::AsyncReadResponseExt;

use crate::{Message, State, epic, images};

const CONCURRENT_LIMIT: usize = 20;

pub fn handle_loaded(state: &mut State, items: Vec<epic::LibraryItem>) -> Task<Message> {
    state.library_items = Some(items.clone());

    let mut queue: VecDeque<_> = items.into_iter().collect();
    let initial: Vec<_> = queue.drain(..CONCURRENT_LIMIT.min(queue.len())).collect();

    let fetch_task = Task::batch(initial.iter().map(|item| fetch_game_info(state, item)));
    state.pending_items = queue;
    let decode_task = images::decode_visible(state);
    Task::batch([fetch_task, decode_task])
}

pub fn handle_game_info_failed(state: &mut State) -> Task<Message> {
    if let Some(next) = state.pending_items.pop_front() {
        fetch_game_info(state, &next)
    } else {
        Task::none()
    }
}

pub fn handle_game_info(state: &mut State, info: epic::CatalogItem) -> Task<Message> {
    log::info!("Loaded game: {}", &info.title);

    let image_url = info
        .key_images
        .iter()
        .find(|img| img.image_type == "DieselGameBoxTall")
        .or_else(|| info.key_images.first())
        .map(|img| img.url.clone());

    let client = state.http_client.clone();
    let id = info.id.clone();

    if let Some(raw_url) = image_url {
        state.catalog_items.insert(id.clone(), info);

        if state.image_library.get(&id).is_some() {
            // Bytes already cached: decode now (covers app start with
            // a warm cache, where no scroll event may ever fire).
            let decode_task = images::decode_visible(state);
            if let Some(next) = state.pending_items.pop_front() {
                Task::batch([fetch_game_info(state, &next), decode_task])
            } else {
                decode_task
            }
        } else {
            let encoded_url = match url::Url::parse(&raw_url) {
                Ok(parsed) => parsed.to_string(),
                Err(_) => raw_url,
            };

            let download_task = Task::future(async move {
                let mut res = client.get_async(&encoded_url).await.ok();
                if let Some(ref mut res) = res {
                    if let Ok(bytes) = res.bytes().await {
                        match images::resize_image(&bytes) {
                            Ok(processed) => {
                                let bytes = Arc::new(processed);
                                return Message::ImageDownloaded(id, bytes);
                            }
                            Err(e) => {
                                log::error!("Failed to process image for {}: {}", id, e);
                            }
                        }
                    }
                }
                Message::ImageDownloaded(id, Arc::new(Vec::new()))
            });

            if let Some(next) = state.pending_items.pop_front() {
                let fetch_task = fetch_game_info(state, &next);
                Task::batch([download_task, fetch_task])
            } else {
                download_task
            }
        }
    } else {
        log::warn!("No images found for {}", &info.title);
        state.catalog_items.insert(id.clone(), info);
        if let Some(next) = state.pending_items.pop_front() {
            fetch_game_info(state, &next)
        } else {
            Task::none()
        }
    }
}

pub fn handle_scrolled(state: &mut State, offset_y: f32, width: f32, height: f32) -> Task<Message> {
    state.scroll_offset = offset_y;
    state.viewport_width = width;
    state.viewport_height = height;
    evict_stale(state);
    images::decode_visible(state)
}

pub fn load_library(http_client: &isahc::HttpClient, auth_data: &epic::AuthData) -> Task<Message> {
    let client = http_client.clone();
    let access_token = auth_data.access_token.clone();

    Task::future(async move {
        match epic::get_library_items(&client, &access_token).await {
            Ok(items) => Message::LibraryLoaded(items),
            Err(e) => {
                log::error!("Failed to get library items: {}", e);
                Message::Ignored
            }
        }
    })
}

fn fetch_game_info(state: &State, item: &epic::LibraryItem) -> Task<Message> {
    let client = state.http_client.clone();
    let access_token = state.auth_data.as_ref().unwrap().access_token.clone();
    let namespace = item.namespace.clone();
    let catalog_id = item.catalog_item_id.clone();

    Task::future(async move {
        match epic::get_game_info(&client, &access_token, &namespace, &catalog_id).await {
            Ok(info) => Message::GameInfoLoaded(info),
            Err(e) => {
                log::error!("Failed to get game info: {}", e);
                Message::GameInfoFailed
            }
        }
    })
}

pub fn visible_range(state: &State) -> (usize, usize) {
    let Some(items) = &state.library_items else {
        return (0, 0);
    };
    let pitch = crate::ui::library::row_pitch(state.viewport_width);
    let cols = crate::ui::library::cols_for_width(state.viewport_width);
    let total_rows = items.len() / cols + (items.len() % cols != 0) as usize;
    let first_visible_row = (state.scroll_offset / pitch) as usize;
    let visible_rows = (state.viewport_height / pitch) as usize + 1;
    let lo = first_visible_row.saturating_sub(crate::ui::library::BUFFER_ROWS);
    let hi = (first_visible_row + visible_rows + crate::ui::library::BUFFER_ROWS).min(total_rows);
    (lo, hi)
}

fn evict_stale(state: &mut State) {
    let Some(items) = &state.library_items else {
        return;
    };

    let (lo, hi) = visible_range(state);

    let mut visible_ids = HashSet::new();
    for chunk in items
        .chunks(crate::ui::library::cols_for_width(state.viewport_width))
        .skip(lo)
        .take(hi.saturating_sub(lo))
    {
        for item in chunk {
            if let Some(catalog) = state.catalog_items.get(item.catalog_item_id.as_ref()) {
                visible_ids.insert(catalog.id.clone());
            }
        }
    }

    state
        .decoded_images
        .retain(|id, _| visible_ids.contains(id));
}

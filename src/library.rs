use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use iced::Task;
use isahc::AsyncReadResponseExt;

use crate::{Message, State, epic, images, search};

pub fn handle_loaded(
    state: &mut State,
    items: Vec<epic::CatalogItem>,
    purchase_dates: HashMap<String, epic::UtcDateTime>,
) -> Task<Message> {
    let mut items = items;
    for item in &mut items {
        item.search_key = search::build_search_key(&item.title);
    }
    state.catalog_items = Some(items);
    state.total_items = state.catalog_items.as_ref().map(|v| v.len()).unwrap_or(0);
    state.inflight_fetches = 0;
    state.purchase_dates = purchase_dates;

    search::rebuild_order(state);

    let download_tasks = download_missing(state);
    let decode_task = images::decode_visible(state);
    Task::batch([download_tasks, decode_task])
}

// Spawn one thumbnail download per item missing from the disk cache.
// Decodes happen separately in `decode_visible`, so rows still swap in
// atomically once their bytes are cached.
fn download_missing(state: &State) -> Task<Message> {
    let Some(items) = &state.catalog_items else {
        return Task::none();
    };
    let client = state.http_client.clone();

    let tasks = items
        .iter()
        .filter(|item| state.image_library.get(&item.id).is_none())
        .filter_map(|item| {
            let raw_url = item
                .key_images
                .iter()
                .find(|img| img.image_type == "DieselGameBoxTall")
                .or_else(|| item.key_images.first())
                .map(|img| img.url.clone())?;
            let id = item.id.clone();
            let client = client.clone();
            let encoded_url = match url::Url::parse(&raw_url) {
                Ok(parsed) => parsed.to_string(),
                Err(_) => raw_url,
            };
            Some(Task::future(async move {
                let mut res = client.get_async(&encoded_url).await.ok();
                if let Some(ref mut res) = res
                    && let Ok(bytes) = res.bytes().await
                {
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
                Message::ImageDownloaded(id, Arc::new(Vec::new()))
            }))
        })
        .collect::<Vec<_>>();

    Task::batch(tasks)
}

pub fn handle_scrolled(state: &mut State, offset_y: f32, width: f32, height: f32) -> Task<Message> {
    state.scroll_offset = offset_y;
    state.viewport_width = width;
    state.viewport_height = height;
    refresh_visible(state)
}

// Drop off-screen decodes and decode the current window. Used after scrolls
// and after anything else that changes which rows are visible (sort/filter).
pub fn refresh_visible(state: &mut State) -> Task<Message> {
    evict_stale(state);
    images::decode_visible(state)
}

pub fn load_library(http_client: &isahc::HttpClient, auth_data: &epic::AuthData) -> Task<Message> {
    let client = http_client.clone();
    let access_token = auth_data.access_token.clone();

    Task::future(async move {
        match epic::get_library_catalog(&client, &access_token).await {
            Ok(catalog) => Message::LibraryLoaded(catalog.items, catalog.purchase_dates),
            Err(e) => {
                log::error!("Failed to get library items: {}", e);
                Message::Ignored
            }
        }
    })
}

pub fn visible_range(state: &State) -> (usize, usize) {
    if state.catalog_items.is_none() {
        return (0, 0);
    };
    let pitch = crate::ui::library::row_pitch(state.viewport_width);
    let cols = crate::ui::library::cols_for_width(state.viewport_width);
    let total_rows = state.total_items / cols + (state.total_items % cols != 0) as usize;
    let first_visible_row = (state.scroll_offset / pitch) as usize;
    let visible_rows = (state.viewport_height / pitch) as usize + 1;
    let lo = first_visible_row.saturating_sub(crate::ui::library::BUFFER_ROWS);
    let hi = (first_visible_row + visible_rows + crate::ui::library::BUFFER_ROWS).min(total_rows);
    (lo, hi)
}

fn evict_stale(state: &mut State) {
    let Some(items) = &state.catalog_items else {
        return;
    };
    let order = &state.order;

    let (lo, hi) = visible_range(state);

    let mut visible_ids = HashSet::new();
    for chunk in order
        .chunks(crate::ui::library::cols_for_width(state.viewport_width))
        .skip(lo)
        .take(hi.saturating_sub(lo))
    {
        for &index in chunk {
            visible_ids.insert(items[index].id.clone());
        }
    }

    state
        .decoded_images
        .retain(|id, _| visible_ids.contains(id));
}

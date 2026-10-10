use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use iced::Task;
use isahc::AsyncReadResponseExt;

use crate::ui::library as library_ui;
use crate::ui::virtual_grid::{self, GridViewport};
use crate::{Message, Page, State, epic, images, search};

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
    state.inflight_fetches = 0;
    state.purchase_dates = purchase_dates;
    state.focused_game_idx = None;
    state.details_error = false;

    search::rebuild_order(state);

    let download_tasks = download_missing(state);
    let decode_task = images::decode_visible(state);
    let critic_tasks = fetch_critics(state);
    Task::batch([download_tasks, decode_task, critic_tasks])
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

/// Open the detail page for one catalog item, fetching its store-page
/// details unless they are already cached for the namespace.
pub fn handle_game_selected(state: &mut State, index: usize) -> Task<Message> {
    let Some(items) = &state.catalog_items else {
        return Task::none();
    };
    let Some(item) = items.get(index) else {
        return Task::none();
    };
    let namespace = item.namespace.clone();

    state.focused_game_idx = Some(index);
    state.details_error = false;
    state.page = Page::GameDetail;

    if state.game_details.contains_key(&namespace) {
        return Task::none();
    }
    let Some(token) = state
        .auth_data
        .as_ref()
        .map(|auth| auth.access_token.clone())
    else {
        return Task::none();
    };
    let client = state.http_client.clone();

    Task::future(async move {
        match epic::get_game_details(&client, &token, &namespace).await {
            Ok(details) => Message::GameDetailsLoaded {
                namespace,
                details: Box::new(details),
            },
            Err(e) => {
                log::error!("Failed to get game details: {}", e);
                Message::GameDetailsFailed
            }
        }
    })
}

pub fn handle_details_loaded(
    state: &mut State,
    namespace: String,
    details: Box<epic::GameFullDetails>,
) -> Task<Message> {
    state.detail_bodies.insert(
        namespace.clone(),
        crate::ui::game_detail::parse_description(details.description()),
    );
    state.game_details.insert(namespace, *details);
    Task::none()
}

pub fn handle_details_failed(state: &mut State) -> Task<Message> {
    state.details_error = true;
    Task::none()
}

// One critic fetch: Epic first, OpenCritic search+details as fallback.
// `attempts` counts tries so far; transient failures requeue the job.
pub struct CriticJob {
    pub product_id: String,
    pub product_name: String,
    pub id: String,
    pub attempts: u32,
}

// How many critic fetches may be in flight at once. OpenCritic starts
// refusing connections when the whole library hits it at the same time,
// so this stays small; each completion pulls the next job off the queue.
const CRITIC_PARALLELISM: usize = 8;
// Total tries per game (initial attempt + requeues) before giving up.
const CRITIC_MAX_ATTEMPTS: u32 = 3;

// Queue every library item that has a `productId`, then start the first
// few; completions pull the rest off the queue one by one.
fn fetch_critics(state: &mut State) -> Task<Message> {
    let Some(items) = &state.catalog_items else {
        return Task::none();
    };
    state.critic_queue = items
        .iter()
        .filter_map(|item| {
            Some(CriticJob {
                product_id: item.product_id.clone().filter(|id| !id.is_empty())?,
                product_name: item.title.clone(),
                id: item.id.clone(),
                attempts: 0,
            })
        })
        .collect();
    let take = CRITIC_PARALLELISM.min(state.critic_queue.len());
    let initial: Vec<CriticJob> = state.critic_queue.drain(..take).collect();
    let client = state.http_client.clone();
    Task::batch(
        initial
            .into_iter()
            .map(|job| spawn_critic_task(&client, job)),
    )
}

fn spawn_critic_task(client: &isahc::HttpClient, job: CriticJob) -> Task<Message> {
    let client = client.clone();
    let CriticJob {
        product_id,
        product_name,
        id,
        attempts,
    } = job;
    Task::future(async move {
        match epic::get_critic_reviews(&client, &product_id, &product_name).await {
            Ok(score) => Message::CriticLoaded { id, score },
            Err(epic::CriticError::Transient(e)) if attempts + 1 < CRITIC_MAX_ATTEMPTS => {
                log::debug!(
                    "critic fetch throttled for {product_id} (attempt {}), requeueing: {e:#}",
                    attempts + 1
                );
                Message::CriticRetry {
                    product_id,
                    product_name,
                    id,
                    attempts: attempts + 1,
                }
            }
            Err(e) => {
                log::debug!("critic fetch failed for {product_id}: {e:#}");
                Message::CriticLoaded { id, score: None }
            }
        }
    })
}

/// Pull the next queued job now that a slot freed up; `Task::none()` once
/// the queue is drained.
fn pop_critic_task(state: &mut State) -> Task<Message> {
    let client = state.http_client.clone();
    match state.critic_queue.pop_front() {
        Some(job) => spawn_critic_task(&client, job),
        None => Task::none(),
    }
}

pub fn handle_critic_loaded(
    state: &mut State,
    id: String,
    score: Option<epic::CriticScore>,
) -> Task<Message> {
    if let Some(items) = &mut state.catalog_items
        && let Some(item) = items.iter_mut().find(|item| item.id == id)
    {
        item.critic = score;
    }
    // Scores stream in after the library loads; keep the display order
    // current so a critic sort settles as results arrive.
    search::rebuild_order(state);
    pop_critic_task(state)
}

// A throttled fetch gets another chance: push it to the back so other
// games go first (natural spacing without needing a timer), then fill
// the freed slot with the next job.
pub fn handle_critic_retry(
    state: &mut State,
    product_id: String,
    product_name: String,
    id: String,
    attempts: u32,
) -> Task<Message> {
    state.critic_queue.push_back(CriticJob {
        product_id,
        product_name,
        id,
        attempts,
    });
    pop_critic_task(state)
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
    let config = library_ui::GRID_CONFIG;
    virtual_grid::visible_range(
        &config,
        &GridViewport {
            width: state.viewport_width,
            height: state.viewport_height,
            scroll_offset: state.scroll_offset,
        },
        state.order.len(),
    )
}

fn evict_stale(state: &mut State) {
    let Some(items) = &state.catalog_items else {
        return;
    };
    let order = &state.order;

    let (lo, hi) = visible_range(state);

    let mut visible_ids = HashSet::new();
    for chunk in order
        .chunks(virtual_grid::columns(
            &library_ui::GRID_CONFIG,
            state.viewport_width,
        ))
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

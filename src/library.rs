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
    state.details_error = false;

    // Show cached scores instantly so rating sorts settle without waiting
    // on the network; the queues below only fetch what the cache lacks
    // (or everything, when the cache is stale).
    apply_cached_ratings(state);

    search::rebuild_order(state);

    // Persist the fresh library in the background when it differs from disk.
    // `critic` / `user_rating` / `search_key` are `skip`ped in the file, so
    // cloning after `apply_cached_ratings` still writes base data only.
    let persist_task = persist_library(state);
    let download_tasks = download_missing(state);
    let decode_task = images::decode_visible(state);
    let critic_tasks = fetch_critics(state);
    let rating_tasks = fetch_ratings(state);
    Task::batch([
        persist_task,
        download_tasks,
        decode_task,
        critic_tasks,
        rating_tasks,
    ])
}

/// Show the on-disk library instantly at startup. The network fetch still
/// runs in the background and replaces this via `handle_loaded` when it
/// completes.
pub fn load_cached(state: &mut State) -> Task<Message> {
    let path = crate::library_cache::cache_path();
    let Some((mut items, purchase_dates)) = crate::library_cache::load(&path) else {
        return Task::none();
    };
    for item in &mut items {
        item.search_key = search::build_search_key(&item.title);
    }
    state.catalog_items = Some(items);
    state.inflight_fetches = 0;
    state.purchase_dates = purchase_dates;
    state.details_error = false;

    apply_cached_ratings(state);
    search::rebuild_order(state);

    let download_tasks = download_missing(state);
    let decode_task = images::decode_visible(state);
    let critic_tasks = fetch_critics(state);
    let rating_tasks = fetch_ratings(state);
    Task::batch([download_tasks, decode_task, critic_tasks, rating_tasks])
}

// Compare the fresh library against the disk cache off the UI thread and
// rewrite the file when anything changed. Fire-and-forget: completion maps
// to `Ignored`.
fn persist_library(state: &State) -> Task<Message> {
    let Some(items) = state.catalog_items.clone() else {
        return Task::none();
    };
    let purchase_dates = state.purchase_dates.clone();
    Task::future(async move {
        let path = crate::library_cache::cache_path();
        crate::library_cache::save_if_changed(&items, &purchase_dates, &path);
        Message::Ignored
    })
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

// Copy cached critic/user ratings onto freshly loaded items. Uncached
// games (new arrivals, never-rated) keep `None` and are fetched below.
fn apply_cached_ratings(state: &mut State) {
    let Some(items) = &mut state.catalog_items else {
        return;
    };
    for item in items.iter_mut() {
        state.ratings_cache.apply(item);
    }
}

// Queue a background persist of the ratings cache. At most one save runs
// at a time; arrivals during a save only flag the cache, and the save
// completion re-saves once if anything changed meanwhile.
fn persist_ratings(state: &mut State) -> Task<Message> {
    let Some(snapshot) = state.ratings_cache.snapshot() else {
        return Task::none();
    };
    let path = crate::ratings_cache::cache_path();
    Task::future(async move {
        if let Err(e) = crate::ratings_cache::save_snapshot(&snapshot, &path) {
            log::warn!("Failed to save ratings cache: {e:#}");
        }
        Message::RatingsCacheSaved
    })
}

pub fn handle_ratings_cache_saved(state: &mut State) -> Task<Message> {
    if let Some(snapshot) = state.ratings_cache.save_finished() {
        let path = crate::ratings_cache::cache_path();
        Task::future(async move {
            if let Err(e) = crate::ratings_cache::save_snapshot(&snapshot, &path) {
                log::warn!("Failed to save ratings cache: {e:#}");
            }
            Message::RatingsCacheSaved
        })
    } else {
        Task::none()
    }
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

    // Opening a different game starts the detail page at the top;
    // reopening the same one keeps the detail page's last position.
    match state.page {
        Page::GameDetail { game_id } => {
            if game_id != index {
                state.detail_scroll_offset = 0.0;
            }
        }
        _ => {}
    };

    state.details_error = false;
    state.page = Page::GameDetail { game_id: index };

    // The detail scrollable is only created when the page is shown and
    // always starts at the top, so snap it back to the saved offset the
    // same way the library grid restores its position.
    let restore = virtual_grid::scroll_detail_to_offset(state.detail_scroll_offset);

    if state.game_details.contains_key(&namespace) {
        return restore;
    }
    let Some(token) = state
        .auth_data
        .as_ref()
        .map(|auth| auth.access_token.clone())
    else {
        return Task::none();
    };
    let client = state.http_client.clone();

    let fetch = Task::future(async move {
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
    });
    Task::batch([fetch, restore])
}

/// Detail page's own scroll state: position saved separately from the
/// library grid, plus the viewport size the detail layout responds to.
pub fn handle_detail_scrolled(
    state: &mut State,
    offset_y: f32,
    width: f32,
    height: f32,
) -> Task<Message> {
    state.detail_scroll_offset = offset_y;
    state.viewport_width = width;
    state.viewport_height = height;
    Task::none()
}

pub fn handle_details_loaded(
    state: &mut State,
    namespace: String,
    details: Box<epic::GameFullDetails>,
) -> Task<Message> {
    let bodies = crate::ui::game_detail::parse_description(details.description());
    let downloads = fetch_description_images(state, &bodies);
    state.detail_bodies.insert(namespace.clone(), bodies);
    state.game_details.insert(namespace, *details);
    downloads
}

/// Fetch the images embedded in a game's description so the detail view
/// renders them instead of their alt text. Bytes stay in
/// `State::detail_images` (`Handle::from_bytes`) — no disk cache, no
/// thumbnails: the built-in `image` widget displays them at original size.
fn fetch_description_images(
    state: &State,
    bodies: &[iced::widget::markdown::Item],
) -> Task<Message> {
    let client = state.http_client.clone();
    Task::batch(
        crate::ui::game_detail::description_image_urls(bodies)
            .into_iter()
            .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
            .map(|url| {
                let client = client.clone();
                Task::future(async move {
                    let bytes = match client.get_async(&url).await {
                        Ok(mut res) => res.bytes().await.map(|b| b.to_vec()).unwrap_or_default(),
                        Err(e) => {
                            log::error!("Failed to fetch description image {url}: {e}");
                            Vec::new()
                        }
                    };
                    Message::DescriptionImageDownloaded { url, bytes }
                })
            })
            .collect::<Vec<_>>(),
    )
}

pub fn handle_description_image_loaded(
    state: &mut State,
    url: String,
    bytes: Vec<u8>,
) -> Task<Message> {
    if bytes.is_empty() {
        // Failed fetch: keep what was rendered so far (the alt text).
        return Task::none();
    }
    state
        .detail_images
        .insert(url, iced::widget::image::Handle::from_bytes(bytes));
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
// few; completions pull the rest off the queue one by one. Games already
// in a fresh disk cache are skipped (their scores were applied at load),
// but a stale cache refetches everything in the background.
fn fetch_critics(state: &mut State) -> Task<Message> {
    let Some(items) = &state.catalog_items else {
        return Task::none();
    };
    let stale = state.ratings_cache.is_stale();
    state.critic_queue = items
        .iter()
        // DLC never has its own OpenCritic score; skipping it here keeps
        // the queue short instead of burning searches on it.
        .filter(|item| !item.is_dlc())
        .filter(|item| stale || !state.ratings_cache.has_critic(&item.id))
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
    // Cache fresh scores for instant startup (unrated games stay out, so a
    // rating earned later is picked up). Persistence runs coalesced in the
    // background; a synchronous rewrite per arrival froze first loads.
    let persist = if state
        .ratings_cache
        .update_critic(id.clone(), score.as_ref())
    {
        persist_ratings(state)
    } else {
        Task::none()
    };
    if let Some(items) = &mut state.catalog_items
        && let Some(item) = items.iter_mut().find(|item| item.id == id)
    {
        item.critic = score;
    }
    // Scores stream in after the library loads; keep the display order
    // current so a critic sort settles as results arrive.
    search::rebuild_order(state);
    Task::batch([persist, pop_critic_task(state)])
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

// One user-rating fetch: Epic store GraphQL `getProductStars` by sandbox
// id (the catalog `namespace`). `attempts` counts tries so far; failed
// fetches requeue so throttling doesn't leave games unrated.
pub struct RatingJob {
    pub sandbox_id: String,
    pub id: String,
    pub attempts: u32,
}

// Same shape as the critic queue: a few store GraphQL fetches in flight
// at once, each completion pulling the next job off the queue.
const RATING_PARALLELISM: usize = 64;
// Total tries per game (initial attempt + requeues) before giving up.
const RATING_MAX_ATTEMPTS: u32 = 3;

// Queue every library item with a namespace, then start the first few;
// completions pull the rest off the queue one by one. Needs the store
// auth token, so no ratings are fetched when logged out.
fn fetch_ratings(state: &mut State) -> Task<Message> {
    let Some(items) = &state.catalog_items else {
        return Task::none();
    };
    let Some(token) = state
        .auth_data
        .as_ref()
        .map(|auth| auth.access_token.clone())
    else {
        return Task::none();
    };
    let stale = state.ratings_cache.is_stale();
    state.rating_queue = items
        .iter()
        .filter(|item| !item.namespace.is_empty())
        // Same skip rule as the critic queue: fresh cache hits were
        // applied at load, so only new/unrated games need fetching.
        .filter(|item| stale || !state.ratings_cache.has_user_rating(&item.id))
        .map(|item| RatingJob {
            sandbox_id: item.namespace.clone(),
            id: item.id.clone(),
            attempts: 0,
        })
        .collect();
    let take = RATING_PARALLELISM.min(state.rating_queue.len());
    let initial: Vec<RatingJob> = state.rating_queue.drain(..take).collect();
    Task::batch(
        initial
            .into_iter()
            .map(|job| spawn_rating_task(&state.http_client, &token, job)),
    )
}

fn spawn_rating_task(client: &isahc::HttpClient, token: &str, job: RatingJob) -> Task<Message> {
    let client = client.clone();
    let token = token.to_string();
    let RatingJob {
        sandbox_id,
        id,
        attempts,
    } = job;
    Task::future(async move {
        match epic::get_product_stars(&client, &token, &sandbox_id).await {
            Ok(rating) => Message::UserRatingLoaded { id, rating },
            Err(e) if attempts + 1 < RATING_MAX_ATTEMPTS => {
                log::debug!(
                    "user rating fetch throttled for {sandbox_id} (attempt {}), requeueing: {e:#}",
                    attempts + 1
                );
                Message::UserRatingRetry {
                    sandbox_id,
                    id,
                    attempts: attempts + 1,
                }
            }
            Err(e) => {
                log::debug!("user rating fetch failed for {sandbox_id}: {e:#}");
                Message::UserRatingLoaded { id, rating: None }
            }
        }
    })
}

/// Pull the next queued rating job now that a slot freed up;
/// `Task::none()` once the queue is drained.
fn pop_rating_task(state: &mut State) -> Task<Message> {
    let (client, token) = (
        state.http_client.clone(),
        state.auth_data.as_ref().map(|a| a.access_token.clone()),
    );
    let Some(token) = token else {
        return Task::none();
    };
    match state.rating_queue.pop_front() {
        Some(job) => spawn_rating_task(&client, &token, job),
        None => Task::none(),
    }
}

pub fn handle_rating_loaded(state: &mut State, id: String, rating: Option<f32>) -> Task<Message> {
    let persist = if state.ratings_cache.update_user_rating(id.clone(), rating) {
        persist_ratings(state)
    } else {
        Task::none()
    };
    if let Some(items) = &mut state.catalog_items
        && let Some(item) = items.iter_mut().find(|item| item.id == id)
    {
        item.user_rating = rating;
    }
    // Ratings stream in after the library loads; keep the display order
    // current so a user-rating sort settles as results arrive.
    search::rebuild_order(state);
    Task::batch([persist, pop_rating_task(state)])
}

// A throttled fetch gets another chance: push it to the back so other
// games go first (natural spacing without needing a timer), then fill
// the freed slot with the next job.
pub fn handle_rating_retry(
    state: &mut State,
    sandbox_id: String,
    id: String,
    attempts: u32,
) -> Task<Message> {
    state.rating_queue.push_back(RatingJob {
        sandbox_id,
        id,
        attempts,
    });
    pop_rating_task(state)
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

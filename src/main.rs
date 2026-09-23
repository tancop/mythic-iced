#![windows_subsystem = "windows"]

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use iced::futures::executor::block_on;
use iced::{Element, Font, Task, Theme};
use isahc::AsyncReadResponseExt;
use serde::{Deserialize, Serialize};
use smart_default::SmartDefault;

mod decode;
mod epic;
mod images;
mod ui;

const FONT_FILE: &[u8] = include_bytes!("../assets/Inter.ttf");

pub const UI_FONT: Font = {
    let mut font = Font::with_name("Inter");
    font.weight = iced::font::Weight::Medium;
    font
};

pub const BOLD_FONT: Font = {
    let mut font = Font::with_name("Inter");
    font.weight = iced::font::Weight::Bold;
    font
};

fn main() {
    env_logger::init();

    iced::application(boot, update, view)
        .title("Mythic")
        .theme(Theme::Custom(ui::get_theme().into()))
        .font(FONT_FILE)
        .default_font(UI_FONT)
        .run()
        .unwrap();
}

#[derive(SmartDefault)]
pub struct State {
    #[default(isahc::HttpClient::new().unwrap())]
    pub http_client: isahc::HttpClient,
    pub auth_data: Option<epic::AuthData>,
    #[default(Page::Login)]
    pub page: Page,
    pub exchange_code: String,
    pub library_items: Option<Vec<epic::LibraryItem>>,
    #[default(HashMap::new())]
    pub catalog_items: HashMap<String, epic::CatalogItem>,
    #[default(images::ImageLibrary::empty())]
    pub image_library: images::ImageLibrary,
    #[default(VecDeque::new())]
    pub pending_items: VecDeque<epic::LibraryItem>,
    #[default(0.0)]
    pub scroll_offset: f32,
    #[default(1024.0)]
    pub viewport_width: f32,
    #[default(768.0)]
    pub viewport_height: f32,
    #[default(HashMap::new())]
    pub decoded_images: HashMap<String, (u32, u32, Arc<Vec<u8>>)>,
    #[default(HashSet::new())]
    pub inflight_decodes: HashSet<String>,
}

const CONCURRENT_LIMIT: usize = 20;

pub enum Page {
    Library,
    Login,
    PasteToken,
}

#[derive(Clone, Debug)]
pub struct DecodedCard {
    pub id: String,
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<Vec<u8>>,
}

#[derive(Clone, Debug)]
enum Message {
    Ignored,
    StartLogin,
    SubmitToken(String),
    LibraryLoaded(Vec<epic::LibraryItem>),
    GameInfoLoaded(epic::CatalogItem),
    // A catalog fetch failed: the item is skipped, but the queue must still
    // advance or every item behind it would never load.
    GameInfoFailed,
    ImageDownloaded(String, Arc<Vec<u8>>),
    Scrolled {
        offset_y: f32,
        width: f32,
        height: f32,
    },
    ChunkDecoded {
        decoded: Vec<DecodedCard>,
        failed: Vec<String>,
    },
}

fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::Ignored => Task::none(),
        Message::ImageDownloaded(id, bytes) => {
            state.image_library.insert(id, (*bytes).clone());
            let _ = state.image_library.save(&image_library_path());
            decode_visible(state)
        }
        Message::Scrolled {
            offset_y,
            width,
            height,
        } => {
            state.scroll_offset = offset_y;
            state.viewport_width = width;
            state.viewport_height = height;
            evict_stale(state);
            decode_visible(state)
        }
        Message::ChunkDecoded { decoded, failed } => {
            for id in failed {
                state.inflight_decodes.remove(&id);
            }
            for card in decoded {
                state.inflight_decodes.remove(&card.id);
                state
                    .decoded_images
                    .insert(card.id, (card.width, card.height, card.pixels));
            }
            Task::none()
        }
        Message::StartLogin => {
            open::that(epic::get_auth_url()).unwrap();
            state.page = Page::PasteToken;
            Task::none()
        }
        Message::SubmitToken(token) => {
            let auth_data = block_on(epic::authenticate(&state.http_client, &token));
            match auth_data {
                Ok(auth_data) => {
                    let task = load_library(&state.http_client, &auth_data);

                    save_refresh_token(&auth_data.refresh_token);
                    state.auth_data = Some(auth_data);
                    state.page = Page::Library;

                    task
                }
                Err(e) => {
                    log::error!("Failed to authenticate: {}", e);
                    state.page = Page::Login;
                    Task::none()
                }
            }
        }
        Message::LibraryLoaded(items) => {
            state.library_items = Some(items.clone());

            let mut queue: VecDeque<_> = items.into_iter().collect();
            let initial: Vec<_> = queue.drain(..CONCURRENT_LIMIT.min(queue.len())).collect();

            let fetch_task = Task::batch(initial.iter().map(|item| fetch_game_info(state, item)));
            state.pending_items = queue;
            let decode_task = decode_visible(state);
            Task::batch([fetch_task, decode_task])
        }
        Message::GameInfoFailed => {
            if let Some(next) = state.pending_items.pop_front() {
                fetch_game_info(state, &next)
            } else {
                Task::none()
            }
        }
        Message::GameInfoLoaded(info) => {
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
                    if let Some(next) = state.pending_items.pop_front() {
                        fetch_game_info(state, &next)
                    } else {
                        Task::none()
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
                                match resize_image(&bytes) {
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
    }
}

fn view(state: &State) -> Element<'_, Message> {
    match state.page {
        Page::Library => ui::library::view(state),
        Page::Login => ui::login::view(state),
        Page::PasteToken => ui::login::view_paste_token(state),
    }
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

fn visible_range(state: &State) -> (usize, usize) {
    let Some(items) = &state.library_items else {
        return (0, 0);
    };
    let pitch = ui::library::row_pitch(state.viewport_width);
    let cols = ui::library::cols_for_width(state.viewport_width);
    let total_rows = items.len() / cols + (items.len() % cols != 0) as usize;
    let first_visible_row = (state.scroll_offset / pitch) as usize;
    let visible_rows = (state.viewport_height / pitch) as usize + 1;
    let lo = first_visible_row.saturating_sub(ui::library::BUFFER_ROWS);
    let hi = (first_visible_row + visible_rows + ui::library::BUFFER_ROWS).min(total_rows);
    (lo, hi)
}

fn decode_visible(state: &mut State) -> Task<Message> {
    let Some(items) = &state.library_items else {
        return Task::none();
    };

    let cols = ui::library::cols_for_width(state.viewport_width);
    let (lo, hi) = visible_range(state);

    // One task per grid chunk so a whole row swaps in atomically instead of
    // cards popping in one by one (each completion re-renders the grid).
    let mut chunks: Vec<Vec<(String, Vec<u8>)>> = Vec::new();

    for chunk in items.chunks(cols).skip(lo).take(hi.saturating_sub(lo)) {
        let mut pending: Vec<(String, Vec<u8>)> = Vec::new();
        for item in chunk {
            let Some(catalog) = state.catalog_items.get(item.catalog_item_id.as_ref()) else {
                continue;
            };
            if state.decoded_images.contains_key(&catalog.id)
                || !state.inflight_decodes.insert(catalog.id.clone())
            {
                continue;
            }
            match state.image_library.get(&catalog.id) {
                Some(bytes) if !bytes.is_empty() => {
                    pending.push((catalog.id.clone(), bytes.to_vec()));
                }
                _ => {
                    // Not cached yet (or cached empty): don't wedge, retry on
                    // the next scroll event.
                    state.inflight_decodes.remove(&catalog.id);
                }
            }
        }
        if !pending.is_empty() {
            chunks.push(pending);
        }
    }

    Task::batch(
        chunks
            .into_iter()
            .map(|chunk| Task::future(async move { decode_chunk(chunk).await })),
    )
}

fn evict_stale(state: &mut State) {
    let Some(items) = &state.library_items else {
        return;
    };

    let (lo, hi) = visible_range(state);

    let mut visible_ids = HashSet::new();
    for chunk in items
        .chunks(ui::library::cols_for_width(state.viewport_width))
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

async fn decode_chunk(chunk: Vec<(String, Vec<u8>)>) -> Message {
    let mut decoded = Vec::with_capacity(chunk.len());
    let mut failed = Vec::new();
    for (id, bytes) in chunk {
        match image::load_from_memory(&bytes) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let (width, height) = rgba.dimensions();
                decoded.push(DecodedCard {
                    id,
                    width,
                    height,
                    pixels: Arc::new(rgba.into_raw()),
                });
            }
            Err(e) => {
                log::error!("Failed to decode image {}: {}", id, e);
                failed.push(id);
            }
        }
    }
    Message::ChunkDecoded { decoded, failed }
}

fn load_library(http_client: &isahc::HttpClient, auth_data: &epic::AuthData) -> Task<Message> {
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

fn boot() -> (State, Task<Message>) {
    let mut state = State::default();

    let lib_path = image_library_path();
    state.image_library = images::ImageLibrary::load(&lib_path);

    if let Some(token) = load_refresh_token()
        && let Ok(auth_data) = block_on(epic::refresh_token(&state.http_client, &token))
    {
        let task = load_library(&state.http_client, &auth_data);

        save_refresh_token(&auth_data.refresh_token);
        state.auth_data = Some(auth_data);
        state.page = Page::Library;

        (state, task)
    } else {
        (state, Task::none())
    }
}

#[derive(Serialize, Deserialize)]
struct TokenCache {
    refresh_token: String,
}

fn image_library_path() -> PathBuf {
    dirs::cache_dir().unwrap().join("mythic").join("images.db")
}

const THUMB_WIDTH: u32 = 255;
const THUMB_HEIGHT: u32 = 340;

fn resize_image(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let img = image::load_from_memory(bytes)?;
    let resized = img.resize(
        THUMB_WIDTH,
        THUMB_HEIGHT,
        image::imageops::FilterType::Lanczos3,
    );
    let mut buf = std::io::Cursor::new(Vec::new());
    resized.write_to(&mut buf, image::ImageFormat::Jpeg)?;
    Ok(buf.into_inner())
}

fn load_refresh_token() -> Option<String> {
    let cache_file = dirs::cache_dir()
        .unwrap()
        .join("mythic")
        .join("tokens.toml");

    if let Ok(contents) = std::fs::read_to_string(&cache_file) {
        if let Ok(cache) = toml::from_str::<TokenCache>(&contents) {
            return Some(cache.refresh_token);
        }
    }
    None
}

fn save_refresh_token(token: &str) {
    let cache_file = dirs::cache_dir()
        .unwrap()
        .join("mythic")
        .join("tokens.toml");

    let cache = TokenCache {
        refresh_token: token.to_string(),
    };
    if let Ok(contents) = toml::to_string(&cache) {
        std::fs::write(&cache_file, contents).ok();
    }
}

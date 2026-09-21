use std::collections::{HashMap, VecDeque};
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

const UI_FONT: &[u8] = include_bytes!("../assets/Inter.ttf");

fn main() {
    env_logger::init();

    let mut font = Font::with_name("Inter");
    font.weight = iced::font::Weight::Medium;

    iced::application(boot, update, view)
        .title("Mythic")
        .theme(Theme::Custom(ui::get_theme().into()))
        .font(UI_FONT)
        .default_font(font)
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
}

const CONCURRENT_LIMIT: usize = 20;

pub enum Page {
    Library,
    Login,
    PasteToken,
}

#[derive(Clone, Debug)]
enum Message {
    Ignored,
    StartLogin,
    SubmitToken(String),
    LibraryLoaded(Vec<epic::LibraryItem>),
    GameInfoLoaded(epic::CatalogItem),
    ImageDownloaded(String, Arc<Vec<u8>>),
}

fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::Ignored => Task::none(),
        Message::ImageDownloaded(id, bytes) => {
            state.image_library.insert(id, (*bytes).clone());
            let _ = state.image_library.save(&image_library_path());
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

            let task = Task::batch(initial.iter().map(|item| fetch_game_info(state, item)));
            state.pending_items = queue;
            task
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
                Message::Ignored
            }
        }
    })
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

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use iced::futures::executor::block_on;
use iced::{Element, Font, Task, Theme};
use isahc::AsyncReadResponseExt;
use serde::{Deserialize, Serialize};
use smart_default::SmartDefault;

mod decode;
mod epic;
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
    #[default(image_dir())]
    pub image_dir: PathBuf,
}

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
    ImageDownloaded(String),
}

fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::Ignored => Task::none(),
        Message::ImageDownloaded(_) => Task::none(),
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
            let access_token = Arc::new(state.auth_data.as_ref().unwrap().access_token.clone());
            let task = Task::batch(items.iter().map(|item| {
                let client = state.http_client.clone();
                let access_token = access_token.clone();
                let namespace = item.namespace.clone();
                let catalog_id = item.catalog_item_id.clone();

                Task::future(async move {
                    match epic::get_game_info(&client, &access_token, &namespace, &catalog_id).await
                    {
                        Ok(info) => Message::GameInfoLoaded(info),
                        Err(e) => {
                            log::error!("Failed to get game info: {}", e);
                            Message::Ignored
                        }
                    }
                })
            }));
            state.library_items = Some(items);
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

            state.catalog_items.insert(id.clone(), info.clone());

            if let Some(url) = image_url {
                let ext = url
                    .rsplit('.')
                    .next()
                    .unwrap_or("png")
                    .split('?')
                    .next()
                    .unwrap_or("png");
                let image_path = state.image_dir.join(format!("{}.{}", info.id, ext));

                if image_path.exists() {
                    log::debug!("Image already cached: {}", image_path.display());
                    return Task::none();
                }
                Task::future(async move {
                    let mut res = client.get_async(&url).await.ok();
                    if let Some(ref mut res) = res {
                        if let Ok(bytes) = res.bytes().await {
                            let _ = std::fs::write(&image_path, &bytes);
                            log::debug!("Saved image: {}", image_path.display());
                        }
                    }
                    Message::ImageDownloaded(id)
                })
            } else {
                log::warn!("No images found for {}", &info.title);
                Task::none()
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

fn image_dir() -> PathBuf {
    let dir = dirs::cache_dir().unwrap().join("mythic").join("images");
    let _ = std::fs::create_dir_all(&dir);
    dir
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

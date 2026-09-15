use iced::Element;
use iced::futures::executor::block_on;
use serde::{Deserialize, Serialize};

mod decode;
mod epic;
mod ui;

fn main() {
    env_logger::init();

    iced::application(boot, update, view).run().unwrap();
}

pub struct State {
    pub http_client: isahc::HttpClient,
    pub auth_data: Option<epic::AuthData>,
    pub page: Page,
}

pub enum Page {
    Library,
    Login,
}

enum Message {}

fn update(_: &mut State, _: Message) {}

fn view(state: &State) -> Element<'_, Message> {
    match state.page {
        Page::Library => ui::library::view(state),
        Page::Login => ui::login::view(state),
    }
}

fn boot() -> State {
    let client = isahc::HttpClient::new().unwrap();
    if let Some(token) = load_refresh_token()
        && let Ok(auth_data) = block_on(epic::refresh_token(&client, &token))
    {
        save_refresh_token(&auth_data.refresh_token);
        State {
            http_client: client,
            auth_data: Some(auth_data),
            page: Page::Library,
        }
    } else {
        State {
            http_client: client,
            auth_data: None,
            page: Page::Login,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct TokenCache {
    refresh_token: String,
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

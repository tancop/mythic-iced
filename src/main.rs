use iced::futures::executor::block_on;
use iced::{Element, Font, Theme};
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
}

fn update(state: &mut State, message: Message) {
    match message {
        Message::Ignored => {}
        Message::StartLogin => {
            open::that(epic::get_auth_url()).unwrap();
            state.page = Page::PasteToken;
        }
        Message::SubmitToken(token) => {
            let auth_data = block_on(epic::authenticate(&state.http_client, &token));
            match auth_data {
                Ok(auth_data) => {
                    save_refresh_token(&auth_data.refresh_token);
                    state.auth_data = Some(auth_data);
                    state.page = Page::Library;
                }
                Err(e) => {
                    log::error!("Failed to authenticate: {}", e);
                    state.page = Page::Login;
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

fn boot() -> State {
    let mut state = State::default();
    if let Some(token) = load_refresh_token()
        && let Ok(auth_data) = block_on(epic::refresh_token(&state.http_client, &token))
    {
        save_refresh_token(&auth_data.refresh_token);
        state.auth_data = Some(auth_data);
        state.page = Page::Library;
    }
    state
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

use iced::Task;
use iced::futures::executor::block_on;
use serde::{Deserialize, Serialize};

use crate::{Message, State, epic, library};

pub fn handle_start(state: &mut State) -> Task<Message> {
    open::that(epic::get_auth_url()).unwrap();
    state.page = crate::Page::PasteToken;
    Task::none()
}

pub fn handle_submit(state: &mut State, token: String) -> Task<Message> {
    let auth_data = block_on(epic::authenticate(&state.http_client, &token));
    match auth_data {
        Ok(auth_data) => {
            let task = library::load_library(&state.http_client, &auth_data);

            save_refresh_token(&auth_data.refresh_token);
            state.auth_data = Some(auth_data);
            state.page = crate::Page::Library;

            task
        }
        Err(e) => {
            log::error!("Failed to authenticate: {}", e);
            state.page = crate::Page::Login;
            Task::none()
        }
    }
}

pub fn refresh_blocking(
    http_client: &isahc::HttpClient,
    token: &str,
) -> anyhow::Result<epic::AuthData> {
    block_on(epic::refresh_token(http_client, token))
}

#[derive(Serialize, Deserialize)]
struct TokenCache {
    refresh_token: String,
}

fn token_cache_path() -> std::path::PathBuf {
    dirs::cache_dir()
        .unwrap()
        .join("mythic")
        .join("tokens.toml")
}

pub fn load_refresh_token() -> Option<String> {
    let cache_file = token_cache_path();

    if let Ok(contents) = std::fs::read_to_string(&cache_file) {
        if let Ok(cache) = toml::from_str::<TokenCache>(&contents) {
            return Some(cache.refresh_token);
        }
    }
    None
}

pub fn save_refresh_token(token: &str) {
    let cache_file = token_cache_path();

    let cache = TokenCache {
        refresh_token: token.to_string(),
    };
    if let Ok(contents) = toml::to_string(&cache) {
        std::fs::write(&cache_file, contents).ok();
    }
}

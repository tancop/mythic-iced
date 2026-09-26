#![windows_subsystem = "windows"]

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use iced::{Element, Font, Task, Theme};
use smart_default::SmartDefault;

mod decode;
mod epic;
mod images;
mod library;
mod login;
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
        decoded: Vec<images::DecodedCard>,
        failed: Vec<String>,
    },
}

fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::Ignored => Task::none(),
        Message::StartLogin => login::handle_start(state),
        Message::SubmitToken(token) => login::handle_submit(state, token),
        Message::LibraryLoaded(items) => library::handle_loaded(state, items),
        Message::GameInfoLoaded(info) => library::handle_game_info(state, info),
        Message::GameInfoFailed => library::handle_game_info_failed(state),
        Message::Scrolled {
            offset_y,
            width,
            height,
        } => library::handle_scrolled(state, offset_y, width, height),
        Message::ImageDownloaded(id, bytes) => images::handle_downloaded(state, id, bytes),
        Message::ChunkDecoded { decoded, failed } => {
            images::handle_chunk_decoded(state, decoded, failed)
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

fn boot() -> (State, Task<Message>) {
    let mut state = State::default();

    state.image_library = images::ImageLibrary::load(&images::library_path());

    if let Some(token) = login::load_refresh_token()
        && let Ok(auth_data) = login::refresh_blocking(&state.http_client, &token)
    {
        let task = library::load_library(&state.http_client, &auth_data);

        login::save_refresh_token(&auth_data.refresh_token);
        state.auth_data = Some(auth_data);
        state.page = Page::Library;

        (state, task)
    } else {
        (state, Task::none())
    }
}

#![windows_subsystem = "windows"]

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use iced::{Element, Font, Settings, Task, Theme};
use smart_default::SmartDefault;

use crate::images::PixelData;

use crate::search::{FilterRule, SortKey};

mod decode;
mod epic;
mod images;
mod library;
mod logging;
mod login;
mod search;
mod ui;

const INTER_FONT_FILE: &[u8] = include_bytes!("../assets/Inter.ttf");
const FA_SOLID_FILE: &[u8] = include_bytes!("../assets/Font Awesome 7 Solid.otf");

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

pub const FA_SOLID: Font = Font::with_name("Font Awesome 7 Solid");

pub const DEFAULT_TEXT_SIZE: u32 = 16;
pub const LIBRARY_TITLE_TEXT_SIZE: u32 = 14;
pub const HEADING_TEXT_SIZE: u32 = 18;

fn main() {
    crate::logging::init();

    log::info!("Starting Mythic Launcher...");

    iced::application(boot, update, view)
        .title("Mythic")
        .theme(Theme::Custom(ui::get_theme().into()))
        .settings(Settings {
            default_text_size: DEFAULT_TEXT_SIZE.into(),
            default_font: UI_FONT,
            ..Default::default()
        })
        .font(INTER_FONT_FILE)
        .font(FA_SOLID_FILE)
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
    pub catalog_items: Option<Vec<epic::CatalogItem>>,
    #[default(images::ImageLibrary::empty())]
    pub image_library: images::ImageLibrary,
    // Set while the GraphQL library fetch is in flight; the grid shows
    // `Loading...` until it completes.
    pub inflight_fetches: usize,
    #[default(0.0)]
    pub scroll_offset: f32,
    #[default(1024.0)]
    pub viewport_width: f32,
    #[default(768.0)]
    pub viewport_height: f32,
    #[default(HashMap::new())]
    pub decoded_images: HashMap<String, PixelData>,
    #[default(HashSet::new())]
    pub inflight_decodes: HashSet<String>,

    // Filter out DLC items from the library view
    #[default(FilterRule::Block)]
    pub filter_dlc: FilterRule,
    pub search_query: String,
    pub sort_key: SortKey,
    pub sort_reverse: bool,
    // Indexes into catalog_items in display order (filtered + sorted)
    pub order: Vec<usize>,
    // Acquisition dates by catalog id, for purchase-date sorting
    #[default(HashMap::new())]
    pub purchase_dates: HashMap<String, epic::UtcDateTime>,

    pub focused_game_idx: Option<usize>,
    // Full store info by namespace (sandbox id), fetched on demand.
    #[default(HashMap::new())]
    pub game_details: HashMap<String, epic::GameFullDetails>,
    // Parsed markdown bodies for the detail view, keyed by namespace.
    #[default(HashMap::new())]
    pub detail_bodies: HashMap<String, Vec<iced::widget::markdown::Item>>,
    pub details_error: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Library,
    Login,
    PasteToken,
    GameDetail,
}

#[derive(Clone, Debug)]
pub enum Message {
    Ignored,
    StartLogin,
    SubmitToken(String),
    LibraryLoaded(Vec<epic::CatalogItem>, HashMap<String, epic::UtcDateTime>),
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
    SortKeySelected(SortKey),
    SortReverseToggled(bool),
    DlcFilterSelected(FilterRule),
    SearchQueryChanged(String),
    GameSelected(usize),
    GameDetailsLoaded {
        namespace: String,
        details: Box<epic::GameFullDetails>,
    },
    GameDetailsFailed,
    CriticLoaded {
        id: String,
        score: Option<epic::CriticScore>,
    },
    Navigate(Page),
}

fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::Ignored => Task::none(),
        Message::StartLogin => login::handle_start(state),
        Message::SubmitToken(token) => login::handle_submit(state, token),
        Message::LibraryLoaded(items, purchase_dates) => {
            library::handle_loaded(state, items, purchase_dates)
        }
        Message::Scrolled {
            offset_y,
            width,
            height,
        } => library::handle_scrolled(state, offset_y, width, height),
        Message::ImageDownloaded(id, bytes) => images::handle_downloaded(state, id, bytes),
        Message::ChunkDecoded { decoded, failed } => {
            images::handle_chunk_decoded(state, decoded, failed)
        }
        Message::SortKeySelected(key) => search::set_sort_key(state, key),
        Message::SortReverseToggled(reverse) => search::set_sort_reverse(state, reverse),
        Message::DlcFilterSelected(rule) => search::set_dlc_filter(state, rule),
        Message::SearchQueryChanged(query) => search::set_search_query(state, query),
        Message::GameSelected(index) => library::handle_game_selected(state, index),
        Message::GameDetailsLoaded { namespace, details } => {
            library::handle_details_loaded(state, namespace, details)
        }
        Message::GameDetailsFailed => library::handle_details_failed(state),
        Message::CriticLoaded { id, score } => library::handle_critic_loaded(state, id, score),
        Message::Navigate(page) => {
            state.page = page;
            Task::none()
        }
    }
}

fn view(state: &State) -> Element<'_, Message> {
    match state.page {
        Page::Library => {
            iced::widget::column![ui::navbar::view(state), ui::library::view(state),].into()
        }
        Page::Login => ui::login::view(state),
        Page::PasteToken => ui::login::view_paste_token(state),
        Page::GameDetail => {
            iced::widget::column![ui::navbar::view(state), ui::game_detail::view(state)].into()
        }
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

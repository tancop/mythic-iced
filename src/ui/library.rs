use iced::{
    Alignment, Color, Element, Length, Theme,
    widget::{column, container, image, mouse_area, pick_list, row, text, text_input},
};

use iced::widget::container::Style;

use crate::{
    LIBRARY_TITLE_TEXT_SIZE, Message, State,
    images::{self, PixelData},
    search::{DLC_FILTERS, SortOption, effective_sort_key, sort_options},
    ui::{
        TextWidgetExt,
        icons::{UP_DOWN_ARROW, icon_button},
        theme::BRAND_COLOR,
        virtual_grid::{self, GridConfig, GridViewport},
    },
};

pub const GRID_CONFIG: GridConfig = GridConfig {
    target_card_width: 200.0,
    spacing: 8.0,
    buffer_rows: 5,
    card_aspect: (images::THUMB_HEIGHT as f32 + LIBRARY_TITLE_TEXT_SIZE as f32)
        / images::THUMB_WIDTH as f32,
};

pub fn view(state: &State) -> Element<'_, Message> {
    let toolbar = row![
        text_input("Search...", &state.search_query)
            .on_input(Message::SearchQueryChanged)
            .width(Length::Fill),
        text!("Sort:"),
        pick_list(
            sort_options(state.sort_reverse),
            Some(SortOption {
                key: effective_sort_key(state),
                reversed: state.sort_reverse,
            }),
            |option| Message::SortKeySelected(option.key),
        ),
        icon_button(
            UP_DOWN_ARROW,
            if state.sort_reverse {
                BRAND_COLOR
            } else {
                Color::WHITE
            },
            Message::SortReverseToggled(!state.sort_reverse),
        ),
        text!("Show:"),
        pick_list(
            &DLC_FILTERS[..],
            Some(state.filter_dlc),
            Message::DlcFilterSelected
        ),
    ]
    .spacing(GRID_CONFIG.spacing)
    .align_y(Alignment::Center);

    let Some(items) = &state.catalog_items else {
        return container(text!("Loading...")).center(Length::Fill).into();
    };
    if state.inflight_fetches > 0 {
        return container(text!("Loading...")).center(Length::Fill).into();
    }

    // The grid is sized from the visible order, so the height always
    // matches what is actually shown.
    let grid = virtual_grid::virtual_grid(
        GRID_CONFIG,
        GridViewport {
            width: state.viewport_width,
            height: state.viewport_height,
            scroll_offset: state.scroll_offset,
        },
        state.order.len(),
        |position, card_w, card_h| {
            let index = state.order[position];
            let catalog = &items[index];
            let card: Element<'_, Message> = if let Some(PixelData {
                width,
                height,
                pixels,
            }) = state.decoded_images.get(&catalog.id)
            {
                let handle =
                    iced::widget::image::Handle::from_rgba(*width, *height, pixels.to_vec());
                let img = image(handle)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .content_fit(iced::ContentFit::Cover);
                container(column![
                    img,
                    text!("{}", catalog.title)
                        .bold()
                        .size(LIBRARY_TITLE_TEXT_SIZE)
                        .wrapping(text::Wrapping::None)
                ])
                .width(Length::Fixed(card_w))
                .height(Length::Fixed(card_h))
                .clip(true)
                .into()
            } else {
                placeholder_card(card_w, card_h)
            };
            mouse_area(card)
                .on_press(Message::GameSelected(index))
                .into()
        },
        |offset_y, width, height| Message::Scrolled {
            offset_y,
            width,
            height,
        },
    );

    container(column![toolbar, grid].spacing(GRID_CONFIG.spacing))
        .padding(16)
        .into()
}

fn placeholder_card(card_w: f32, card_h: f32) -> Element<'static, Message> {
    container("")
        .style(|theme: &Theme| Style::default().background(theme.palette().primary))
        .width(Length::Fixed(card_w))
        .height(Length::Fixed(card_h))
        .center_y(Length::Fixed(card_h))
        .clip(true)
        .into()
}

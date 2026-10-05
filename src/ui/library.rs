use iced::{
    Alignment, Color, Element, Length, Padding, Theme,
    widget::{
        column, container, image, mouse_area, pick_list, row, space, stack, text, text_input,
    },
};

use iced::widget::container::Style;

use crate::{
    LIBRARY_TITLE_TEXT_SIZE, Message, State,
    images::{self, PixelData},
    search::{DLC_FILTERS, SortOption, effective_sort_key, sort_options},
    ui::{
        TextWidgetExt,
        icons::{UP_DOWN_ARROW, X_MARK, icon_button},
        theme::MYTHIC_GOLD,
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
    // The field always renders as the same widget tree (a stack with the
    // input underneath) so the text input keeps focus while typing:
    // swapping the whole field on the first keystroke would despawn the
    // focused input. Only the overlay button itself is conditional. The
    // input keeps extra right padding so long queries don't slide
    // underneath the button.
    let mut overlay = row![space::horizontal()]
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(Alignment::Center)
        .padding(Padding {
            top: 0.0,
            right: 8.0,
            bottom: 0.0,
            left: 0.0,
        });
    if !state.search_query.is_empty() {
        overlay = overlay.push(icon_button(
            X_MARK,
            Color::WHITE,
            Message::SearchQueryChanged(String::new()),
        ));
    }

    let search_field: Element<'_, Message> = stack![
        text_input("Search...", &state.search_query)
            .on_input(Message::SearchQueryChanged)
            .padding(Padding {
                top: 5.0,
                right: 28.0,
                bottom: 5.0,
                left: 5.0,
            })
            .width(Length::Fill),
        overlay,
    ]
    .width(Length::Fill)
    .into();

    let mut toolbar = row![search_field]
        .spacing(GRID_CONFIG.spacing)
        .align_y(Alignment::Center);

    toolbar = toolbar
        .push(text!("Sort:"))
        .push(pick_list(
            sort_options(state.sort_reverse),
            Some(SortOption {
                key: effective_sort_key(state),
                reversed: state.sort_reverse,
            }),
            |option| Message::SortKeySelected(option.key),
        ))
        .push(icon_button(
            UP_DOWN_ARROW,
            if state.sort_reverse {
                MYTHIC_GOLD
            } else {
                Color::WHITE
            },
            Message::SortReverseToggled(!state.sort_reverse),
        ))
        .push(text!("Show:"))
        .push(pick_list(
            &DLC_FILTERS[..],
            Some(state.filter_dlc),
            Message::DlcFilterSelected,
        ));

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolbar_builds_with_and_without_query() {
        // The clear button only exists for a non-empty query; both
        // variants must build without panicking.
        let _ = view(&State::default());
        let state = State {
            search_query: "civ".to_string(),
            ..State::default()
        };
        let _ = view(&state);
    }
}

use iced::{
    Alignment, Border, Color, Element, Length, Padding, Theme, padding,
    widget::{column, container, image, mouse_area, pick_list, row, stack, text},
};

use iced::widget::container::Style;

use crate::{
    HEADING_TEXT_SIZE, LIBRARY_TITLE_TEXT_SIZE, Message, State,
    epic::{CatalogItem, CriticRating},
    images::{self, PixelData},
    search::{DLC_FILTERS, SortKey, SortOption, effective_sort_key, sort_options},
    ui::{
        TextWidgetExt,
        icons::{UP_DOWN_ARROW, icon_button},
        theme::{
            self, MYTHIC_GOLD, OPENCRITIC_FAIR, OPENCRITIC_MIGHTY, OPENCRITIC_STRONG,
            OPENCRITIC_WEAK,
        },
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
    // The search field lives in the nav bar (`ui::navbar`); this toolbar
    // only carries the sort and DLC controls.
    let toolbar = row![
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
                MYTHIC_GOLD
            } else {
                theme::HIGHLIGHT
            },
            Message::SortReverseToggled(!state.sort_reverse),
        ),
        text!("Show:"),
        pick_list(
            &DLC_FILTERS[..],
            Some(state.filter_dlc),
            Message::DlcFilterSelected,
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
            let base: Element<'_, Message> = if let Some(PixelData {
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
            let card: Element<'_, Message> = match score_badge(catalog, effective_sort_key(state)) {
                Some((label, background)) => stack![
                    base,
                    container(badge(label, background))
                        .align_right(Length::Fixed(card_w))
                        .align_top(Length::Fixed(card_h))
                        .padding(padding::right(5).top(5)),
                ]
                .into(),
                None => base,
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

fn opencritic_color(rating: CriticRating) -> Color {
    match rating {
        CriticRating::Weak => OPENCRITIC_WEAK,
        CriticRating::Fair => OPENCRITIC_FAIR,
        CriticRating::Strong => OPENCRITIC_STRONG,
        CriticRating::Mighty => OPENCRITIC_MIGHTY,
    }
}

/// Score text plus badge background for the active rating sort, if any.
/// `Some(color)` is the critic badge color; `None` means the theme primary
/// (user ratings). Returns `None` when the sort isn't a rating sort or the
/// game has no score for it, so no badge is shown.
fn score_badge(item: &CatalogItem, sort: SortKey) -> Option<(String, Option<Color>)> {
    match sort {
        SortKey::Critic => item.critic.as_ref().map(|critic| {
            (
                critic.average.to_string(),
                Some(opencritic_color(critic.rating)),
            )
        }),
        SortKey::UserRating => item
            .user_rating
            .map(|rating| (format!("{rating:.1}"), None)),
        _ => None,
    }
}

fn badge(label: String, background: Option<Color>) -> Element<'static, Message> {
    container(
        text!("{label}")
            .bold()
            .size(HEADING_TEXT_SIZE)
            .color(theme::HIGHLIGHT),
    )
    .padding(Padding {
        top: 2.0,
        right: 8.0,
        bottom: 2.0,
        left: 8.0,
    })
    .style(move |theme: &Theme| {
        Style::default()
            .background(background.unwrap_or(theme.palette().primary))
            .border(Border::default().rounded(6.0))
    })
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_builds_with_and_without_query() {
        // The clear button lives in the nav bar now, but an active query
        // still switches the effective sort key to `Search`, which changes
        // the badges on the cards; both variants must build.
        let _ = view(&State::default());
        let state = State {
            search_query: "civ".to_string(),
            ..State::default()
        };
        let _ = view(&state);
    }
}

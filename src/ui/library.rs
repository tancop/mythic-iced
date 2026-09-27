use iced::{
    Alignment, Element, Length, Theme,
    widget::{checkbox, column, container, image, pick_list, row, scrollable, text, text_input},
};

use iced::widget::container::Style;

use crate::{
    Message, State,
    images::PixelData,
    search::{DLC_FILTERS, FilterRule, SORT_KEYS, effective_sort_key},
    ui::TextWidgetExt,
};

pub const MIN_COLS: usize = 5;
pub const MAX_COLS: usize = 7;
pub const SPACING: f32 = 8.0;
pub const BUFFER_ROWS: usize = 5;

// Approximate card width the column count derives from: 5 columns at default
// window size, up to 7 when fullscreen.
const TARGET_CARD_WIDTH: f32 = 200.0;

pub fn cols_for_width(viewport_width: f32) -> usize {
    ((viewport_width / TARGET_CARD_WIDTH).round() as usize).clamp(MIN_COLS, MAX_COLS)
}

// Height / width of a card (matches the 255x340 thumbnails).
pub const CARD_ASPECT: f32 = 340.0 / 255.0;

pub fn card_width(viewport_width: f32) -> f32 {
    let cols = cols_for_width(viewport_width);
    ((viewport_width - (cols - 1) as f32 * SPACING) / cols as f32).max(1.0)
}

pub fn card_height(viewport_width: f32) -> f32 {
    card_width(viewport_width) * CARD_ASPECT
}

pub fn row_pitch(viewport_width: f32) -> f32 {
    card_height(viewport_width) + SPACING
}

fn total_rows(state: &State) -> usize {
    let cols = cols_for_width(state.viewport_width);
    // Pinned to the final library size while everything is shown, so the
    // scrollbar doesn't drift as rows stream in; sized from the visible
    // order as soon as a filter hides items.
    let count = if state.filter_dlc == FilterRule::Allow && state.search_query.is_empty() {
        state.total_items
    } else {
        state.order.len()
    };
    count / cols + (count % cols != 0) as usize
}

pub fn view(state: &State) -> Element<'_, Message> {
    let toolbar = row![
        text!("Library"),
        text_input("Search...", &state.search_query)
            .on_input(Message::SearchQueryChanged)
            .width(Length::Fill),
        text!("Sort:"),
        pick_list(
            &SORT_KEYS[..],
            Some(effective_sort_key(state)),
            Message::SortKeySelected
        ),
        checkbox(state.sort_reverse)
            .label("Reverse")
            .on_toggle(Message::SortReverseToggled),
        text!("Show:"),
        pick_list(
            &DLC_FILTERS[..],
            Some(state.filter_dlc),
            Message::DlcFilterSelected
        ),
    ]
    .spacing(SPACING)
    .align_y(Alignment::Center);

    let Some(items) = &state.catalog_items else {
        return container(text!("Loading...")).center(Length::Fill).into();
    };
    let order = &state.order;

    let cols = cols_for_width(state.viewport_width);
    // Sized from the final item count so the scrollbar is stable from the start
    let total_rows = total_rows(state);
    let shown_rows = order.len() / cols + (order.len() % cols != 0) as usize;

    let pitch = row_pitch(state.viewport_width);
    let card_w = card_width(state.viewport_width);
    let card_h = card_height(state.viewport_width);

    let first_visible_row = (state.scroll_offset / pitch) as usize;
    let visible_rows = (state.viewport_height / pitch) as usize + 1;
    let lo = first_visible_row.saturating_sub(BUFFER_ROWS);
    let hi = (first_visible_row + visible_rows + BUFFER_ROWS)
        .min(total_rows)
        .min(shown_rows);

    let mut visible: Vec<Element<'_, Message>> = Vec::new();

    for chunk in order.chunks(cols).skip(lo).take(hi.saturating_sub(lo)) {
        let mut cards: Vec<Element<'_, Message>> = Vec::new();

        for &index in chunk {
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
                container(column![img, text!("{}", catalog.title).size(14).bold()])
                    .width(Length::Fixed(card_w))
                    .height(Length::Fixed(card_h))
                    .clip(true)
                    .into()
            } else {
                placeholder_card(card_w, card_h)
            };

            cards.push(card);
        }

        while cards.len() < cols {
            cards.push(
                container(text!(""))
                    .width(Length::Fixed(card_w))
                    .height(Length::Fixed(card_h))
                    .into(),
            );
        }

        visible.push(row(cards).spacing(SPACING).width(Length::Fill).into());
    }

    let grid = column(visible).spacing(SPACING).width(Length::Fill);

    let top_pad = lo as f32 * pitch;
    let bottom_pad = (total_rows.saturating_sub(hi)) as f32 * pitch;

    container(
        column![
            toolbar,
            scrollable(column![
                container(text!(""))
                    .height(Length::Fixed(top_pad))
                    .width(Length::Fill),
                grid,
                container(text!(""))
                    .height(Length::Fixed(bottom_pad))
                    .width(Length::Fill),
            ])
            .height(Length::Fill)
            .width(Length::Fill)
            .on_scroll(|viewport| {
                let bounds = viewport.bounds();
                Message::Scrolled {
                    offset_y: viewport.absolute_offset().y,
                    width: bounds.width,
                    height: bounds.height,
                }
            }),
        ]
        .spacing(SPACING),
    )
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

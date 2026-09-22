use iced::{
    Element, Length, Theme,
    widget::{column, container, image, row, scrollable, text},
};

use iced::widget::container::Style;

use crate::{Message, State};

pub const MIN_COLS: usize = 5;
pub const MAX_COLS: usize = 7;
pub const SPACING: f32 = 8.0;
pub const BUFFER_ROWS: usize = 5;

// Approximate cell width the column count derives from: 5 columns at default
// window size, up to 7 when fullscreen.
const TARGET_CELL_WIDTH: f32 = 200.0;

pub fn cols_for_width(viewport_width: f32) -> usize {
    ((viewport_width / TARGET_CELL_WIDTH).round() as usize).clamp(MIN_COLS, MAX_COLS)
}

// Height / width of a cell (matches the 255x340 thumbnails).
pub const CELL_ASPECT: f32 = 340.0 / 255.0;

pub fn cell_width(viewport_width: f32) -> f32 {
    let cols = cols_for_width(viewport_width);
    ((viewport_width - (cols - 1) as f32 * SPACING) / cols as f32).max(1.0)
}

pub fn cell_height(viewport_width: f32) -> f32 {
    cell_width(viewport_width) * CELL_ASPECT
}

pub fn row_pitch(viewport_width: f32) -> f32 {
    cell_height(viewport_width) + SPACING
}

pub fn view(state: &State) -> Element<'_, Message> {
    let header = text!("Library");

    let Some(items) = &state.library_items else {
        return container(column![header, text!("Loading...")])
            .center(Length::Fill)
            .into();
    };

    let cols = cols_for_width(state.viewport_width);
    let total_rows = items.len() / cols + (items.len() % cols != 0) as usize;

    let pitch = row_pitch(state.viewport_width);
    let cell_w = cell_width(state.viewport_width);
    let cell_h = cell_height(state.viewport_width);

    let first_visible_row = (state.scroll_offset / pitch) as usize;
    let visible_rows = (state.viewport_height / pitch) as usize + 1;
    let lo = first_visible_row.saturating_sub(BUFFER_ROWS);
    let hi = (first_visible_row + visible_rows + BUFFER_ROWS).min(total_rows);

    let mut visible: Vec<Element<'_, Message>> = Vec::new();

    for chunk in items.chunks(cols).skip(lo).take(hi.saturating_sub(lo)) {
        let mut cells: Vec<Element<'_, Message>> = Vec::new();

        for item in chunk {
            let cell: Element<'_, Message> = if let Some(catalog) =
                state.catalog_items.get(item.catalog_item_id.as_ref())
            {
                if let Some((w, h, pixels)) = state.decoded_images.get(&catalog.id) {
                    let handle = iced::widget::image::Handle::from_rgba(*w, *h, pixels.to_vec());
                    let img = image(handle)
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .content_fit(iced::ContentFit::Cover);
                    container(img)
                        .width(Length::Fixed(cell_w))
                        .height(Length::Fixed(cell_h))
                        .clip(true)
                        .into()
                } else {
                    placeholder_cell(&item.sandbox_name, cell_w, cell_h)
                }
            } else {
                placeholder_cell(&item.sandbox_name, cell_w, cell_h)
            };

            cells.push(cell);
        }

        while cells.len() < cols {
            cells.push(
                container(text!(""))
                    .width(Length::Fixed(cell_w))
                    .height(Length::Fixed(cell_h))
                    .into(),
            );
        }

        visible.push(row(cells).spacing(SPACING).width(Length::Fill).into());
    }

    let grid = column(visible).spacing(SPACING).width(Length::Fill);

    let top_pad = lo as f32 * pitch;
    let bottom_pad = (total_rows.saturating_sub(hi)) as f32 * pitch;

    container(
        column![
            header,
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

fn placeholder_cell(name: &str, cell_w: f32, cell_h: f32) -> Element<'_, Message> {
    container(text!("{}", name).size(14).width(Length::Fill).center())
        .style(|theme: &Theme| Style::default().background(theme.palette().primary))
        .width(Length::Fixed(cell_w))
        .height(Length::Fixed(cell_h))
        .center_y(Length::Fixed(cell_h))
        .clip(true)
        .into()
}

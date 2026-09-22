use iced::{
    Element, Length, Theme,
    widget::{column, container, image, row, scrollable, text},
};

use iced::widget::container::Style;

use crate::{Message, State};

const CELL_WIDTH: f32 = 255.0;
const CELL_HEIGHT: f32 = 340.0;
const COLS: usize = 5;
const SPACING: f32 = 8.0;
const ROW_PITCH: f32 = CELL_HEIGHT + SPACING;
const BUFFER_ROWS: usize = 5;
const VIEWPORT_HEIGHT: f32 = 800.0;

pub fn view(state: &State) -> Element<'_, Message> {
    let header = text!("Library");

    let Some(items) = &state.library_items else {
        return container(column![header, text!("Loading...")])
            .center(Length::Fill)
            .into();
    };

    let total_rows = items.len() / COLS + (items.len() % COLS != 0) as usize;

    let first_visible_row = (state.scroll_offset / ROW_PITCH) as usize;
    let visible_rows = (VIEWPORT_HEIGHT / ROW_PITCH) as usize + 1;
    let lo = first_visible_row.saturating_sub(BUFFER_ROWS);
    let hi = (first_visible_row + visible_rows + BUFFER_ROWS).min(total_rows);

    let mut visible: Vec<Element<'_, Message>> = Vec::new();

    for chunk in items.chunks(COLS).skip(lo).take(hi.saturating_sub(lo)) {
        let mut cells: Vec<Element<'_, Message>> = Vec::new();

        for item in chunk {
            let cell: Element<'_, Message> =
                if let Some(catalog) = state.catalog_items.get(item.catalog_item_id.as_ref()) {
                    if let Some((w, h, pixels)) = state.decoded_images.get(&catalog.id) {
                        let handle = iced::widget::image::Handle::from_rgba(
                            *w,
                            *h,
                            pixels.to_vec(),
                        );
                        let img = image(handle)
                            .width(Length::Fixed(CELL_WIDTH))
                            .height(Length::Fixed(CELL_HEIGHT))
                            .content_fit(iced::ContentFit::Cover);
                        container(img)
                            .width(Length::Fixed(CELL_WIDTH))
                            .height(Length::Fixed(CELL_HEIGHT))
                            .into()
                    } else {
                        placeholder_cell(&item.sandbox_name)
                    }
                } else {
                    placeholder_cell(&item.sandbox_name)
                };

            cells.push(cell);
        }

        while cells.len() < COLS {
            cells.push(
                container(text!(""))
                    .width(Length::Fixed(CELL_WIDTH))
                    .height(Length::Fixed(CELL_HEIGHT))
                    .into(),
            );
        }

        visible.push(row(cells).spacing(SPACING).into());
    }

    let grid = column(visible).spacing(SPACING);

    let top_pad = lo as f32 * ROW_PITCH;
    let bottom_pad = (total_rows.saturating_sub(hi)) as f32 * ROW_PITCH;

    container(
        column![
            header,
            scrollable(
                column![
                    container(text!("")).height(Length::Fixed(top_pad)),
                    grid,
                    container(text!("")).height(Length::Fixed(bottom_pad)),
                ]
            )
            .height(Length::Fill)
            .on_scroll(|viewport| {
                Message::Scrolled(viewport.absolute_offset().y)
            }),
        ]
        .spacing(SPACING),
    )
    .padding(16)
    .into()
}

fn placeholder_cell(name: &str) -> Element<'_, Message> {
    container(text!("{}", name).size(14))
        .style(|theme: &Theme| Style::default().background(theme.palette().primary))
        .width(Length::Fixed(CELL_WIDTH))
        .height(Length::Fixed(CELL_HEIGHT))
        .center_y(Length::Fixed(CELL_HEIGHT))
        .into()
}

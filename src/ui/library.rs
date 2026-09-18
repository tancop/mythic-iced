use iced::{
    Element, Length, Theme,
    widget::{column, container, image, row, scrollable, text},
};

// separate line to stop rustfmt merging into function import
use iced::widget::container::Style;

use crate::{Message, State};

const CELL_WIDTH: f32 = 255.0;
const CELL_HEIGHT: f32 = 340.0;
const COLS: usize = 5;
const SPACING: f32 = 8.0;

pub fn view(state: &State) -> Element<'_, Message> {
    let header = text!("Library");

    let Some(items) = &state.library_items else {
        return container(column![header, text!("Loading...")])
            .center(Length::Fill)
            .into();
    };

    let mut rows: Vec<Element<'_, Message>> = Vec::new();

    for chunk in items.chunks(COLS) {
        let mut cells: Vec<Element<'_, Message>> = Vec::new();

        for item in chunk {
            let cell: Element<'_, Message> =
                if let Some(catalog) = state.catalog_items.get(item.catalog_item_id.as_ref()) {
                    let jpg = state.image_dir.join(format!("{}.jpg", catalog.id));
                    let png = state.image_dir.join(format!("{}.png", catalog.id));
                    let img_path = if jpg.exists() { jpg } else { png };
                    if img_path.exists() {
                        let handle = iced::widget::image::Handle::from_path(img_path);
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

        // pad incomplete rows
        while cells.len() < COLS {
            cells.push(
                container(text!(""))
                    .width(Length::Fixed(CELL_WIDTH))
                    .height(Length::Fixed(CELL_HEIGHT))
                    .into(),
            );
        }

        rows.push(row(cells).spacing(SPACING).into());
    }

    let grid = column(rows).spacing(SPACING);

    container(column![header, scrollable(grid).height(Length::Fill),])
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

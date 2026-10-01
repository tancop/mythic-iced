use iced::{
    Color, Element, Padding,
    widget::{mouse_area, row, text},
};

use crate::{Message, Page, State, ui::TextWidgetExt};

const INACTIVE_TAB: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.5);

fn tab<'a>(content: &'a str, active: bool, on_press: Message) -> Element<'a, Message> {
    mouse_area(
        text(content)
            .bold()
            .size(crate::HEADING_TEXT_SIZE)
            .color(if active { Color::WHITE } else { INACTIVE_TAB }),
    )
    .on_press(on_press)
    .into()
}

pub fn view(state: &State) -> Element<'_, Message> {
    row![
        tab(
            "Library",
            matches!(state.page, Page::Library),
            Message::Navigate(Page::Library)
        ),
        tab(
            "Details",
            matches!(state.page, Page::GameDetail),
            Message::Navigate(Page::GameDetail)
        ),
    ]
    .padding(Padding {
        top: 4.0,
        right: 16.0,
        bottom: 0.0,
        left: 16.0,
    })
    .spacing(10)
    .into()
}

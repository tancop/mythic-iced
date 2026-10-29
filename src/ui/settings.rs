use iced::{
    Alignment, Color, Element, Length, Padding,
    widget::{column, container, row, space, stack, text, text_input},
};

use crate::{
    Message, State,
    ui::{
        TextWidgetExt,
        icons::{COPY, icon_button},
        theme::{self, WARNING},
    },
};

pub fn view(state: &State) -> Element<'_, Message> {
    let token = state
        .auth_data
        .as_ref()
        .map(|auth| auth.access_token.as_str())
        .unwrap_or("");

    // Same inline-button pattern as the library search field: the input
    // stays the same widget tree so it keeps focus, and the copy button
    // floats on the right with padding keeping text out from under it.
    // `on_input` swallows edits so the value is effectively read-only
    // while remaining selectable.
    let overlay = row![space::horizontal()]
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(Alignment::Center)
        .padding(Padding {
            top: 0.0,
            right: 8.0,
            bottom: 0.0,
            left: 0.0,
        })
        .push(icon_button(
            COPY,
            theme::HIGHLIGHT,
            Message::CopyAccessToken,
        ));

    let field: Element<'_, Message> = stack![
        text_input("", token)
            .on_input(|_| Message::Ignored)
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

    container(
        column![
            text!("Access token").bold(),
            field,
            text!("Warning: this token grants full access to your account. Do not share it with anyone.")
                .size(14)
                .color(WARNING),
        ]
        .spacing(8),
    )
    .padding(16)
    .into()
}

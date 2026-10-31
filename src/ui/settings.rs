use iced::{
    Alignment, Border, Color, Element, Length, Padding, Shadow, Theme,
    widget::{button, checkbox, column, container, row, space, stack, text, text_input},
};

use crate::{
    Message, State, logging,
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

    let logging_row = row![
        checkbox(logging::debug_logging_enabled())
            .label("Debug logging")
            .on_toggle(Message::DebugLoggingToggled),
        // Plain text styled like a hyperlink; there is no dedicated
        // link widget in this iced version.
        text_button("Open log folder", theme::INFO, Message::OpenLogsFolder),
    ]
    .spacing(16)
    .align_y(Alignment::Center);

    container(
        column![
            text!("Access token").bold(),
            field,
            text!("Warning: this token grants full access to your account. Do not share it with anyone.")
                .size(14)
                .color(WARNING),
            space::vertical().height(16),
            text!("Logging").bold(),
            logging_row,
        ]
        .spacing(8),
    )
    .padding(16)
    .into()
}

/// Hyperlink-styled text button: colored label on a transparent
/// background, since this iced version has no dedicated link widget.
/// Same transparent style as `icons::icon_button`.
fn text_button<'a>(label: &'a str, color: Color, on_press: Message) -> Element<'a, Message> {
    button(text(label))
        .padding(0)
        .on_press(on_press)
        .style(move |_theme: &Theme, _status| button::Style {
            background: None,
            text_color: color,
            border: Border::default(),
            shadow: Shadow::default(),
            snap: true,
        })
        .into()
}

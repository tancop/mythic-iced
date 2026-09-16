use iced::{
    Alignment, Element, Length,
    widget::{button, column, container, text, text_input},
};

use crate::{Message, State};

pub fn view(_: &State) -> Element<'_, Message> {
    container(
        column![
            text!("Sign in with Epic Games"),
            button("Open").on_press(Message::StartLogin)
        ]
        .align_x(Alignment::Center),
    )
    .center(Length::Fill)
    .into()
}

pub fn view_paste_token(state: &State) -> Element<'_, Message> {
    container(
        column![
            text!("Paste your authorizationCode here:"),
            text_input("56e288...", "")
                .on_input(|_| Message::Ignored)
                .on_paste(|s| Message::SubmitToken(s))
                .width(400),
        ]
        .align_x(Alignment::Center),
    )
    .center(Length::Fill)
    .into()
}

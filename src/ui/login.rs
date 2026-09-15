use iced::{
    Element,
    widget::{button, column, container, text, text_input},
};

use crate::{Message, State};

pub fn view(_: &State) -> Element<'_, Message> {
    container(column![
        text!("Sign in with Epic Games"),
        button("Open").on_press(Message::StartLogin)
    ])
    .center(800)
    .into()
}

pub fn view_paste_token(state: &State) -> Element<'_, Message> {
    container(column![
        text!("Paste your token here:"),
        text_input("", &state.exchange_code)
            .on_input(|_| Message::Ignored)
            .on_paste(|s| Message::SubmitToken(s)),
    ])
    .center(800)
    .into()
}

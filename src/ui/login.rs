use iced::{Element, widget::text};

use crate::{Message, State};

pub fn view(_: &State) -> Element<'_, Message> {
    text!("Sign in with Epic Games").into()
}

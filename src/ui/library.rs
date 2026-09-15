use iced::{Element, widget::text};

use crate::{Message, State};

pub fn view(state: &State) -> Element<'_, Message> {
    if let Some(auth_data) = &state.auth_data {
        text!("Hi {}!", auth_data.display_name).into()
    } else {
        text!("signed out").into()
    }
}

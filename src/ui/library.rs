use iced::{
    Element, Length,
    widget::{column, container, text},
};

use crate::{Message, State};

pub fn view(state: &State) -> Element<'_, Message> {
    let Some(auth_data) = &state.auth_data else {
        return text!("signed out").into();
    };

    container(column![
        text!("Hi {}!", auth_data.display_name),
        if let Some(items) = &state.library_items {
            text!("You have {} items in your library.", items.len())
        } else {
            text!("Loading...")
        }
    ])
    .center(Length::Fill)
    .into()
}

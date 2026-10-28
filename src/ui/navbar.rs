use iced::{
    Alignment, Color, Element, Length, Padding,
    widget::{mouse_area, row, space, stack, text, text_input},
};

use crate::{
    Message, Page, State,
    ui::{
        TextWidgetExt,
        icons::{X_MARK, icon_button},
    },
};

const INACTIVE_TAB: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.5);

/// Fixed width of the search field, so it keeps the same size it had in
/// the library toolbar instead of stretching with the window.
const SEARCH_WIDTH: f32 = 550.0;

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

/// Search field anchored to the right end of the nav bar. The field always
/// renders as the same widget tree (a stack with the input underneath) so
/// the text input keeps focus while typing: swapping the whole field on the
/// first keystroke would despawn the focused input. Only the overlay button
/// itself is conditional. The input keeps extra right padding so long
/// queries don't slide underneath the button.
fn search_field(state: &State) -> Element<'_, Message> {
    let mut overlay = row![space::horizontal()]
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(Alignment::Center)
        .padding(Padding {
            top: 0.0,
            right: 8.0,
            bottom: 0.0,
            left: 0.0,
        });
    if !state.search_query.is_empty() {
        overlay = overlay.push(icon_button(
            X_MARK,
            Color::WHITE,
            Message::SearchQueryChanged(String::new()),
        ));
    }

    stack![
        text_input("Search...", &state.search_query)
            .on_input(Message::SearchQueryChanged)
            .on_submit(Message::SearchSubmitted)
            .padding(Padding {
                top: 5.0,
                right: 28.0,
                bottom: 5.0,
                left: 5.0,
            })
            .width(Length::Fill),
        overlay,
    ]
    .width(Length::Fixed(SEARCH_WIDTH))
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
        tab(
            "Settings",
            matches!(state.page, Page::Settings),
            Message::Navigate(Page::Settings)
        ),
        // Pushes the search field flush against the right edge instead of
        // letting it sit right after the Settings tab.
        space::horizontal(),
        search_field(state),
    ]
    .padding(Padding {
        top: 4.0,
        right: 16.0,
        bottom: 0.0,
        left: 16.0,
    })
    .spacing(10)
    .align_y(Alignment::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_field_builds_with_and_without_query() {
        // The clear button only exists for a non-empty query; both
        // variants must build without panicking.
        let _ = view(&State::default());
        let state = State {
            search_query: "civ".to_string(),
            ..State::default()
        };
        let _ = view(&state);
    }
}

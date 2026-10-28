use iced::{
    Border, Color, Element, Shadow, Theme,
    widget::{button, text},
};
use iced_widget::Text;

use crate::FA_SOLID;

pub const UP_DOWN_ARROW: char = '\u{f338}';
pub const ARROW_LEFT: char = '\u{f060}';
pub const X_MARK: char = '\u{f00d}';
pub const STAR: char = '\u{f005}';
pub const COPY: char = '\u{f0c5}';

/// Font Awesome icon shown at all times on a transparent background; state
/// is communicated only through the icon color, which the caller picks.
pub fn icon_button<'a, Message>(
    code_point: char,
    color: Color,
    on_press: Message,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    button(text(code_point).font(FA_SOLID).size(20))
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

pub fn icon(code_point: char, size: u32) -> Text<'static> {
    text(code_point).font(FA_SOLID).size(size).into()
}

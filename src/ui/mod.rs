use iced::theme::Custom;

pub mod library;
pub mod login;
mod theme;

pub fn get_theme() -> Custom {
    Custom::new("Mythic".into(), theme::MAIN_PALETTE)
}

pub trait TextWidgetExt {
    fn bold(self) -> Self;
}

impl TextWidgetExt for iced::widget::Text<'_> {
    fn bold(self) -> Self {
        self.font(crate::BOLD_FONT)
    }
}

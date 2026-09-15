use iced::theme::Custom;

pub mod library;
pub mod login;
mod theme;

pub fn get_theme() -> Custom {
    Custom::new("Mythic".into(), theme::MAIN_PALETTE)
}

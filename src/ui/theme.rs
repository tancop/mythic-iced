use iced::{Color, theme::Palette};

const fn hsl(h: f32, s: f32, l: f32) -> Color {
    const fn f(h: f32, s: f32, l: f32, n: u8) -> f32 {
        let k = n as f32 + (h / 30.0) % 12.0;
        let a = s * (l.min(1.0 - l));
        l - a * ((k - 3.0).min(9.0 - k).min(1.0)).max(-1.0)
    }

    Color::from_rgb(f(h, s, l, 0), f(h, s, l, 8), f(h, s, l, 4))
}

pub const BACKGROUND: Color = hsl(195.0, 0.1, 0.1);

pub const TEXT: Color = Color::WHITE;

pub const ALT: Color = hsl(195.0, 0.1, 0.2);

pub const SUCCESS: Color = hsl(130.0, 1.0, 0.5);

pub const WARNING: Color = hsl(61.0, 0.9, 0.5);

pub const DANGER: Color = hsl(6.0, 0.9, 0.5);

pub const INFO: Color = hsl(210.0, 1.0, 0.5);

/// Mythic rarity color taken from Fortnite wiki
pub const BRAND_COLOR: Color = Color::from_rgb8(0xed, 0xbe, 0x51);

// OpenCritic color palette

pub const OPENCRITIC_WEAK: Color = Color::from_rgb8(0x80, 0xb0, 0x6a);

pub const OPENCRITIC_FAIR: Color = Color::from_rgb8(0x4a, 0xa1, 0xce);

pub const OPENCRITIC_STRONG: Color = Color::from_rgb8(0x9e, 0x00, 0xb4);

pub const OPENCRITIC_MIGHTY: Color = Color::from_rgb8(0xfc, 0x43, 0x0a);

pub const MAIN_PALETTE: Palette = Palette {
    background: BACKGROUND,
    text: TEXT,
    primary: ALT,
    success: SUCCESS,
    warning: WARNING,
    danger: DANGER,
};

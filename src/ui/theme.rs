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

pub const MAIN_PALETTE: Palette = Palette {
    background: BACKGROUND,
    text: TEXT,
    primary: ALT,
    success: SUCCESS,
    warning: WARNING,
    danger: DANGER,
};

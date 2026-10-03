use iced::alignment::Vertical;
use iced::{Color, Font, Pixels, Radians, Renderer};
use iced_widget::canvas::Stroke;
use iced_widget::canvas::path::arc::Arc;
use iced_widget::canvas::{self, Text};
use iced_widget::text::Alignment;

pub struct ProgressCircle {
    pub progress: f32,
    pub color: Color,
    pub text: String,
    pub font: &'static Font,
    pub text_size: Pixels,
}

impl<Message> canvas::Program<Message> for ProgressCircle {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &iced_widget::renderer::core::Theme,
        bounds: iced::Rectangle,
        _cursor: iced_widget::core::mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let center = frame.center();

        // Inset by half the stroke so the ring stays inside the frame.
        let stroke_width = 6.0;
        let radius = (bounds.width.min(bounds.height) / 2.0 - stroke_width / 2.0).max(0.0);

        // Start at the top, sweep clockwise with progress.
        let start = Radians(-std::f32::consts::FRAC_PI_2);
        let arc = canvas::Path::new(|path| {
            path.arc(Arc {
                center,
                radius,
                start_angle: start,
                end_angle: Radians(start.0 + self.progress * 2.0 * std::f32::consts::PI),
            });
        });

        frame.stroke(
            &arc,
            Stroke {
                width: stroke_width,
                style: canvas::Style::Solid(self.color),
                line_cap: canvas::LineCap::Round,
                ..Default::default()
            },
        );

        frame.fill_text(Text {
            content: self.text.to_owned(),
            position: center,
            color: Color::WHITE,
            size: self.text_size,
            font: *self.font,
            align_x: Alignment::Center,
            align_y: Vertical::Center,
            ..Default::default()
        });

        vec![frame.into_geometry()]
    }
}

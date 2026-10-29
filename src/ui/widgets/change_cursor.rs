use iced::{Element, mouse};
use iced_widget::core::Widget;

pub struct ChangeCursor<'a, Message, Theme = iced::Theme, Renderer = iced::Renderer> {
    inner: Element<'a, Message, Theme, Renderer>,
    interaction: mouse::Interaction,
}

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for ChangeCursor<'a, Message, Theme, Renderer>
where
    Renderer: iced_widget::core::Renderer,
{
    fn size(&self) -> iced::Size<iced::Length> {
        self.inner.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut iced_widget::core::widget::Tree,
        renderer: &Renderer,
        limits: &iced_widget::core::layout::Limits,
    ) -> iced_widget::core::layout::Node {
        self.inner.as_widget_mut().layout(tree, renderer, limits)
    }

    fn draw(
        &self,
        tree: &iced_widget::core::widget::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &iced_widget::core::renderer::Style,
        layout: iced_widget::core::Layout<'_>,
        cursor: iced_widget::core::mouse::Cursor,
        viewport: &iced::Rectangle,
    ) {
        self.inner
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport)
    }

    fn mouse_interaction(
        &self,
        _tree: &iced_widget::core::widget::Tree,
        _layout: iced_widget::core::Layout<'_>,
        _cursor: iced_widget::core::mouse::Cursor,
        _viewport: &iced::Rectangle,
        _renderer: &Renderer,
    ) -> iced_widget::core::mouse::Interaction {
        self.interaction
    }
}

impl<'a, Message, Theme, Renderer> From<ChangeCursor<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: iced_widget::core::Renderer + 'a,
{
    fn from(change_cursor: ChangeCursor<'a, Message, Theme, Renderer>) -> Self {
        Element::new(change_cursor)
    }
}

pub fn change_cursor<'a, Message, Theme, Renderer: iced_widget::core::Renderer>(
    inner: Element<'a, Message, Theme, Renderer>,
    interaction: mouse::Interaction,
) -> ChangeCursor<'a, Message, Theme, Renderer> {
    ChangeCursor { inner, interaction }
}

//! Virtualized card grid: renders only the visible row window (plus a buffer)
//! inside a scrollable padded to the full content height, so the scrollbar
//! stays stable while content streams in.

use iced::{
    Element, Length, Task,
    widget::{
        column, container, operation::scroll_to, row, scrollable, scrollable::AbsoluteOffset, text,
    },
};

/// Stable id of the grid scrollable, so sort/search/filter changes can drive
/// it back to the top programmatically.
pub const GRID_SCROLL_ID: &str = "library-grid";

/// Task that really moves the grid scrollbar to the top (setting
/// `scroll_offset` alone only affects thumbnail loading, not the widget).
pub fn scroll_to_top<Message>() -> Task<Message> {
    scroll_to(GRID_SCROLL_ID, AbsoluteOffset { x: 0.0, y: 0.0 })
}

/// Task that restores the grid scrollbar to a previously saved offset.
/// Needed when returning to the library: its scrollable is recreated at
/// the top while `state.scroll_offset` still holds the old position.
pub fn scroll_to_offset<Message>(y: f32) -> Task<Message> {
    scroll_to(GRID_SCROLL_ID, AbsoluteOffset { x: 0.0, y })
}

/// Everything about the grid layout that a caller might want to tune.
#[derive(Clone, Copy, Debug)]
pub struct GridConfig {
    /// Card width the column count derives from; the grid keeps adding
    /// columns as the viewport widens (no min/max clamp).
    pub target_card_width: f32,
    pub spacing: f32,
    pub buffer_rows: usize,
    /// Card height / width.
    pub card_aspect: f32,
}

/// Current scroll state, fed back through the scrollable's `on_scroll`.
#[derive(Clone, Copy, Debug)]
pub struct GridViewport {
    pub width: f32,
    pub height: f32,
    pub scroll_offset: f32,
}

/// Column count for the current width. Never zero; unbounded above.
pub fn columns(config: &GridConfig, viewport_width: f32) -> usize {
    ((viewport_width / config.target_card_width).round() as usize).max(1)
}

/// Card (width, height) for the current width.
pub fn card_size(config: &GridConfig, viewport_width: f32) -> (f32, f32) {
    let cols = columns(config, viewport_width);
    let width = ((viewport_width - (cols - 1) as f32 * config.spacing) / cols as f32).max(1.0);
    (width, width * config.card_aspect)
}

pub fn row_pitch(config: &GridConfig, viewport_width: f32) -> f32 {
    let (_, height) = card_size(config, viewport_width);
    height + config.spacing
}

/// Row count for `item_count` items at the current width.
fn rows_for_items(config: &GridConfig, viewport_width: f32, item_count: usize) -> usize {
    item_count.div_ceil(columns(config, viewport_width))
}

/// Visible row window `[lo, hi)` extended by the buffer, clamped to the
/// rows needed for `item_count` items.
pub fn visible_range(
    config: &GridConfig,
    viewport: &GridViewport,
    item_count: usize,
) -> (usize, usize) {
    let total_rows = rows_for_items(config, viewport.width, item_count);
    let pitch = row_pitch(config, viewport.width);
    let first_visible_row = (viewport.scroll_offset / pitch) as usize;
    let visible_rows = (viewport.height / pitch) as usize + 1;
    let lo = first_visible_row.saturating_sub(config.buffer_rows);
    let hi = (first_visible_row + visible_rows + config.buffer_rows).min(total_rows);
    (lo, hi)
}

/// Grid over `item_count` items in display order. `render_item` maps a flat
/// display position plus the cell size to a card; `on_scroll` receives
/// `(offset_y, width, height)` like the previous inline handler did.
pub fn virtual_grid<'a, Message: 'a>(
    config: GridConfig,
    viewport: GridViewport,
    item_count: usize,
    render_item: impl Fn(usize, f32, f32) -> Element<'a, Message> + 'a,
    on_scroll: impl Fn(f32, f32, f32) -> Message + 'a,
) -> Element<'a, Message> {
    let cols = columns(&config, viewport.width);
    let (card_w, card_h) = card_size(&config, viewport.width);
    let pitch = card_h + config.spacing;
    let total_rows = rows_for_items(&config, viewport.width, item_count);
    let (lo, hi) = visible_range(&config, &viewport, item_count);

    let mut visible: Vec<Element<'a, Message>> = Vec::new();

    for row_index in lo..hi {
        let mut cards: Vec<Element<'a, Message>> = Vec::with_capacity(cols);

        for col in 0..cols {
            let index = row_index * cols + col;
            if index < item_count {
                cards.push(render_item(index, card_w, card_h));
            } else {
                cards.push(
                    container(text!(""))
                        .width(Length::Fixed(card_w))
                        .height(Length::Fixed(card_h))
                        .into(),
                );
            }
        }

        visible.push(
            row(cards)
                .spacing(config.spacing)
                .width(Length::Fill)
                .into(),
        );
    }

    let grid = column(visible).spacing(config.spacing).width(Length::Fill);

    let top_pad = lo as f32 * pitch;
    let bottom_pad = total_rows.saturating_sub(hi) as f32 * pitch;

    scrollable(column![
        container(text!(""))
            .height(Length::Fixed(top_pad))
            .width(Length::Fill),
        grid,
        container(text!(""))
            .height(Length::Fixed(bottom_pad))
            .width(Length::Fill),
    ])
    .id(GRID_SCROLL_ID)
    .height(Length::Fill)
    .width(Length::Fill)
    .on_scroll(move |scroll| {
        let bounds = scroll.bounds();
        on_scroll(scroll.absolute_offset().y, bounds.width, bounds.height)
    })
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: GridConfig = GridConfig {
        target_card_width: 200.0,
        spacing: 8.0,
        buffer_rows: 5,
        card_aspect: 340.0 / 255.0,
    };

    #[test]
    fn columns_scale_without_clamp() {
        assert_eq!(columns(&CONFIG, 1024.0), 5);
        assert_eq!(columns(&CONFIG, 100.0), 1);
        assert_eq!(columns(&CONFIG, 0.0), 1);
        // No upper clamp: very wide windows keep adding columns.
        assert_eq!(columns(&CONFIG, 4000.0), 20);
    }

    #[test]
    fn rows_round_up() {
        let cols = columns(&CONFIG, 1024.0);
        assert_eq!(rows_for_items(&CONFIG, 1024.0, cols * 2), 2);
        assert_eq!(rows_for_items(&CONFIG, 1024.0, cols * 2 + 1), 3);
        assert_eq!(rows_for_items(&CONFIG, 1024.0, 0), 0);
    }

    #[test]
    fn window_covers_visible_rows_plus_buffer() {
        let viewport = GridViewport {
            width: 1024.0,
            height: 768.0,
            scroll_offset: 0.0,
        };
        // 5 columns at this width, so 500 items fill 100 rows.
        let (lo, hi) = visible_range(&CONFIG, &viewport, 500);
        assert_eq!(lo, 0);
        // 3 visible rows + 5 buffer rows.
        assert_eq!(hi, 8);

        let pitch = row_pitch(&CONFIG, viewport.width);
        let scrolled = GridViewport {
            scroll_offset: pitch * 10.0,
            ..viewport
        };
        assert_eq!(visible_range(&CONFIG, &scrolled, 500), (5, 18));
    }

    #[test]
    fn window_clamps_to_item_count() {
        let viewport = GridViewport {
            width: 1024.0,
            height: 768.0,
            scroll_offset: 0.0,
        };
        assert_eq!(visible_range(&CONFIG, &viewport, 15), (0, 3));
        assert_eq!(visible_range(&CONFIG, &viewport, 0), (0, 0));
    }
}

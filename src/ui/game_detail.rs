use std::collections::HashMap;

use iced::{
    Alignment, Border, Color, Element, Length, Padding, Theme,
    alignment::Vertical,
    padding,
    widget::{column, container, image, markdown, rich_text, row, scrollable, text},
};
use iced_widget::{canvas, container::Style, space};

use crate::{
    HEADING_TEXT_SIZE, Message, Page, State,
    epic::{CriticRating, GameDetails, TechRequirement},
    images::PixelData,
    ui::{
        TextWidgetExt,
        icons::{self, ARROW_LEFT, icon, icon_button},
        library::GRID_CONFIG,
        theme::{
            INFO, MYTHIC_GOLD, OPENCRITIC_FAIR, OPENCRITIC_MIGHTY, OPENCRITIC_STRONG,
            OPENCRITIC_WEAK,
        },
        virtual_grid::DETAIL_SCROLL_ID,
        widgets::progress_circle::ProgressCircle,
    },
};

fn centered(label: &str) -> Element<'_, Message> {
    container(text!("{}", label)).center(Length::Fill).into()
}

fn meta_row<'a>(label: &str, value: &str) -> Element<'a, Message> {
    row![text!("{label}:").bold(), text!("{}", value)]
        .spacing(8)
        .into()
}

fn is_none_or_empty(value: &Option<String>) -> bool {
    value.as_deref().is_none_or(|v| v.is_empty())
}

/// Close the gap in `[label] (url)` / `![alt] (url)` pairs and inline the
/// `[label] [https://url]` reference targets Epic sometimes emits, which
/// strict markdown does not parse as links or images (a reference link
/// without a definition renders as literal text).
fn fix_link_spacing(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(idx) = rest.find(']') {
        let after_bracket = &rest[idx + ']'.len_utf8()..];
        let spaces = after_bracket.len() - after_bracket.trim_start_matches([' ', '\t']).len();
        let tail = &after_bracket[spaces..];
        if spaces > 0 && tail.starts_with('(') {
            out.push_str(&rest[..=idx]);
            out.push('(');
            rest = &tail['('.len_utf8()..];
        } else if let Some((target, len)) = inline_reference_target(tail) {
            out.push_str(&rest[..=idx]);
            out.push('(');
            out.push_str(target);
            out.push(')');
            rest = &tail[len..];
        } else {
            out.push_str(&rest[..=idx]);
            rest = after_bracket;
        }
    }
    out.push_str(rest);
    out
}

/// Pull the whitespace Epic can leave inside closing emphasis markers (e.g.
/// `_"More please!" _`) out of the pair, so it renders as italics like
/// tightly spaced `_text_`; loose markdown tolerates the gap, strict
/// CommonMark renders such runs literally. Runs of `_`/`*` are paired by
/// matching length and delimiter, innermost-first; a run preceded by
/// whitespace is the gap the pair needs to lose, and the space between the
/// pair and the following text stays outside where it belongs. Intra-word
/// underscores (`snake_case`), escaped markers and unmatched runs stay
/// verbatim.
fn fix_emphasis_spacing(line: &str) -> String {
    let bytes = line.as_bytes();

    // Maximal runs of `_`/`*`, skipping backslash-escaped markers.
    let mut runs: Vec<(usize, usize, char)> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let delim = bytes[i];
        if (delim == b'_' || delim == b'*') && (i == 0 || bytes[i - 1] != b'\\') {
            let start = i;
            while i < bytes.len() && bytes[i] == delim {
                i += 1;
            }
            runs.push((start, i - start, delim as char));
        } else {
            i += 1;
        }
    }

    let mut out = String::with_capacity(line.len());
    let mut copied = 0;
    // Unmatched opening runs as (content start, length, delimiter); pairs
    // nest, so the topmost one closes first (`_a **b** _`).
    let mut open: Vec<(usize, usize, char)> = Vec::new();
    for (start, len, delim) in runs {
        if let Some((open_end, open_len, open_delim)) = open.last().copied()
            && open_len == len
            && open_delim == delim
            && !line[open_end..start].trim().is_empty()
        {
            let content = &line[open_end..start];
            let trailing = content.len() - content.trim_end().len();
            out.push_str(&line[copied..start - trailing]);
            for _ in 0..len {
                out.push(delim);
            }
            copied = start + len;
            open.pop();
            continue;
        }
        // Otherwise this run may open its own pair. `_` cannot open
        // intra-word (`snake_case`), `*` only needs text to follow.
        let preceded_ok = line[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        if line[start + len..]
            .chars()
            .next()
            .is_some_and(|c| !c.is_whitespace())
            && (delim == '*' || preceded_ok)
        {
            open.push((start + len, len, delim));
        }
    }
    out.push_str(&line[copied..]);
    out
}

/// Match the `[https://target]` second bracket of a reference-style link and
/// return the URL plus the consumed byte length, so it can be rewritten as an
/// inline `(https://target)`. `None` when the bracket doesn't hold a URL:
/// definitions of real reference links (e.g. `[note][1]`) must keep working.
fn inline_reference_target(rest: &str) -> Option<(&str, usize)> {
    let inner = rest.strip_prefix('[')?;
    let end = inner.find(']')?;
    let target = &inner[..end];
    if target.starts_with("https://") || target.starts_with("http://") {
        // `[` + target + `]`, all single bytes.
        Some((target, end + 2))
    } else {
        None
    }
}

/// Normalize Epic's `longDescription` flavor into strict markdown: only `•`
/// starts a list, `[label] (url)` gaps are closed, `[label] [url]` reference
/// targets holding a URL are inlined, whitespace inside closing emphasis
/// markers is pulled out of the pair, and single newlines become hard line
/// breaks. Every other list marker (`*`, `-`, `+`, `1.`/`1)`) is
/// backslash-escaped so it renders verbatim instead of starting a list.
/// Fenced code blocks pass through untouched.
pub fn clean_description(raw: &str) -> String {
    let raw_lines: Vec<&str> = raw.lines().collect();

    // Per-line fixes, tracking fenced code blocks.
    let mut lines: Vec<(String, bool)> = Vec::with_capacity(raw_lines.len());
    let mut in_fence = false;
    for line in raw_lines {
        let trimmed = line.trim_start();
        let is_fence = trimmed.starts_with("```") || trimmed.starts_with("~~~");
        if is_fence {
            in_fence = !in_fence;
        }
        if in_fence || is_fence {
            lines.push((line.to_string(), true));
            continue;
        }
        let mut fixed = fix_link_spacing(line);
        fixed = fix_emphasis_spacing(&fixed);
        if let Some(bullet) = fixed.trim_start().strip_prefix('•') {
            fixed = format!("- {}", bullet.trim_start());
        } else if let Some(escaped) = escape_list_marker(&fixed) {
            fixed = escaped;
        }
        lines.push((fixed, false));
    }

    // Single newlines end the line: harden them, leaving blank-line paragraph
    // breaks and fenced code alone.
    let mut out = String::with_capacity(raw.len());
    for (i, (line, is_code)) in lines.iter().enumerate() {
        let next_text = lines
            .get(i + 1)
            .is_some_and(|(next, _)| !next.trim().is_empty());
        out.push_str(line);
        if !is_code
            && !line.trim().is_empty()
            && next_text
            && !line.ends_with("  ")
            && !line.ends_with('\\')
        {
            out.push_str("  ");
        }
        out.push('\n');
    }

    // Collapse the blank lines comment expansion leaves behind.
    let mut collapsed = out.trim().to_string();
    while collapsed.contains("\n\n\n") {
        collapsed = collapsed.replace("\n\n\n", "\n\n");
    }
    collapsed
}

/// Backslash-escape a markdown list marker so the line renders verbatim
/// instead of starting a list. Only `•` starts a list; a leading `*`, `-`
/// or `+` followed by a space, or an ordered `1.`/`1)` marker (e.g. 911
/// Operator's `*` legal disclaimer), is escaped. Doubled markers (`**bold**`,
/// `---` rules) and `*emphasis*` are left alone, preserving indentation.
fn escape_list_marker(line: &str) -> Option<String> {
    let indent_len = line.len() - line.trim_start().len();
    let (indent, trimmed) = line.split_at(indent_len);
    if let Some(marker) = trimmed
        .chars()
        .next()
        .filter(|c| matches!(c, '*' | '-' | '+'))
    {
        let rest = &trimmed[marker.len_utf8()..];
        if !rest.starts_with(marker) && rest.starts_with([' ', '\t']) {
            return Some(format!("{indent}\\{marker}{rest}"));
        }
        return None;
    }
    let digit_len: usize = trimmed
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .map(char::len_utf8)
        .sum();
    if digit_len > 0 && digit_len <= 9 {
        let after_digits = &trimmed[digit_len..];
        if let Some(sep) = after_digits
            .chars()
            .next()
            .filter(|c| matches!(c, '.' | ')'))
        {
            let rest = &after_digits[sep.len_utf8()..];
            if rest.starts_with([' ', '\t']) {
                return Some(format!("{indent}{}\\{}{rest}", &trimmed[..digit_len], sep));
            }
        }
    }
    None
}

/// Clean Epic's description and parse it for `markdown::view_with`. Called
/// once when details load; only the parsed items are stored in
/// `State::detail_bodies` because the view borrows them. Layout (spacing,
/// widths, viewer style) is rebuilt dynamically on every view so it stays
/// responsive.
pub fn parse_description(raw: &str) -> Vec<markdown::Item> {
    markdown::parse(&clean_description(raw)).collect()
}

/// URLs of the images embedded in a parsed description, so their bytes can
/// be fetched when details load and `DetailViewer` can render them in place
/// of the alt text. Quotes can nest images, so recurse into them.
pub fn description_image_urls(items: &[markdown::Item]) -> Vec<String> {
    items
        .iter()
        .flat_map(|item| match item {
            markdown::Item::Image { url, .. } => vec![url.clone()],
            markdown::Item::Quote(items) => description_image_urls(items),
            _ => vec![],
        })
        .collect()
}

fn markdown_settings() -> markdown::Settings {
    let mut settings = markdown::Settings::with_text_size(
        crate::DEFAULT_TEXT_SIZE as f32,
        markdown::Style {
            font: crate::UI_FONT,
            ..markdown::Style::from_palette(crate::ui::theme::MAIN_PALETTE)
        },
    );
    // Epic's scale is much tighter than iced's default (h1 is double size):
    // h1 is 4pt larger bold, h2 is 2pt larger.
    settings.h1_size = settings.text_size + 4.0;
    settings.h2_size = settings.text_size + 2.0;
    settings
}

/// Epic-flavored markdown viewer: descriptions use `#` headings for section
/// titles, but iced's default h1 is double-size medium. Epic renders h1 as
/// 4pt-larger bold instead (h2 size + 2pt), so remap h1 to that style.
/// Description images are shown via iced's built-in `image` widget at their
/// original size (`ScaleDown`: never upscaled, so inline decoration stays
/// pixel-exact); until their bytes arrive, the alt text is displayed.
struct DetailViewer<'a> {
    images: &'a HashMap<String, iced::widget::image::Handle>,
}

impl<'a> markdown::Viewer<'a, Message> for DetailViewer<'a> {
    fn on_link_click(url: markdown::Uri) -> Message {
        Message::OpenUrl(url)
    }

    fn image(
        &self,
        settings: markdown::Settings,
        url: &'a markdown::Uri,
        _title: &'a str,
        alt: &markdown::Text,
    ) -> Element<'a, Message> {
        match self.images.get(url) {
            Some(handle) => image(handle.clone())
                .content_fit(iced::ContentFit::ScaleDown)
                .into(),
            // iced's own fallback: the alternative text as a code block.
            None => container(rich_text(alt.spans(settings.style)))
                .padding(settings.spacing.0)
                .into(),
        }
    }

    fn heading(
        &self,
        settings: markdown::Settings,
        level: &'a markdown::HeadingLevel,
        text: &'a markdown::Text,
        index: usize,
    ) -> Element<'a, Message> {
        if *level == markdown::HeadingLevel::H1 {
            let style = markdown::Style {
                font: crate::BOLD_FONT,
                ..settings.style
            };
            return container(
                rich_text(text.spans(style))
                    .on_link_click(Self::on_link_click)
                    .size(settings.h2_size + 2.0)
                    .font(crate::BOLD_FONT),
            )
            .padding(iced::padding::top(if index > 0 {
                settings.text_size / 2.0
            } else {
                iced::Pixels::ZERO
            }))
            .into();
        }
        markdown::heading(settings, level, text, index, Self::on_link_click)
    }
}

fn detail_scrollable(content: iced::widget::Column<'_, Message>) -> Element<'_, Message> {
    container(
        scrollable(content.height(Length::Shrink).width(Length::Fill))
            .id(DETAIL_SCROLL_ID)
            .height(Length::Fill)
            .width(Length::Fill)
            .on_scroll(|viewport| {
                let bounds = viewport.bounds();
                Message::DetailScrolled {
                    offset_y: viewport.absolute_offset().y,
                    width: bounds.width,
                    height: bounds.height,
                }
            }),
    )
    .padding(24)
    .into()
}

fn requirements<'a>(platform: &str, reqs: &[TechRequirement]) -> Element<'a, Message> {
    let mut body = column![text!("System requirements ({platform})").bold()].spacing(8);
    for req in reqs {
        if is_none_or_empty(&req.minimum) && is_none_or_empty(&req.recommended) {
            continue;
        }
        body = body.push(
            column![
                text!("{}", req.title).bold().size(14),
                text!("Minimum: {}", req.minimum.as_deref().unwrap_or("--")).size(14),
                text!(
                    "Recommended: {}",
                    req.recommended.as_deref().unwrap_or("--")
                )
                .size(14),
            ]
            .spacing(4),
        );
    }
    body.spacing(12).into()
}

fn opencritic_color(rating: CriticRating) -> Color {
    match rating {
        CriticRating::Weak => OPENCRITIC_WEAK,
        CriticRating::Fair => OPENCRITIC_FAIR,
        CriticRating::Strong => OPENCRITIC_STRONG,
        CriticRating::Mighty => OPENCRITIC_MIGHTY,
    }
}

const GOLD_RATING: f32 = 4.5;

fn rating_box(rating: f32) -> Element<'static, Message> {
    container(
        row![
            if rating >= GOLD_RATING {
                icon(icons::STAR, 20).color(MYTHIC_GOLD)
            } else {
                icon(icons::STAR, 20)
            },
            text!("{rating:.2}").size(HEADING_TEXT_SIZE).bold()
        ]
        .align_y(Vertical::Center)
        .spacing(5),
    )
    .padding(padding::horizontal(10).vertical(5))
    .style(move |theme: &Theme| {
        Style::default()
            .background(theme.palette().primary)
            .border(Border::default().rounded(6.0))
    })
    .into()
}

pub fn view(state: &State) -> Element<'_, Message> {
    // Back button at the top of a narrow left sidebar; the details pane
    // fills the rest of the window.
    row![side_bar(), details_view(state)]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// The detail page is only reachable from the library grid, so going back
/// always returns there.
fn side_bar() -> Element<'static, Message> {
    container(column![back_button(), space::vertical()].height(Length::Fill))
        .width(Length::Fixed(64.0))
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .padding(Padding {
            top: 16.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        })
        .style(|theme: &Theme| Style::default().background(theme.palette().primary))
        .into()
}

fn back_button() -> Element<'static, Message> {
    icon_button(ARROW_LEFT, Color::WHITE, Message::Navigate(Page::Library))
}

fn details_view(state: &State) -> Element<'_, Message> {
    let Some(items) = &state.catalog_items else {
        return centered("Loading...");
    };
    let Some(item) = (match state.page {
        Page::GameDetail { game_id } => items.get(game_id),
        _ => None,
    }) else {
        return centered("Select a game from the library.");
    };
    let Some(full) = state.game_details.get(&item.namespace) else {
        return centered(if state.details_error {
            "Couldn't load game details."
        } else {
            "Loading..."
        });
    };

    let title = full.title(&item.title);
    let offer = &full.offer;

    let cover_width = 220.0;
    let cover: Element<'_, Message> = match state.decoded_images.get(&item.id) {
        Some(PixelData {
            width,
            height,
            pixels,
        }) => {
            let handle = iced::widget::image::Handle::from_rgba(*width, *height, pixels.to_vec());
            image(handle).width(Length::Fixed(cover_width)).into()
        }
        None => container("")
            .width(Length::Fixed(cover_width))
            .height(Length::Fixed(cover_width * GRID_CONFIG.card_aspect))
            .into(),
    };

    // Narrow identity column next to the cover; the long description gets the
    // full width below.
    let mut identity = column![text!("{}", title).bold().size(24)]
        .spacing(8)
        .width(Length::Fill);
    if !offer.developer_display_name.is_empty() {
        identity = identity.push(meta_row("Developer", &offer.developer_display_name));
    }
    if !offer.publisher_display_name.is_empty() {
        identity = identity.push(meta_row("Publisher", &offer.publisher_display_name));
    }
    if let Some(rating) = item.user_rating {
        identity = identity.push(rating_box(rating));
    }
    if let Some(critic) = item.critic.as_ref() {
        let score = column![
            canvas(ProgressCircle {
                progress: critic.average as f32 / 100.0,
                color: opencritic_color(critic.rating),
                text: critic.average.to_string(),
                font: &crate::BOLD_FONT,
                text_size: crate::HEADING_TEXT_SIZE.into(),
            })
            .height(60.0)
            .width(60.0),
            text!("{}", critic.text_rating())
                .size(12)
                .width(Length::Fill)
                .bold()
                .center(),
        ]
        .spacing(4)
        .width(Length::Shrink);

        let recommend = column![
            canvas(ProgressCircle {
                progress: critic.recommend_percentage as f32 / 100.0,
                color: if critic.recommend_percentage == 100 {
                    MYTHIC_GOLD
                } else {
                    INFO
                },
                text: format!("{}%", critic.recommend_percentage),
                font: &crate::BOLD_FONT,
                text_size: if critic.recommend_percentage == 100 {
                    crate::DEFAULT_TEXT_SIZE.into()
                } else {
                    crate::HEADING_TEXT_SIZE.into()
                },
            })
            .height(60.0)
            .width(60.0),
            text!("Recommend")
                .size(12)
                .width(Length::Fill)
                .bold()
                .center(),
        ]
        .spacing(4)
        .width(Length::Shrink);

        identity = identity.push(row![score, recommend].spacing(16));

        if !critic.url.is_empty() {
            identity = identity.push(text!("{}", critic.url).size(12));
        }
    }

    let mut content = column![row![cover, identity].spacing(24)]
        .spacing(20)
        .width(Length::Fill);

    if let Some(description) = state.detail_bodies.get(&item.namespace) {
        // Only the parsed items are cached in `State::detail_bodies`; the
        // layout (spacing, width, viewer style) is rebuilt every view so it
        // stays responsive to window size.
        content = content.push(row![
            container(markdown::view_with(
                description,
                markdown_settings(),
                &DetailViewer {
                    images: &state.detail_images,
                },
            ))
            .width(if state.viewport_width < (800.0 * 0.9) {
                Length::FillPortion(9)
            } else {
                Length::Fixed(800.0)
            })
            .padding(Padding {
                top: 8.0,
                bottom: 8.0,
                ..Padding::ZERO
            }),
            space().width(Length::FillPortion(1)),
        ]);
    } else {
        content = content.push(text!("{}", full.description()));
    }

    let mut meta = column![].spacing(8);
    if let Some(seller) = offer.seller.as_ref().filter(|s| !s.name.is_empty()) {
        meta = meta.push(meta_row("Seller", &seller.name));
    }
    if !offer.offer_type.is_empty() {
        meta = meta.push(meta_row("Offer type", &offer.offer_type));
    }
    if let Some(date) = full.release_date() {
        meta = meta.push(meta_row("Release date", date));
    }
    content = content.push(meta);

    // Store configuration is only present for products with a store page.
    // Offer-level info above is always shown; the rest needs `Full` details.
    let GameDetails::Full(details) = &full.details else {
        content = content.push(text!("No further information available").size(14));
        return detail_scrollable(content);
    };

    if let Some(website) = details.game_website.as_deref().filter(|s| !s.is_empty()) {
        content = content.push(meta_row("Website", website));
    }
    if let Some(link) = details.privacy_link.as_deref().filter(|s| !s.is_empty()) {
        content = content.push(meta_row("Privacy", link));
    }
    if let Some(review) = details
        .social_links
        .as_deref()
        .filter(|links| !links.is_empty())
    {
        let socials = review
            .iter()
            .map(|link| format!("{}: {}", link.platform, link.url))
            .collect::<Vec<_>>()
            .join(", ");
        content = content.push(meta_row("Social", &socials));
    }

    if !details.supported_text.is_empty() {
        content = content.push(meta_row("Languages", &details.supported_text.join(", ")));
    }

    if let Some(audio) = details
        .supported_audio
        .as_deref()
        .filter(|audio| !audio.is_empty())
    {
        content = content.push(meta_row("Audio", &audio.join(", ")));
    }

    let tags: Vec<_> = details
        .tags
        .iter()
        .map(|tag| tag.name.as_str())
        .filter(|name| !name.is_empty())
        .collect();
    if !tags.is_empty() {
        content = content.push(meta_row("Tags", &tags.join(", ")));
    }

    if let Some(reqs) = details
        .technical_requirements
        .windows
        .as_deref()
        .filter(|reqs| !reqs.is_empty())
    {
        content = content.push(requirements("Windows", reqs));
    }
    if let Some(reqs) = details
        .technical_requirements
        .macos
        .as_deref()
        .filter(|reqs| !reqs.is_empty())
    {
        content = content.push(requirements("macOS", reqs));
    }

    detail_scrollable(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example_description() -> String {
        let data: serde_json::Value =
            serde_json::from_slice(&std::fs::read("tests/getGameDetails.json").unwrap()).unwrap();
        data["data"]["Product"]["sandbox"]["configuration"]
            .as_array()
            .expect("configuration")
            .iter()
            .filter_map(|entry| entry["configs"]["longDescription"].as_str())
            .next()
            .expect("longDescription")
            .to_string()
    }

    #[test]
    fn unterminated_comment_is_kept() {
        assert_eq!(clean_description("Hello <!--oops"), "Hello <!--oops");
    }

    #[test]
    fn single_newlines_become_hard_breaks() {
        assert_eq!(clean_description("one\ntwo"), "one  \ntwo");
        // Blank lines still separate paragraphs.
        assert_eq!(clean_description("one\n\ntwo"), "one\n\ntwo");
    }

    #[test]
    fn hard_breaks_skip_fenced_code() {
        assert_eq!(
            clean_description("```\none\ntwo\n```"),
            "```\none\ntwo\n```"
        );
    }

    #[test]
    fn link_spacing_is_closed() {
        assert_eq!(
            clean_description("[label] (https://example.com)"),
            "[label](https://example.com)"
        );
        assert_eq!(
            clean_description("![alt] (https://example.com/img.png)"),
            "![alt](https://example.com/img.png)"
        );
        // Already-closed pairs are untouched.
        assert_eq!(
            clean_description("[label](https://example.com)"),
            "[label](https://example.com)"
        );
    }

    #[test]
    fn reference_style_links_become_inline() {
        // Epic sometimes emits reference-style targets instead of inline
        // ones; without a definition they render as literal text.
        assert_eq!(
            clean_description("[description][https://badmarkdown.com]"),
            "[description](https://badmarkdown.com)"
        );
        assert_eq!(
            clean_description("[label] [https://example.com]"),
            "[label](https://example.com)"
        );
        assert_eq!(
            clean_description("![alt] [https://example.com/img.png]"),
            "![alt](https://example.com/img.png)"
        );
        // Targets that aren't URLs leave real reference links alone.
        assert_eq!(clean_description("[note][1]"), "[note][1]");

        // The rewritten form parses as a clickable link.
        let dump = format!(
            "{:?}",
            parse_description("[description][https://badmarkdown.com]")
        );
        assert!(
            dump.contains(r#"link: Some("https://badmarkdown.com")"#),
            "expected a link span, got {dump}"
        );
    }

    #[test]
    fn emphasis_gap_is_closed() {
        // The case Epic actually emits: the space between the quote and the
        // closing marker must go, like `_text_` does without spaces.
        assert_eq!(
            clean_description("_\"More please!\" _"),
            "_\"More please!\"_"
        );
        // The space after the pair stays (it separates words).
        assert_eq!(clean_description("a _\"x!\" _ and b"), "a _\"x!\"_ and b");
        assert_eq!(clean_description("__\"No.\" __"), "__\"No.\"__");
        assert_eq!(clean_description("*\"Sarcasm.\" *"), "*\"Sarcasm.\"*");
        // Proper syntax and plain text keep rendering unchanged.
        assert_eq!(clean_description("_plain_"), "_plain_");
        assert_eq!(clean_description("**bold**"), "**bold**");
        assert_eq!(clean_description("snake_case"), "snake_case");
        assert_eq!(clean_description("a _ _ b"), "a _ _ b");
    }

    #[test]
    fn emphasis_gap_parses_as_italic() {
        let dump = format!("{:?}", parse_description("_\"More please!\" _"));
        assert!(
            dump.contains("emphasis: true"),
            "expected an italic span, got {dump}"
        );
    }

    #[test]
    fn bullets_become_markdown_list() {
        let items = parse_description("• first\n• second");
        let [markdown::Item::List { bullets, .. }] = items.as_slice() else {
            panic!("expected a single list, got {items:?}");
        };
        assert_eq!(bullets.len(), 2);
    }

    #[test]
    fn description_image_urls_are_collected() {
        let items = parse_description(
            "![decoration](https://cdn.example.com/icon.png)\n\n> ![nested](https://example.com/b.png)",
        );
        assert_eq!(
            description_image_urls(&items),
            vec![
                "https://cdn.example.com/icon.png".to_string(),
                "https://example.com/b.png".to_string(),
            ]
        );

        // No images means nothing to fetch.
        assert_eq!(
            description_image_urls(&parse_description("plain text")),
            Vec::<String>::new()
        );
    }

    #[test]
    fn example_description_parses_bullet_list() {
        let raw = example_description();
        assert!(raw.contains("•"));

        let items = parse_description(&raw);
        assert!(
            items
                .iter()
                .any(|item| matches!(item, markdown::Item::List { .. })),
            "feature bullets should parse as a list"
        );
    }

    #[test]
    fn asterisk_does_not_start_a_list() {
        assert_eq!(clean_description("* first"), "\\* first");
        assert_eq!(clean_description("  * indented"), "\\* indented");
        // No-space `*item` was already verbatim; it stays untouched.
        assert_eq!(clean_description("*item"), "*item");

        for raw in ["* first\n* second", "*first\n*second"] {
            let items = parse_description(raw);
            assert!(
                items
                    .iter()
                    .all(|item| !matches!(item, markdown::Item::List { .. })),
                "asterisk must stay verbatim for {raw:?}, got {items:?}"
            );
        }
        let dump = format!("{:?}", parse_description("* first\n* second"));
        assert!(dump.contains("* first"), "marker kept: {dump}");
        assert!(dump.contains("* second"), "marker kept: {dump}");
    }

    #[test]
    fn other_markers_do_not_start_lists() {
        assert_eq!(clean_description("- item"), "\\- item");
        assert_eq!(clean_description("+ item"), "\\+ item");
        assert_eq!(clean_description("1. first"), "1\\. first");
        assert_eq!(clean_description("2) second"), "2\\) second");

        for raw in ["- a\n- b", "+ a\n+ b", "1. a\n2. b"] {
            let items = parse_description(raw);
            assert!(
                items
                    .iter()
                    .all(|item| !matches!(item, markdown::Item::List { .. })),
                "only • starts a list for {raw:?}, got {items:?}"
            );
        }
    }

    #[test]
    fn double_star_and_emphasis_are_kept() {
        // Bold, rules and whole-line emphasis are not lists either.
        assert_eq!(clean_description("**bold**"), "**bold**");
        assert_eq!(clean_description("***"), "***");
        assert_eq!(clean_description("---"), "---");
        assert_eq!(clean_description("*emphasis*"), "*emphasis*");
        // `**bold**` still parses as strong text, not a list.
        let items = parse_description("**bold**");
        assert!(
            items
                .iter()
                .all(|item| !matches!(item, markdown::Item::List { .. })),
            "bold must not become a list: {items:?}"
        );
    }
}

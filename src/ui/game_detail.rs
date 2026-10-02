use iced::{
    Element, Length,
    widget::{column, container, image, markdown, row, scrollable, text},
};

use crate::{
    Message, State,
    epic::{GameDetails, TechRequirement},
    images::PixelData,
    ui::{TextWidgetExt, library::GRID_CONFIG},
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

/// Close the gap in `[label] (url)` / `![alt] (url)` pairs Epic sometimes
/// emits, which strict markdown does not parse as links or images.
fn fix_link_spacing(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(idx) = rest.find(']') {
        let after_bracket = &rest[idx + ']'.len_utf8()..];
        let spaces = after_bracket.len() - after_bracket.trim_start_matches([' ', '\t']).len();
        if spaces > 0 && after_bracket[spaces..].starts_with('(') {
            out.push_str(&rest[..=idx]);
            out.push('(');
            rest = &after_bracket[spaces + '('.len_utf8()..];
        } else {
            out.push_str(&rest[..=idx]);
            rest = after_bracket;
        }
    }
    out.push_str(rest);
    out
}

/// Normalize Epic's `longDescription` flavor into strict markdown:
/// comments become paragraphs, `•` bullets become lists, `[label] (url)`
/// gaps are closed, and single newlines become hard line breaks. Fenced code
/// blocks pass through untouched.
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
        if let Some(bullet) = fixed.trim_start().strip_prefix('•') {
            fixed = format!("- {}", bullet.trim_start());
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

/// Clean Epic's description and parse it for `markdown::view`. Called once
/// when details load; the items are stored in `State::detail_bodies` because
/// the view borrows them.
pub fn parse_description(raw: &str) -> Vec<markdown::Item> {
    markdown::parse(&clean_description(raw)).collect()
}

fn markdown_settings() -> markdown::Settings {
    markdown::Settings::with_text_size(
        crate::DEFAULT_TEXT_SIZE as f32,
        markdown::Style {
            font: crate::UI_FONT,
            ..markdown::Style::from_palette(crate::ui::theme::MAIN_PALETTE)
        },
    )
}

fn requirements<'a>(platform: &str, reqs: &[TechRequirement]) -> Element<'a, Message> {
    let mut body = column![text!("System requirements ({platform})").bold()].spacing(4);
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
            .spacing(2),
        );
    }
    body.spacing(8).into()
}

pub fn view(state: &State) -> Element<'_, Message> {
    let Some(items) = &state.catalog_items else {
        return centered("Loading...");
    };
    let Some(item) = state.focused_game_idx.and_then(|index| items.get(index)) else {
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
        .spacing(4)
        .width(Length::Fill);
    if !offer.developer_display_name.is_empty() {
        identity = identity.push(meta_row("Developer", &offer.developer_display_name));
    }
    if !offer.publisher_display_name.is_empty() {
        identity = identity.push(meta_row("Publisher", &offer.publisher_display_name));
    }
    if let Some(critic) = item.critic.as_ref() {
        identity = identity.push(meta_row(
            "OpenCritic",
            &format!(
                "{} {} ({}% recommended)",
                critic.average,
                critic.rating(),
                critic.recommend_percentage
            ),
        ));
        if !critic.url.is_empty() {
            identity = identity.push(text!("{}", critic.url).size(12));
        }
    }

    let mut content = column![row![cover, identity].spacing(16)].spacing(12);

    if let Some(description) = state.detail_bodies.get(&item.namespace) {
        content = content
            .push(markdown::view(description, markdown_settings()).map(|_| Message::Ignored));
    } else {
        content = content.push(text!("{}", full.description()));
    }

    let mut meta = column![].spacing(4);
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
        return container(scrollable(content).height(Length::Fill).width(Length::Fill))
            .padding(16)
            .into();
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

    container(scrollable(content).height(Length::Fill).width(Length::Fill))
        .padding(16)
        .into()
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
    fn bullets_become_markdown_list() {
        let items = parse_description("• first\n• second");
        let [markdown::Item::List { bullets, .. }] = items.as_slice() else {
            panic!("expected a single list, got {items:?}");
        };
        assert_eq!(bullets.len(), 2);
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
}

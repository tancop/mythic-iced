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

/// Strip the `<!-- ... -->` HTML comments Epic embeds in `longDescription`,
/// which would otherwise show up as raw text in the rendered markdown.
pub fn clean_description(raw: &str) -> String {
    let mut stripped = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find("<!--") {
        stripped.push_str(&rest[..start]);
        let after = &rest[start + "<!--".len()..];
        match after.find("-->") {
            Some(end) => rest = &after[end + "-->".len()..],
            // Unterminated comment: keep it verbatim rather than dropping content.
            None => {
                stripped.push_str("<!--");
                rest = after;
                break;
            }
        }
    }
    stripped.push_str(rest);

    // Epic uses `•` bullets, which are not markdown lists; normalize them so
    // they render as one.
    let mut normalized = String::with_capacity(stripped.len());
    for line in stripped.lines() {
        if let Some(bullet) = line.trim_start().strip_prefix('•') {
            normalized.push_str("- ");
            normalized.push_str(bullet.trim_start());
        } else {
            normalized.push_str(line);
        }
        normalized.push('\n');
    }
    normalized.trim().to_string()
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
            serde_json::from_slice(&std::fs::read("getCatalogOffer.json").unwrap()).unwrap();
        data["data"]["Catalog"]["catalogOffer"]["longDescription"]
            .as_str()
            .expect("longDescription")
            .to_string()
    }

    #[test]
    fn strips_html_comments() {
        assert_eq!(
            clean_description("<!--textBlock-->\n<!--text-->\nHello\n\nWorld"),
            "Hello\n\nWorld"
        );
    }

    #[test]
    fn unterminated_comment_is_kept() {
        assert_eq!(clean_description("Hello <!--oops"), "Hello <!--oops");
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
    fn example_description_parses_without_raw_comments() {
        let raw = example_description();
        assert!(raw.contains("<!--"));
        let cleaned = clean_description(&raw);
        assert!(!cleaned.contains("<!--"));

        let items = parse_description(&raw);
        assert!(
            items
                .iter()
                .any(|item| matches!(item, markdown::Item::List { .. })),
            "feature bullets should parse as a list"
        );
    }
}

use iced::{
    Element, Length,
    widget::{column, container, image, row, scrollable, text},
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

    let mut content = column![
        row![
            cover,
            column![
                text!("{}", title).bold().size(24),
                text!("{}", full.description())
            ]
            .spacing(8)
            .width(Length::Fill),
        ]
        .spacing(16),
    ]
    .spacing(12);

    let mut meta = column![].spacing(4);
    if !offer.developer_display_name.is_empty() {
        meta = meta.push(meta_row("Developer", &offer.developer_display_name));
    }
    if !offer.publisher_display_name.is_empty() {
        meta = meta.push(meta_row("Publisher", &offer.publisher_display_name));
    }
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

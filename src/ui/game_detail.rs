use iced::{
    Element, Length,
    widget::{column, container, image, row, scrollable, text},
};

use crate::{
    Message, State,
    epic::TechRequirement,
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
    let Some(details) = state.game_details.get(&item.namespace) else {
        return centered(if state.details_error {
            "Couldn't load game details."
        } else {
            "Loading..."
        });
    };

    let title = match details.product_display_name.as_deref() {
        Some(name) => name.into(),
        None => item.title.clone(),
    };

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

    // Products without a store page only have a name and artwork.
    if !details.has_store_page() {
        return container(
            scrollable(
                row![
                    cover,
                    column![
                        text!("{}", title).bold().size(24).width(Length::Fill),
                        text!("No information available").width(Length::Fill),
                    ]
                ]
                .spacing(16),
            )
            .height(Length::Fill)
            .width(Length::Fill),
        )
        .padding(16)
        .into();
    }

    let mut content = column![
        row![
            cover,
            column![
                text!("{}", title).bold().size(24),
                text!("{}", details.description())
            ]
            .spacing(8)
            .width(Length::Fill),
        ]
        .spacing(16),
    ]
    .spacing(12);

    let mut meta = column![].spacing(4);
    if !details.developer_display_name.is_empty() {
        meta = meta.push(meta_row("Developer", &details.developer_display_name));
    }
    if !details.publisher_display_name.is_empty() {
        meta = meta.push(meta_row("Publisher", &details.publisher_display_name));
    }
    if let Some(date) = details.release_date() {
        meta = meta.push(meta_row("Release date", date));
    }
    if let Some(website) = details.game_website.as_deref().filter(|s| !s.is_empty()) {
        meta = meta.push(meta_row("Website", website));
    }
    content = content.push(meta);

    let languages = details.supported_text.as_deref().unwrap_or_default();
    if !languages.is_empty() {
        content = content.push(meta_row("Languages", &languages.join(", ")));
    }

    let tags: Vec<_> = details
        .tags
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|tag| tag.name.as_str())
        .filter(|name| !name.is_empty())
        .collect();
    if !tags.is_empty() {
        content = content.push(meta_row("Tags", &tags.join(", ")));
    }

    if let Some(reqs) = details
        .technical_requirements
        .as_ref()
        .and_then(|reqs| reqs.windows.as_deref())
        .filter(|reqs| !reqs.is_empty())
    {
        content = content.push(requirements("Windows", reqs));
    }
    if let Some(reqs) = details
        .technical_requirements
        .as_ref()
        .and_then(|reqs| reqs.macos.as_deref())
        .filter(|reqs| !reqs.is_empty())
    {
        content = content.push(requirements("macOS", reqs));
    }

    if let Some(legal) = details.legal_text.as_deref().filter(|s| !s.is_empty()) {
        content = content.push(text!("{}", legal).size(12));
    }

    container(scrollable(content).height(Length::Fill).width(Length::Fill))
        .padding(16)
        .into()
}

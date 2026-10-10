use anyhow::bail;
use isahc::{AsyncReadResponseExt, Request};
use serde::Deserialize;

use super::{
    CatalogOffer, ContentType, EmptyGameDetails, FullGameDetails, GameDetails, GameFullDetails,
    GqlError, HomeDetails, NamespaceInfo, NamespaceMapping, RequestBuilderExt, STORE_GQL_URL,
    STORE_USER_AGENT, StoreDetails,
};

const DETAILS_QUERY: &str = include_str!("../graphql/getGameDetails.gql");
const NAMESPACE_QUERY: &str = include_str!("../graphql/getCatalogNamespace.gql");
const OFFER_QUERY: &str = include_str!("../graphql/getCatalogOffer.gql");

async fn post_store_gql(
    client: &isahc::HttpClient,
    auth_token: &str,
    query: &str,
    variables: serde_json::Value,
) -> anyhow::Result<Vec<u8>> {
    let body = serde_json::to_vec(&serde_json::json!({
        "query": query,
        "variables": variables,
    }))?;
    let req = Request::post(STORE_GQL_URL)
        .bearer_auth(auth_token)
        .user_agent(STORE_USER_AGENT)
        .content_type(ContentType::Json)
        .body(body)
        .unwrap();
    let mut res = client.send_async(req).await?;
    Ok(res.bytes().await?)
}

fn check_gql_errors(errors: &Option<Vec<GqlError>>, context: &str) -> anyhow::Result<()> {
    if let Some(errors) = errors
        && !errors.is_empty()
    {
        let msgs: Vec<_> = errors.iter().map(|e| e.message.as_str()).collect();
        bail!("{context} GraphQL query failed: {}", msgs.join("; "));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct DetailsGqlResponse {
    data: Option<DetailsData>,
    #[serde(default)]
    errors: Option<Vec<GqlError>>,
}

#[derive(Debug, Deserialize)]
struct DetailsData {
    #[serde(rename = "Product")]
    product: DetailsProduct,
}

#[derive(Debug, Deserialize)]
struct DetailsProduct {
    sandbox: DetailsSandbox,
}

#[derive(Debug, Deserialize)]
struct DetailsSandbox {
    // Entries for other configuration fragments come back as `{}`.
    #[serde(default)]
    configuration: Vec<DetailsConfigEntry>,
}

#[derive(Debug, Deserialize)]
struct DetailsConfigEntry {
    #[serde(default)]
    configs: Option<StoreConfigs>,
}

/// One `configuration` entry: either the Home blurb or the Store
/// configuration. Tried in order — a Home entry would otherwise match
/// `GameDetails::Empty`, which accepts any object.
#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(untagged)]
enum StoreConfigs {
    Home(HomeDetails),
    Store(GameDetails),
}

/// Split the `configuration` fragments into the Home long description and
/// the Store configuration. Absent fragments yield an empty blurb / `Empty`.
fn split_configurations(entries: Vec<DetailsConfigEntry>) -> StoreDetails {
    let mut long_description = String::new();
    let mut details = None;
    for entry in entries {
        match entry.configs {
            Some(StoreConfigs::Home(home)) if long_description.is_empty() => {
                long_description = home.long_description;
            }
            Some(StoreConfigs::Store(store)) if details.is_none() => {
                details = Some(store);
            }
            _ => {}
        }
    }
    StoreDetails {
        long_description,
        details: details.unwrap_or(GameDetails::Empty(EmptyGameDetails {})),
    }
}

impl GameDetails {
    pub fn has_store_page(&self) -> bool {
        matches!(self, GameDetails::Full(_))
    }

    pub fn full(&self) -> Option<&FullGameDetails> {
        match self {
            GameDetails::Full(full) => Some(full),
            GameDetails::Empty(_) => None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct OfferGqlResponse {
    data: Option<OfferData>,
    #[serde(default)]
    errors: Option<Vec<GqlError>>,
}

#[derive(Debug, Deserialize)]
struct OfferData {
    #[serde(rename = "Catalog")]
    catalog: OfferCatalog,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OfferCatalog {
    catalog_offer: Option<CatalogOffer>,
}

impl CatalogOffer {
    /// Release date as `YYYY-MM-DD` when the value carries a time part.
    pub fn release_date_trimmed(&self) -> Option<&str> {
        self.pc_release_date
            .as_deref()
            .or(self.release_date.as_deref())
            .map(|date| date.get(..10).unwrap_or(date))
    }
}

#[derive(Debug, Deserialize)]
struct NamespaceGqlResponse {
    data: Option<NamespaceData>,
    #[serde(default)]
    errors: Option<Vec<GqlError>>,
}

#[derive(Debug, Deserialize)]
struct NamespaceData {
    #[serde(rename = "Catalog")]
    catalog: NamespaceCatalog,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NamespaceCatalog {
    #[serde(default)]
    catalog_ns: Option<NamespaceInfo>,
}

impl GameFullDetails {
    pub fn has_store_page(&self) -> bool {
        self.details.has_store_page()
    }

    pub fn title(&self, fallback: &str) -> String {
        if self.offer.title.is_empty() {
            fallback.to_string()
        } else {
            self.offer.title.clone()
        }
    }

    /// Long blurb from `HomeConfiguration` when present, else the offer's
    /// short description (the offer `longDescription` is unreliable).
    pub fn description(&self) -> &str {
        if self.long_description.is_empty() {
            &self.offer.description
        } else {
            &self.long_description
        }
    }

    pub fn release_date(&self) -> Option<&str> {
        self.offer.release_date_trimmed()
    }
}

/// Find the main game's offer id: the `productHome` entry's offer. Falls back
/// to the first live entry when no `productHome` exists.
pub fn find_main_offer_id(mappings: &[NamespaceMapping]) -> Option<String> {
    mappings
        .iter()
        .find(|m| {
            m.page_type == "productHome"
                && m.deleted_date.is_none()
                && !m.mappings.offer_id.is_empty()
        })
        .or_else(|| {
            mappings
                .iter()
                .find(|m| m.deleted_date.is_none() && !m.mappings.offer_id.is_empty())
        })
        .map(|m| m.mappings.offer_id.clone())
}

/// Fetch the namespace mappings for one product (`sandbox_id` is the catalog
/// `namespace`). Same store GraphQL endpoint/auth as the library fetch.
pub async fn get_catalog_namespace(
    client: &isahc::HttpClient,
    auth_token: &str,
    sandbox_id: &str,
) -> anyhow::Result<NamespaceInfo> {
    let bytes = post_store_gql(
        client,
        auth_token,
        NAMESPACE_QUERY,
        serde_json::json!({ "sandboxId": sandbox_id }),
    )
    .await?;

    log::debug!(
        "catalog namespace gql response: {:?}",
        str::from_utf8(&bytes).unwrap_or("<non-utf8>")
    );

    let trimmed = super::trim_trailing_whitespace(&bytes);
    let gql = serde_json::from_slice::<NamespaceGqlResponse>(trimmed).map_err(|e| {
        anyhow::anyhow!(
            "failed to parse catalog namespace response ({} bytes): {e}",
            bytes.len()
        )
    })?;
    check_gql_errors(&gql.errors, "catalog namespace")?;
    let Some(data) = gql.data else {
        bail!("catalog namespace GraphQL response missing data");
    };
    data.catalog
        .catalog_ns
        .ok_or_else(|| anyhow::anyhow!("catalog namespace response has no catalogNs"))
}

/// Fetch one offer by namespace + offer id. `locale` is always `en` for now.
pub async fn get_catalog_offer(
    client: &isahc::HttpClient,
    auth_token: &str,
    sandbox_id: &str,
    offer_id: &str,
) -> anyhow::Result<CatalogOffer> {
    let bytes = post_store_gql(
        client,
        auth_token,
        OFFER_QUERY,
        serde_json::json!({
            "sandboxId": sandbox_id,
            "offerId": offer_id,
            "locale": "en",
        }),
    )
    .await?;

    log::debug!(
        "catalog offer gql response: {:?}",
        str::from_utf8(&bytes).unwrap_or("<non-utf8>")
    );

    let trimmed = super::trim_trailing_whitespace(&bytes);
    let gql = serde_json::from_slice::<OfferGqlResponse>(trimmed).map_err(|e| {
        anyhow::anyhow!(
            "failed to parse catalog offer response ({} bytes): {e}",
            bytes.len()
        )
    })?;
    check_gql_errors(&gql.errors, "catalog offer")?;
    let Some(data) = gql.data else {
        bail!("catalog offer GraphQL response missing data");
    };
    data.catalog
        .catalog_offer
        .ok_or_else(|| anyhow::anyhow!("catalog offer response has no catalogOffer"))
}

/// Fetch one product's store info: the Home blurb plus the Store
/// configuration. `sandbox_id` is the catalog `namespace`; `locale` is always
/// `en` for now and `templateId` is left unset. Same store GraphQL
/// endpoint/auth as the library fetch.
pub async fn get_store_details(
    client: &isahc::HttpClient,
    auth_token: &str,
    sandbox_id: &str,
) -> anyhow::Result<StoreDetails> {
    let bytes = post_store_gql(
        client,
        auth_token,
        DETAILS_QUERY,
        serde_json::json!({
            "locale": "en",
            "sandboxId": sandbox_id,
        }),
    )
    .await?;

    log::debug!(
        "game details gql response: {:?}",
        str::from_utf8(&bytes).unwrap_or("<non-utf8>")
    );

    let trimmed = super::trim_trailing_whitespace(&bytes);
    let gql = serde_json::from_slice::<DetailsGqlResponse>(trimmed).map_err(|e| {
        anyhow::anyhow!(
            "failed to parse game details response ({} bytes): {e}",
            bytes.len()
        )
    })?;
    check_gql_errors(&gql.errors, "game details")?;
    let Some(data) = gql.data else {
        bail!("game details GraphQL response missing data");
    };

    Ok(split_configurations(data.product.sandbox.configuration))
}

/// Fetch full store info for one product: resolve the main game's offer id
/// via `getCatalogNamespace`, then fetch `getGameDetails` and
/// `getCatalogOffer` in parallel.
pub async fn get_game_details(
    client: &isahc::HttpClient,
    auth_token: &str,
    sandbox_id: &str,
) -> anyhow::Result<GameFullDetails> {
    log::debug!("fetching game details for sandbox_id: {}", sandbox_id);

    let namespace = get_catalog_namespace(client, auth_token, sandbox_id).await?;
    let offer_id = find_main_offer_id(&namespace.mappings)
        .ok_or_else(|| anyhow::anyhow!("catalog namespace has no offer mappings"))?;

    let (details_result, offer_result) = iced::futures::future::join(
        get_store_details(client, auth_token, sandbox_id),
        get_catalog_offer(client, auth_token, sandbox_id, &offer_id),
    )
    .await;

    let store = details_result?;
    Ok(GameFullDetails {
        offer_id,
        offer: offer_result?,
        details: store.details,
        long_description: store.long_description,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_details_deserialize_from_example() {
        let bytes = std::fs::read("tests/getGameDetails.json").unwrap();
        let resp: DetailsGqlResponse = serde_json::from_slice(&bytes).unwrap();
        let data = resp.data.expect("data");
        // Home blurb plus Store configuration; the other fragments are `{}`.
        assert_eq!(data.product.sandbox.configuration.len(), 4);
        let store = split_configurations(data.product.sandbox.configuration);

        assert!(store.long_description.contains("ultimate off-road"));
        assert!(store.long_description.contains("coop multiplayer"));

        let GameDetails::Full(details) = &store.details else {
            panic!("expected full game details");
        };

        assert!(details.critic_reviews.open_critic);
        assert!(details.supported_text.contains(&"English".to_string()));
        assert!(details.tags.iter().any(|tag| tag.name == "Single Player"));
        assert!(details.tags.iter().any(|tag| tag.name == "Cloud Saves"));

        let windows = details
            .technical_requirements
            .windows
            .as_deref()
            .unwrap_or_default();
        let storage = windows
            .iter()
            .find(|req| req.title == "Storage")
            .expect("storage row");
        assert!(
            storage
                .minimum
                .as_deref()
                .is_some_and(|v| v.contains("1 GB available space"))
        );
        assert!(details.technical_requirements.macos.is_none());
    }

    #[test]
    fn product_without_store_page_parses_as_empty() {
        // Legacy GTA-style: nulls where `FullGameDetails` requires values,
        // plus a null Home blurb.
        let bytes = serde_json::json!({
            "data": {
                "Product": {
                    "sandbox": {
                        "configuration": [{
                            "configs": {
                                "criticReviews": null,
                                "externalPlatformLaunchOptions": null,
                                "gameWebsite": null,
                                "privacyLink": null,
                                "socialLinks": null,
                                "supportedAudio": null,
                                "supportedText": null,
                                "tags": [],
                                "technicalRequirements": null
                            }
                        }, {
                            "configs": {
                                "longDescription": null
                            }
                        }]
                    }
                }
            }
        });
        let resp: DetailsGqlResponse =
            serde_json::from_value(bytes).expect("empty-style response parses");
        let store = split_configurations(resp.data.expect("data").product.sandbox.configuration);
        assert_eq!(store.long_description, "");
        assert!(!store.details.has_store_page());
        assert!(matches!(store.details, GameDetails::Empty(_)));
    }

    #[test]
    fn namespace_resolves_main_game_offer() {
        let bytes = std::fs::read("tests/getCatalogNamespace.json").unwrap();
        let resp: NamespaceGqlResponse = serde_json::from_slice(&bytes).unwrap();
        let info = resp
            .data
            .expect("data")
            .catalog
            .catalog_ns
            .expect("catalogNs");
        assert_eq!(info.store, "EGS");
        assert_eq!(info.mappings.len(), 6);
        // `productHome` entry carries the main game's offer id.
        assert_eq!(
            find_main_offer_id(&info.mappings),
            Some("7aea960be7dd4d86a9b30cf5daa03eeb".to_string())
        );
    }

    #[test]
    fn catalog_offer_deserializes_from_example() {
        let bytes = std::fs::read("tests/getCatalogOffer.json").unwrap();
        let resp: OfferGqlResponse = serde_json::from_slice(&bytes).unwrap();
        let offer = resp
            .data
            .expect("data")
            .catalog
            .catalog_offer
            .expect("catalogOffer");

        assert_eq!(offer.title, "MudRunner");
        assert_eq!(offer.developer_display_name, "Saber Interactive");
        assert_eq!(offer.publisher_display_name, "Focus Entertainment");
        assert_eq!(offer.offer_type, "BASE_GAME");
        assert!(offer.description.contains("ultimate off-road"));
        assert_eq!(offer.release_date_trimmed(), Some("2020-11-26"));
        assert!(offer.tags.iter().any(|tag| tag.name == "Single Player"));
        assert!(offer.categories.contains(&"games".to_string()));
        let seller = offer.seller.expect("seller");
        assert_eq!(seller.name, "Focus Entertainment Publishing");
    }

    #[test]
    fn full_details_prefers_home_blurb() {
        let offer = CatalogOffer {
            description: "short".to_string(),
            ..CatalogOffer::default()
        };
        let full = GameFullDetails {
            offer_id: "id".to_string(),
            offer,
            details: GameDetails::Empty(EmptyGameDetails {}),
            long_description: "long".to_string(),
        };
        assert_eq!(full.description(), "long");

        let fallback = GameFullDetails {
            long_description: String::new(),
            ..full
        };
        assert_eq!(fallback.description(), "short");
    }
}

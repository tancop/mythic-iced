mod auth;
mod library;
mod manifest;
mod reviews;
mod store;

use std::str::FromStr;

pub use auth::{authenticate, get_auth_url, refresh_token};
pub use library::get_library_catalog;
pub use reviews::{CriticError, get_critic_reviews};
pub use store::get_game_details;

use const_format::formatcp;
use isahc::{
    auth::{Authentication, Credentials},
    config::Configurable,
    http,
};
use serde::{Deserialize, Serialize};

const USER_AGENT: &'static str =
    "UELauncher/11.0.1-14907503+++Portal+Release-Live Windows/10.0.19041.1.256.64bit";
const STORE_USER_AGENT: &'static str = "EpicGamesLauncher/14.0.8-22004686+++Portal+Release-Live";

const STORE_GQL_HOST: &'static str = "launcher.store.epicgames.com";
const ARTIFACT_SERVICE_HOST: &'static str =
    "artifact-public-service-prod.beee.live.use1a.on.epicgames.com";

const STORE_GQL_URL: &'static str = formatcp!("https://{}/graphql", STORE_GQL_HOST);

enum ContentType {
    Json,
    FormData,
    Text,
}

impl ContentType {
    fn to_string(&self) -> &'static str {
        match self {
            ContentType::Json => "application/json",
            ContentType::FormData => "application/x-www-form-urlencoded",
            ContentType::Text => "text/plain",
        }
    }
}

trait RequestBuilderExt {
    fn bearer_auth(self, token: &str) -> Self;
    fn basic_auth(self, username: &str, password: &str) -> Self;
    fn user_agent(self, user_agent: &str) -> Self;
    fn content_type(self, content_type: ContentType) -> Self;
}

impl RequestBuilderExt for http::request::Builder {
    fn bearer_auth(self, token: &str) -> Self {
        self.header("Authorization", &format!("Bearer {}", token))
    }

    fn basic_auth(self, username: &str, password: &str) -> Self {
        self.authentication(Authentication::basic())
            .credentials(Credentials::new(username, password))
    }

    fn user_agent(self, user_agent: &str) -> Self {
        self.header("User-Agent", user_agent)
    }

    fn content_type(self, content_type: ContentType) -> Self {
        self.header("Content-Type", content_type.to_string())
    }
}

fn trim_trailing_whitespace(bytes: &[u8]) -> &[u8] {
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &bytes[..end]
}

#[derive(Debug, Deserialize)]
struct GqlError {
    message: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct AuthData {
    pub access_token: String,
    pub refresh_token: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
}

pub struct LibraryCatalog {
    pub items: Vec<CatalogItem>,
    pub purchase_dates: std::collections::HashMap<String, UtcDateTime>,
}

/// `HomeConfiguration` configs: carries the long blurb. `longDescription`
/// may be explicit null, but the key is always selected — so no `default`
/// here: a missing key means this is a Store entry and must fall through to
/// the next variant.
#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HomeDetails {
    #[serde(deserialize_with = "crate::decode::deserialize_null_default")]
    pub long_description: String,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CriticReviews {
    pub open_critic: bool,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct XCloudLink {
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub link_src: String,
    #[serde(
        rename = "type",
        default,
        deserialize_with = "crate::decode::deserialize_null_default"
    )]
    pub link_type: String,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExternalPlatformLaunchOptions {
    #[serde(default)]
    pub x_cloud: Option<XCloudLink>,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SocialLink {
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub platform: String,
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub url: String,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StoreTag {
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub id: String,
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub name: String,
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub group_name: String,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TechRequirement {
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub title: String,
    #[serde(default)]
    pub minimum: Option<String>,
    #[serde(default)]
    pub recommended: Option<String>,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TechRequirements {
    #[serde(default)]
    pub windows: Option<Vec<TechRequirement>>,
    #[serde(default)]
    pub macos: Option<Vec<TechRequirement>>,
}

/// Full store configuration: `supportedText`, `tags` and
/// `technicalRequirements` are always present, and
/// `criticReviews.openCritic` is always true/false, never null.
#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FullGameDetails {
    pub critic_reviews: CriticReviews,
    #[serde(default)]
    pub external_platform_launch_options: Option<ExternalPlatformLaunchOptions>,
    #[serde(default)]
    pub game_website: Option<String>,
    #[serde(default)]
    pub privacy_link: Option<String>,
    #[serde(default)]
    pub social_links: Option<Vec<SocialLink>>,
    #[serde(default)]
    pub supported_audio: Option<Vec<String>>,
    pub supported_text: Vec<String>,
    pub tags: Vec<StoreTag>,
    pub technical_requirements: TechRequirements,
}

/// Products without a store page (e.g. legacy GTA 5) come back with nulls
/// where `FullGameDetails` requires values. Matches any object as a
/// fallback; `GameDetails` tries `Full` first so real pages never land here.
#[derive(Debug, Deserialize, Clone, PartialEq, Default)]
pub struct EmptyGameDetails {}

/// Store configuration with two shapes: full info, or empty (no store page).
#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum GameDetails {
    Full(Box<FullGameDetails>),
    Empty(EmptyGameDetails),
}

#[derive(Debug, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExternalLink {
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub text: String,
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub url: String,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Seller {
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub id: String,
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub name: String,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct OfferItem {
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub id: String,
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub namespace: String,
    #[serde(default)]
    pub release_info: Option<ReleaseInfo>,
}

/// Catalog offer for the main game entry: identity, store text and seller.
/// Fetched via `getCatalogOffer`; complements `GameDetails` without overlap.
#[derive(Debug, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct CatalogOffer {
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub title: String,
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub developer_display_name: String,
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub description: String,
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub offer_type: String,
    #[serde(default)]
    pub external_links: Option<Vec<ExternalLink>>,
    #[serde(default)]
    pub seller: Option<Seller>,
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub publisher_display_name: String,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub tags: Vec<StoreTag>,
    #[serde(default)]
    pub items: Vec<OfferItem>,
    #[serde(deserialize_with = "crate::decode::deserialize_path_list", default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub pc_release_date: Option<String>,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct OfferIdMapping {
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub offer_id: String,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct NamespaceMapping {
    #[serde(default)]
    pub deleted_date: Option<String>,
    #[serde(default)]
    pub mappings: OfferIdMapping,
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub page_slug: String,
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub page_type: String,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct NamespaceInfo {
    #[serde(default)]
    pub mappings: Vec<NamespaceMapping>,
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub store: String,
}

/// Store info from `getGameDetails`: the Home blurb plus the Store
/// configuration (languages, tags, requirements).
#[derive(Debug, Clone, PartialEq)]
pub struct StoreDetails {
    pub long_description: String,
    pub details: GameDetails,
}

/// Combined store info for one product: offer identity/short text plus the
/// Home blurb and store configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct GameFullDetails {
    pub offer_id: String,
    pub offer: CatalogOffer,
    pub details: GameDetails,
    pub long_description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub enum CriticRating {
    Weak,
    Fair,
    Strong,
    Mighty,
}

impl FromStr for CriticRating {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Weak" => Ok(Self::Weak),
            "Fair" => Ok(Self::Fair),
            "Strong" => Ok(Self::Strong),
            "Mighty" => Ok(Self::Mighty),
            _ => Err(()),
        }
    }
}

/// OpenCritic score kept on each library item: average score, recommend
/// percentage and the OpenCritic page URL. All other API fields are ignored.
#[derive(Debug, Clone, PartialEq)]
pub struct CriticScore {
    pub average: i32,
    pub rating: CriticRating,
    pub recommend_percentage: i32,
    pub url: String,
}

pub type UtcDateTime = chrono::DateTime<chrono::Utc>;

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KeyImage {
    #[serde(rename = "type")]
    pub image_type: String,
    pub url: String,
    #[serde(default)]
    pub md5: String,
    #[serde(default)]
    pub width: u16,
    #[serde(default)]
    pub height: u16,
    #[serde(default)]
    pub size: u32,
    #[serde(
        default,
        deserialize_with = "crate::decode::deserialize_optional_datetime"
    )]
    pub uploaded_date: Option<UtcDateTime>,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DlcRef {
    pub id: String,
    #[serde(default)]
    pub namespace: String,
    #[serde(default)]
    pub unsearchable: bool,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct MainGameRef {
    pub id: String,
    #[serde(default)]
    pub namespace: String,
    #[serde(default)]
    pub unsearchable: bool,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseInfo {
    #[serde(default)]
    pub app_id: String,
    #[serde(default)]
    pub platform: Vec<String>,
    #[serde(default)]
    pub compatible_apps: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogItem {
    pub id: String,
    #[serde(default)]
    pub namespace: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub key_images: Vec<KeyImage>,
    #[serde(deserialize_with = "crate::decode::deserialize_path_list", default)]
    pub categories: Vec<String>,

    #[serde(
        default = "crate::decode::default_epoch",
        deserialize_with = "crate::decode::deserialize_datetime"
    )]
    pub creation_date: UtcDateTime,
    #[serde(
        default = "crate::decode::default_epoch",
        deserialize_with = "crate::decode::deserialize_datetime"
    )]
    pub last_modified_date: UtcDateTime,

    #[serde(default)]
    pub developer: String,

    // None when the item is a DLC
    #[serde(default)]
    pub dlc_item_list: Option<Vec<DlcRef>>,

    // Some for DLC (points at the parent game), None for games.
    // Categories alone can't be trusted: some DLC are mislabeled `games`.
    #[serde(default)]
    pub main_game_item: Option<MainGameRef>,

    // Present in the GraphQL library response; used for platform filtering.
    #[serde(default)]
    pub release_info: Vec<ReleaseInfo>,

    // Record-level `productId` (pricing + critic reviews). Not inside
    // `catalogItem` itself; filled in during merge.
    #[serde(skip, default)]
    pub product_id: Option<String>,

    // OpenCritic score fetched after library load; None when unavailable.
    #[serde(skip, default)]
    pub critic: Option<CriticScore>,

    // Precomputed by the loading code via `search::build_search_key`:
    // lowercased title with noise words and punctuation stripped. Never
    // serialized; search compares against this instead of `title`.
    #[serde(default)]
    pub search_key: String,
}

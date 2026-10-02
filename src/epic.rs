use anyhow::bail;
use const_format::formatcp;
use isahc::{
    AsyncReadResponseExt, Request,
    auth::{Authentication, Credentials},
    config::Configurable,
    http,
};
use serde::{Deserialize, Serialize};
use std::io::Write;

const USER_AGENT: &'static str =
    "UELauncher/11.0.1-14907503+++Portal+Release-Live Windows/10.0.19041.1.256.64bit";
const STORE_USER_AGENT: &'static str = "EpicGamesLauncher/14.0.8-22004686+++Portal+Release-Live";
// required for the oauth request;
const USER_BASIC: &'static str = "34a02cf8f4414e29b15921876da36f9a";
const PW_BASIC: &'static str = "daafbccc737745039dffe53d94fc76cf";
const LABEL: &'static str = "Live-EternalKnight";

const OAUTH_HOST: &'static str = "account-public-service-prod03.ol.epicgames.com";
const LAUNCHER_HOST: &'static str = "launcher-public-service-prod06.ol.epicgames.com";

const STORE_GQL_HOST: &'static str = "launcher.store.epicgames.com";
const ARTIFACT_SERVICE_HOST: &'static str =
    "artifact-public-service-prod.beee.live.use1a.on.epicgames.com";

// Encoding of "https://www.epicgames.com/id/api/redirect?clientId=34a02cf8f4414e29b15921876da36f9a&responseType=code"
const ENCODED_REDIRECT_URL: &'static str = "https%3A%2F%2Fwww.epicgames.com%2Fid%2Fapi%2Fredirect%3F\
    clientId%3D34a02cf8f4414e29b15921876da36f9a%26responseType%3Dcode";

const AUTH_URL: &'static str = formatcp!(
    "https://www.epicgames.com/id/login?redirectUrl={}",
    ENCODED_REDIRECT_URL
);

const TOKEN_URL: &'static str = formatcp!("https://{}/account/api/oauth/token", OAUTH_HOST);

pub fn get_auth_url() -> &'static str {
    AUTH_URL
}

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

#[derive(Serialize)]
struct TokenRequest<'a> {
    grant_type: &'a str,
    code: &'a str,
    token_type: &'a str,
}

#[derive(Deserialize, Debug)]
struct AuthError {
    #[serde(rename = "errorCode")]
    error_code: String,
    #[serde(rename = "errorMessage")]
    error_message: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct AuthData {
    pub access_token: String,
    pub refresh_token: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
}

pub async fn authenticate(client: &isahc::HttpClient, auth_code: &str) -> anyhow::Result<AuthData> {
    let req = Request::post(TOKEN_URL)
        .content_type(ContentType::FormData)
        .user_agent(USER_AGENT)
        .basic_auth(USER_BASIC, PW_BASIC)
        .body(
            serde_url_params::to_string(&TokenRequest {
                grant_type: &"authorization_code",
                code: auth_code,
                token_type: &"eg1",
            })
            .unwrap(),
        )
        .unwrap();
    let mut res = client.send_async(req).await?;

    log::debug!("response: {:?}", res);
    let bytes = res.bytes().await?;
    log::debug!("response data: {:?}", bytes);

    let data = match serde_json::from_slice::<AuthData>(&bytes) {
        Ok(data) => data,
        Err(e) => match e.classify() {
            serde_json::error::Category::Data => {
                // valid JSON but no data fields, API returned error
                let err: AuthError = serde_json::from_slice(&bytes).unwrap();
                bail!(
                    "authentication failed with error {}: {}",
                    err.error_code,
                    err.error_message
                );
            }
            _ => bail!("failed to parse response: {}", e),
        },
    };
    Ok(data)
}

const REFRESH_URL: &'static str = formatcp!("https://{}/account/api/oauth/verify", OAUTH_HOST);

#[derive(Serialize)]
struct RefreshRequest<'a> {
    grant_type: &'a str,
    refresh_token: &'a str,
    #[serde(rename = "include_perms")]
    include_perms: &'a str,
    token_type: &'a str,
}

pub async fn refresh_token(
    client: &isahc::HttpClient,
    refresh_token: &str,
) -> anyhow::Result<AuthData> {
    let req = Request::post(TOKEN_URL)
        .content_type(ContentType::FormData)
        .user_agent(USER_AGENT)
        .basic_auth(USER_BASIC, PW_BASIC)
        .body(
            serde_url_params::to_string(&RefreshRequest {
                grant_type: "refresh_token",
                refresh_token,
                include_perms: "false",
                token_type: "eg1",
            })
            .unwrap(),
        )
        .unwrap();
    let mut res = client.send_async(req).await?;

    let bytes = res.bytes().await?;
    log::debug!("refresh response: {:?}", str::from_utf8(&bytes).unwrap());
    let data = serde_json::from_slice::<AuthData>(&bytes)?;

    if std::env::var("DUMP_TOKENS").is_ok() {
        std::fs::File::create("tokens.json")?.write_all(&bytes)?;
    }

    Ok(data)
}

const IGNORE_NAMESPACES: &[&str] = &[
    "ue",                               // Unreal Engine
    "89efe5924d3d467c839449ab6ab52e7f", // Fab assets
];

pub struct LibraryCatalog {
    pub items: Vec<CatalogItem>,
    pub purchase_dates: std::collections::HashMap<String, UtcDateTime>,
}

const STORE_GQL_URL: &'static str = formatcp!("https://{}/graphql", STORE_GQL_HOST);
const LIBRARY_QUERY: &str = include_str!("graphql/getUserLibrary.gql");
const DETAILS_QUERY: &str = include_str!("graphql/getGameDetails.gql");
const NAMESPACE_QUERY: &str = include_str!("graphql/getCatalogNamespace.gql");
const OFFER_QUERY: &str = include_str!("graphql/getCatalogOffer.gql");

fn trim_trailing_whitespace(bytes: &[u8]) -> &[u8] {
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &bytes[..end]
}

#[derive(Serialize)]
struct GqlVariables {
    cursor: Option<String>,
    locale: String,
}

#[derive(Serialize)]
struct GqlRequest<'a> {
    query: &'a str,
    variables: GqlVariables,
}

#[derive(Debug, Deserialize)]
struct GqlError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct GqlResponse {
    data: Option<GqlData>,
    #[serde(default)]
    errors: Option<Vec<GqlError>>,
}

#[derive(Debug, Deserialize)]
struct GqlData {
    #[serde(rename = "Library")]
    library: GqlLibrary,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlLibrary {
    library_items: GqlLibraryItems,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlLibraryItems {
    // Kept as raw values so one malformed record is skipped with a warning
    // instead of failing the whole page (see `parse_library_records`).
    #[serde(default)]
    records: Vec<serde_json::Value>,
    response_metadata: Option<GqlResponseMeta>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlResponseMeta {
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlRecord {
    #[serde(
        default,
        deserialize_with = "crate::decode::deserialize_optional_datetime"
    )]
    acquisition_date: Option<UtcDateTime>,
    // Record-level id used for pricing and critic reviews (not inside
    // `catalogItem`, so it is copied onto the item during merge).
    #[serde(default)]
    product_id: Option<String>,
    #[serde(default)]
    catalog_item: Option<CatalogItem>,
}

fn record_label(value: &serde_json::Value) -> String {
    let title = value
        .get("catalogItem")
        .and_then(|item| item.get("title"))
        .and_then(|title| title.as_str())
        .unwrap_or("<unknown title>");
    let id = value
        .get("catalogItem")
        .and_then(|item| item.get("id"))
        .and_then(|id| id.as_str())
        .unwrap_or("<unknown id>");
    format!("{title} ({id})")
}

/// Convert one page's raw record values, skipping malformed ones with a
/// warning so a single bad record can't discard ~100 games.
fn parse_library_records(values: &[serde_json::Value]) -> Vec<GqlRecord> {
    values
        .iter()
        .filter_map(
            |value| match serde_json::from_value::<GqlRecord>(value.clone()) {
                Ok(record) => Some(record),
                Err(e) => {
                    log::warn!(
                        "skipping malformed library record {}: {}",
                        record_label(value),
                        e
                    );
                    None
                }
            },
        )
        .collect()
}

/// Fetch the whole library (catalog info included) via the undocumented
/// store GraphQL API. Page cursors are base64 `{"offset":N}` blobs, so after
/// probing the first page (which reveals the page step) the remaining pages
/// are fetched with up to `PAGE_PARALLELISM` requests in flight: each
/// completion spawns the next unrequested offset until a page comes back
/// with a null cursor, empty records, or an error. Requires the store UA +
/// bearer auth, else 401.
pub async fn get_library_catalog(
    client: &isahc::HttpClient,
    auth_token: &str,
) -> anyhow::Result<LibraryCatalog> {
    use iced::futures::future::{FutureExt, select_all};

    const PAGE_PARALLELISM: usize = 5;
    const FALLBACK_STEP: u64 = 100;

    // Probe: the first page is fetched alone so the page step can be derived
    // from the server's own cursor instead of assumed.
    let first = fetch_library_page(client, auth_token, None, 0).await?;
    if first.next_cursor.is_none() || first.records.is_empty() {
        return Ok(merge_library_pages(vec![first]));
    }
    let step = first
        .next_cursor
        .as_deref()
        .and_then(crate::decode::decode_offset_cursor)
        .filter(|step| *step > 0)
        .unwrap_or_else(|| {
            log::warn!(
                "unexpected library cursor shape {:?}, assuming page step {FALLBACK_STEP}",
                first.next_cursor
            );
            FALLBACK_STEP
        });

    let mut pages = vec![first];
    let mut next_offset = step;
    let mut pending: Vec<_> = (0..PAGE_PARALLELISM)
        .map(|_| {
            let offset = next_offset;
            next_offset += step;
            fetch_library_page(
                client,
                auth_token,
                Some(crate::decode::encode_offset_cursor(offset)),
                offset,
            )
            .boxed()
        })
        .collect();

    let mut end_found = false;
    while !pending.is_empty() {
        let (result, _index, rest) = select_all(pending).await;
        pending = rest;
        match result {
            Ok(page) => {
                let end = page.next_cursor.is_none() || page.records.is_empty();
                if end {
                    end_found = true;
                }
                pages.push(page);
                if !end_found {
                    let offset = next_offset;
                    next_offset += step;
                    pending.push(
                        fetch_library_page(
                            client,
                            auth_token,
                            Some(crate::decode::encode_offset_cursor(offset)),
                            offset,
                        )
                        .boxed(),
                    );
                }
            }
            Err(e) => {
                // Stop spawning; keep whatever pages already arrived so one
                // flaky page can't discard the whole library.
                log::error!("library page fetch failed, stopping early: {e:#}");
                end_found = true;
            }
        }
    }

    Ok(merge_library_pages(pages))
}

struct LibraryPage {
    offset: u64,
    records: Vec<serde_json::Value>,
    next_cursor: Option<String>,
}

/// Fetch a single library page (retried once), returning its raw records and
/// the server's `nextCursor`.
async fn fetch_library_page(
    client: &isahc::HttpClient,
    auth_token: &str,
    cursor: Option<String>,
    offset: u64,
) -> anyhow::Result<LibraryPage> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        match fetch_library_page_once(client, auth_token, cursor.clone()).await {
            Ok((records, next_cursor)) => {
                return Ok(LibraryPage {
                    offset,
                    records,
                    next_cursor,
                });
            }
            Err(e) if attempt < 2 => {
                log::warn!("library page at offset {offset} failed, retrying: {e:#}");
            }
            Err(e) => {
                return Err(anyhow::anyhow!("library page at offset {offset}: {e:#}"));
            }
        }
    }
}

async fn fetch_library_page_once(
    client: &isahc::HttpClient,
    auth_token: &str,
    cursor: Option<String>,
) -> anyhow::Result<(Vec<serde_json::Value>, Option<String>)> {
    let body = serde_json::to_vec(&GqlRequest {
        query: LIBRARY_QUERY,
        variables: GqlVariables {
            cursor: cursor.clone(),
            locale: "en".to_string(),
        },
    })?;

    let req = Request::post(STORE_GQL_URL)
        .bearer_auth(auth_token)
        .user_agent(STORE_USER_AGENT)
        .content_type(ContentType::Json)
        .body(body)
        .unwrap();
    let mut res = client.send_async(req).await?;
    let bytes = res.bytes().await?;

    log::debug!(
        "library gql response: {:?}",
        str::from_utf8(&bytes).unwrap_or("<non-utf8>")
    );

    // Trailing whitespace (e.g. the final newline) is valid JSON and
    // ignored by serde; trim defensively so transport padding can't
    // trip page parsing.
    let trimmed = trim_trailing_whitespace(&bytes);
    let gql = serde_json::from_slice::<GqlResponse>(trimmed).map_err(|e| {
        anyhow::anyhow!(
            "failed to parse library GraphQL page (cursor={cursor:?}, {} bytes): {e}",
            bytes.len()
        )
    })?;
    if let Some(errors) = gql.errors
        && !errors.is_empty()
    {
        let msgs: Vec<_> = errors.iter().map(|e| e.message.as_str()).collect();
        bail!("library GraphQL query failed: {}", msgs.join("; "));
    }
    let Some(data) = gql.data else {
        bail!("library GraphQL response missing data");
    };

    let items = data.library.library_items;
    Ok((
        items.records,
        items.response_metadata.and_then(|m| m.next_cursor),
    ))
}

/// Merge fetched pages in offset order into catalog items + purchase dates,
/// applying the usual namespace / platform filters.
fn merge_library_pages(mut pages: Vec<LibraryPage>) -> LibraryCatalog {
    pages.sort_by_key(|page| page.offset);
    let mut items = Vec::new();
    let mut purchase_dates = std::collections::HashMap::new();
    for page in pages {
        for record in parse_library_records(&page.records) {
            let Some(mut item) = record.catalog_item else {
                continue;
            };
            if IGNORE_NAMESPACES.contains(&item.namespace.as_str()) {
                continue;
            }
            if !item.supports_windows() {
                continue;
            }
            item.product_id = record.product_id;
            if let Some(date) = record.acquisition_date {
                purchase_dates.insert(item.id.clone(), date);
            }
            items.push(item);
        }
    }
    LibraryCatalog {
        items,
        purchase_dates,
    }
}

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

#[derive(Debug, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct NamespaceInfo {
    #[serde(default)]
    pub mappings: Vec<NamespaceMapping>,
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub store: String,
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

    let trimmed = trim_trailing_whitespace(&bytes);
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

    let trimmed = trim_trailing_whitespace(&bytes);
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

    let trimmed = trim_trailing_whitespace(&bytes);
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

const MANIFEST_URL: &'static str = formatcp!(
    "https://{}/launcher/api/public/assets/v2/platform/Windows",
    LAUNCHER_HOST
);

pub async fn get_game_manifest(
    client: &isahc::HttpClient,
    auth_token: &str,
    namespace: &str,
    app_name: &str,
    catalog_item_id: &str,
) -> anyhow::Result<String> {
    let url = format!(
        "{MANIFEST_URL}/namespace/{namespace}/catalogItem/{catalog_item_id}/app/{app_name}/label/Live"
    );

    let req = Request::get(&url)
        .bearer_auth(auth_token)
        .user_agent(USER_AGENT)
        .body(())
        .unwrap();

    let mut res = client.send_async(req).await?;

    log::debug!("game assets: {:?}", res);

    Ok(res.text().await?)
}

/// OpenCritic score kept on each library item: average score, recommend
/// percentage and the OpenCritic page URL. All other API fields are ignored.
#[derive(Debug, Clone, PartialEq)]
pub struct CriticScore {
    pub average: i32,
    pub recommend_percentage: i32,
    pub url: String,
}

impl CriticScore {
    /// Rating word on the OpenCritic scale (Mighty/Strong/Fair/Weak),
    /// derived from the average score.
    pub fn rating(&self) -> &'static str {
        if self.average >= 84 {
            "Mighty"
        } else if self.average >= 75 {
            "Strong"
        } else if self.average >= 66 {
            "Fair"
        } else {
            "Weak"
        }
    }
}

#[derive(Debug, Deserialize)]
struct OpenCriticResponse {
    #[serde(default, rename = "criticReviews")]
    critic_reviews: Option<OpenCriticReviews>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenCriticReviews {
    #[serde(default)]
    critic_average: Option<i32>,
    #[serde(default)]
    recommend_percentage: Option<i32>,
    #[serde(default)]
    url: Option<String>,
}

/// Parse one open-critic response body into a score; `None` when the product
/// has no critic data. Unknown fields are ignored.
fn parse_critic_response(bytes: &[u8]) -> anyhow::Result<Option<CriticScore>> {
    let resp = serde_json::from_slice::<OpenCriticResponse>(bytes)?;
    let Some(reviews) = resp.critic_reviews else {
        return Ok(None);
    };
    let (Some(average), Some(recommend_percentage), Some(url)) = (
        reviews.critic_average,
        reviews.recommend_percentage,
        reviews.url,
    ) else {
        return Ok(None);
    };
    if url.is_empty() {
        return Ok(None);
    }
    Ok(Some(CriticScore {
        average,
        recommend_percentage,
        url,
    }))
}

/// Fetch OpenCritic scores for one product (`productId` from the library
/// record). No authentication needed. Returns `None` when the product has
/// no critic data instead of failing.
pub async fn get_critic_reviews(
    client: &isahc::HttpClient,
    product_id: &str,
) -> anyhow::Result<Option<CriticScore>> {
    let url = format!(
        "https://egs-platform-service.store.epicgames.com/api/v1/egs/products/{product_id}\
        /critic-reviews/open-critic?count=1&locale=en&start=0&store=EGS"
    );
    let req = Request::get(&url)
        .user_agent(STORE_USER_AGENT)
        .body(())
        .unwrap();
    let mut res = client.send_async(req).await?;
    if !res.status().is_success() {
        log::debug!(
            "critic reviews for {product_id} returned HTTP {}",
            res.status()
        );
        return Ok(None);
    }
    let bytes = res.bytes().await?;
    log::debug!(
        "critic reviews response for {product_id}: {:?}",
        str::from_utf8(&bytes).unwrap_or("<non-utf8>")
    );
    parse_critic_response(&bytes)
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

impl CatalogItem {
    pub fn is_dlc(&self) -> bool {
        self.main_game_item.is_some()
    }

    pub fn supports_windows(&self) -> bool {
        self.release_info
            .iter()
            .any(|r| r.platform.iter().any(|p| p == "Windows"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &str, main: Option<&str>) -> CatalogItem {
        CatalogItem {
            id: "id".to_string(),
            namespace: "ns".to_string(),
            title: title.to_string(),
            description: String::new(),
            key_images: Vec::new(),
            categories: vec!["games".to_string()],
            creation_date: chrono::Utc::now(),
            last_modified_date: chrono::Utc::now(),
            developer: String::new(),
            dlc_item_list: None,
            main_game_item: main.map(|id| MainGameRef {
                id: id.to_string(),
                namespace: String::new(),
                unsearchable: false,
            }),
            release_info: Vec::new(),
            product_id: None,
            critic: None,
            search_key: crate::search::build_search_key(title),
        }
    }

    #[test]
    fn dlc_detected_via_main_game_item_not_categories() {
        assert!(!item("Game", None).is_dlc());
        // Mislabeled DLC: tagged `games` but has a parent game.
        assert!(item("Commander Lilith DLC", Some("parent")).is_dlc());
        assert!(item("Map Pack", Some("parent")).is_dlc());
    }

    #[test]
    fn odd_datetimes_fall_back_instead_of_failing() {
        use crate::decode::parse_datetime_lenient;

        // Standard Epic shape.
        assert!(parse_datetime_lenient("2020-05-21T15:09:34.499Z").is_some());
        // Observed variants: space separator, missing zone, date-only.
        assert!(parse_datetime_lenient("2020-05-21 15:09:34").is_some());
        assert!(parse_datetime_lenient("2020-05-21T15:09:34").is_some());
        assert!(parse_datetime_lenient("2020-05-21").is_some());
        // Garbage / empty.
        assert!(parse_datetime_lenient("not-a-date").is_none());
        assert!(parse_datetime_lenient("").is_none());

        // A record with garbage dates still deserializes via fallbacks.
        let record: GqlRecord = serde_json::from_value(serde_json::json!({
            "acquisitionDate": "garbage",
            "catalogItem": {
                "id": "abc",
                "namespace": "ns",
                "title": "Weird Dates",
                "keyImages": [],
                "categories": [],
                "creationDate": "",
                "lastModifiedDate": null,
                "dlcItemList": null,
                "mainGameItem": null,
                "releaseInfo": [{"appId": "x", "platform": ["Windows"]}],
            }
        }))
        .unwrap();
        assert!(record.acquisition_date.is_none());
        let item = record.catalog_item.expect("item");
        assert_eq!(item.creation_date, crate::decode::default_epoch());
        assert!(item.supports_windows());
    }

    #[test]
    fn offset_cursors_round_trip() {
        use crate::decode::{decode_offset_cursor, encode_offset_cursor};

        // Known server value for the second page.
        assert_eq!(encode_offset_cursor(100), "eyJvZmZzZXQiOjEwMH0=");
        assert_eq!(decode_offset_cursor("eyJvZmZzZXQiOjEwMH0="), Some(100));
        assert_eq!(decode_offset_cursor(&encode_offset_cursor(0)), Some(0));
        assert_eq!(
            decode_offset_cursor(&encode_offset_cursor(12345)),
            Some(12345)
        );
        assert_eq!(decode_offset_cursor(""), None);
        assert_eq!(decode_offset_cursor("!!!"), None);
    }

    fn test_record(id: &str, title: &str) -> serde_json::Value {
        serde_json::json!({
            "acquisitionDate": "2020-05-21T15:09:34.499Z",
            "catalogItem": {
                "id": id,
                "namespace": "ns",
                "title": title,
                "keyImages": [],
                "categories": [{"path": "games"}],
                "creationDate": "2019-08-19T19:51:13.782Z",
                "lastModifiedDate": "2025-06-24T17:04:34.242Z",
                "dlcItemList": null,
                "mainGameItem": null,
                "releaseInfo": [{"appId": "x", "platform": ["Windows"]}],
            }
        })
    }

    #[test]
    fn merge_keeps_page_offset_order() {
        // Pages may finish out of order; merge must restore offset order and
        // carry purchase dates along.
        let catalog = merge_library_pages(vec![
            LibraryPage {
                offset: 100,
                records: vec![test_record("b", "Second")],
                next_cursor: None,
            },
            LibraryPage {
                offset: 0,
                records: vec![test_record("a", "First")],
                next_cursor: Some("cursor".to_string()),
            },
        ]);
        let titles: Vec<_> = catalog.items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["First", "Second"]);
        assert!(catalog.purchase_dates.contains_key("a"));
        assert!(catalog.purchase_dates.contains_key("b"));
    }

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

    #[test]
    fn malformed_record_does_not_kill_page() {
        let values = vec![
            serde_json::json!({
                "acquisitionDate": "2020-05-21T15:09:34.499Z",
                "catalogItem": {
                    "id": "good",
                    "namespace": "ns",
                    "title": "Good Game",
                    "keyImages": [],
                    "categories": [{"path": "games"}],
                    "creationDate": "2019-08-19T19:51:13.782Z",
                    "lastModifiedDate": "2025-06-24T17:04:34.242Z",
                    "dlcItemList": null,
                    "mainGameItem": null,
                    "releaseInfo": [{"appId": "x", "platform": ["Windows"]}],
                }
            }),
            // Missing required `id`: unrecoverable, must be skipped.
            serde_json::json!({
                "acquisitionDate": "2020-05-21T15:09:34.499Z",
                "catalogItem": {
                    "namespace": "ns",
                    "title": "Broken Game",
                    "keyImages": [],
                    "categories": [],
                    "creationDate": "2019-08-19T19:51:13.782Z",
                    "lastModifiedDate": "2025-06-24T17:04:34.242Z",
                }
            }),
        ];
        let records = parse_library_records(&values);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].catalog_item.as_ref().unwrap().title, "Good Game");
    }

    #[test]
    fn critic_parses_from_example() {
        let bytes = std::fs::read("tests/open-critic.json").unwrap();
        let score = parse_critic_response(&bytes)
            .expect("parses")
            .expect("has reviews");
        assert_eq!(score.average, 89);
        assert_eq!(score.recommend_percentage, 92);
        assert_eq!(
            score.url,
            "https://opencritic.com/game/2719/sid-meiers-civilization-vi"
        );
        assert_eq!(score.rating(), "Mighty");
    }

    #[test]
    fn critic_rating_follows_opencritic_scale() {
        let score = |average: i32| CriticScore {
            average,
            recommend_percentage: 0,
            url: "https://opencritic.com/game/1/x".to_string(),
        };
        assert_eq!(score(100).rating(), "Mighty");
        assert_eq!(score(84).rating(), "Mighty");
        assert_eq!(score(83).rating(), "Strong");
        assert_eq!(score(75).rating(), "Strong");
        assert_eq!(score(74).rating(), "Fair");
        assert_eq!(score(66).rating(), "Fair");
        assert_eq!(score(65).rating(), "Weak");
        assert_eq!(score(0).rating(), "Weak");
    }

    #[test]
    fn critic_missing_data_yields_none() {
        for body in [
            r#"{"criticReviews": null}"#,
            r#"{"criticReviews": {}}"#,
            r#"{"criticReviews": {"criticAverage": 89}}"#,
            r#"{"criticReviews": {"criticAverage": 89, "recommendPercentage": 92, "url": ""}}"#,
        ] {
            assert!(
                parse_critic_response(body.as_bytes())
                    .expect("parses")
                    .is_none(),
                "body should yield None: {body}"
            );
        }
    }

    #[test]
    fn merge_carries_product_id_onto_item() {
        let mut value = test_record("a", "First");
        value["productId"] = serde_json::json!("prod-123");
        let catalog = merge_library_pages(vec![LibraryPage {
            offset: 0,
            records: vec![value],
            next_cursor: None,
        }]);
        assert_eq!(catalog.items.len(), 1);
        assert_eq!(catalog.items[0].product_id.as_deref(), Some("prod-123"));
        assert_eq!(catalog.items[0].critic, None);
    }
}

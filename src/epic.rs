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
            let Some(item) = record.catalog_item else {
                continue;
            };
            if IGNORE_NAMESPACES.contains(&item.namespace.as_str()) {
                continue;
            }
            if !item.supports_windows() {
                continue;
            }
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DetailsVariables {
    locale: String,
    sandbox_id: String,
}

#[derive(Serialize)]
struct DetailsRequest<'a> {
    query: &'a str,
    variables: DetailsVariables,
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
    configs: Option<GameDetails>,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DetailsBanner {
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub description: String,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SocialLink {
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub platform: String,
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub url: String,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct StoreTag {
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub id: String,
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub name: String,
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub group_name: String,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Default)]
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

/// Store-page details for one product (`sandboxId` is the catalog
/// `namespace`). Only the fields the detail view shows are modeled;
/// everything else the query returns is ignored.
#[derive(Debug, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameDetails {
    #[serde(default)]
    pub product_display_name: Option<String>,
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub short_description: String,
    #[serde(default)]
    pub banner: Option<DetailsBanner>,
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub developer_display_name: String,
    #[serde(default, deserialize_with = "crate::decode::deserialize_null_default")]
    pub publisher_display_name: String,
    #[serde(default)]
    pub pc_release_date: Option<String>,
    #[serde(default)]
    pub effective_date: Option<String>,
    #[serde(default)]
    pub game_website: Option<String>,
    #[serde(default)]
    pub privacy_link: Option<String>,
    #[serde(default)]
    pub supported_text: Option<Vec<String>>,
    #[serde(default)]
    pub supported_audio: Option<Vec<String>>,
    #[serde(default)]
    pub social_links: Option<Vec<SocialLink>>,
    #[serde(default)]
    pub tags: Option<Vec<StoreTag>>,
    #[serde(default)]
    pub technical_requirements: Option<TechRequirements>,
    #[serde(default)]
    pub legal_text: Option<String>,
}

impl GameDetails {
    /// Products without a store page (e.g. legacy GTA 5) come back with
    /// almost every field null; `productDisplayName` is never null on normal
    /// games, so its absence marks them.
    pub fn has_store_page(&self) -> bool {
        self.product_display_name
            .as_deref()
            .is_some_and(|date| !date.is_empty())
    }

    /// Long blurb when the product has one, else the short description.
    pub fn description(&self) -> &str {
        self.banner
            .as_ref()
            .map(|banner| banner.description.as_str())
            .filter(|description| !description.is_empty())
            .unwrap_or(&self.short_description)
    }

    /// Release date as `YYYY-MM-DD` when the value carries a time part.
    pub fn release_date(&self) -> Option<&str> {
        self.pc_release_date
            .as_deref()
            .or(self.effective_date.as_deref())
            .map(|date| date.get(..10).unwrap_or(date))
    }
}

/// Fetch one product's store-page details. `sandbox_id` is the catalog
/// `namespace`; `locale` is always `en` for now and `templateId` is left
/// unset. Same store GraphQL endpoint/auth as the library fetch.
pub async fn get_game_details(
    client: &isahc::HttpClient,
    auth_token: &str,
    sandbox_id: &str,
) -> anyhow::Result<GameDetails> {
    let body = serde_json::to_vec(&DetailsRequest {
        query: DETAILS_QUERY,
        variables: DetailsVariables {
            locale: "en".to_string(),
            sandbox_id: sandbox_id.to_string(),
        },
    })?;

    log::debug!("fetching game details for sandbox_id: {}", sandbox_id);

    let req = Request::post(STORE_GQL_URL)
        .bearer_auth(auth_token)
        .user_agent(STORE_USER_AGENT)
        .content_type(ContentType::Json)
        .body(body)
        .unwrap();
    let mut res = client.send_async(req).await?;
    let bytes = res.bytes().await?;

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
    if let Some(errors) = gql.errors
        && !errors.is_empty()
    {
        let msgs: Vec<_> = errors.iter().map(|e| e.message.as_str()).collect();
        bail!("game details GraphQL query failed: {}", msgs.join("; "));
    }
    let Some(data) = gql.data else {
        bail!("game details GraphQL response missing data");
    };

    data.product
        .sandbox
        .configuration
        .into_iter()
        .find_map(|entry| entry.configs)
        .ok_or_else(|| anyhow::anyhow!("game details response has no StoreConfiguration"))
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
    fn graphql_first_page_deserializes() {
        let bytes = std::fs::read("tests/graphql_output.json").unwrap();
        let trimmed = trim_trailing_whitespace(&bytes);
        let resp: GqlResponse = serde_json::from_slice(trimmed).unwrap();
        let data = resp.data.expect("data");
        let records = parse_library_records(&data.library.library_items.records);
        assert!(!records.is_empty());

        // Fallback: search by title across all records.
        let lilith = records
            .iter()
            .filter_map(|r| r.catalog_item.as_ref())
            .find(|c| c.title == "Commander Lilith DLC")
            .expect("lilith record");
        assert!(lilith.is_dlc());
        assert!(lilith.supports_windows());

        let civ = records
            .iter()
            .filter_map(|r| r.catalog_item.as_ref())
            .find(|c| c.title == "Sid Meier's Civilization VI")
            .expect("civ record");
        assert!(!civ.is_dlc());

        let map_pack = records
            .iter()
            .filter_map(|r| r.catalog_item.as_ref())
            .find(|c| c.title == "Map Pack")
            .expect("map pack record");
        assert!(map_pack.is_dlc());

        assert!(
            data.library
                .library_items
                .response_metadata
                .and_then(|m| m.next_cursor)
                .is_some()
        );
        let _ = lilith;
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
        let bytes = std::fs::read("tests/game_detail.json").unwrap();
        let resp: DetailsGqlResponse = serde_json::from_slice(&bytes).unwrap();
        let configs: Vec<_> = resp
            .data
            .expect("data")
            .product
            .sandbox
            .configuration
            .into_iter()
            .filter_map(|entry| entry.configs)
            .collect();
        // The other configuration fragments come back as `{}`.
        assert_eq!(configs.len(), 1);
        let details = &configs[0];

        assert_eq!(details.product_display_name, Some("MudRunner".into()));
        assert_eq!(details.developer_display_name, "Saber Interactive");
        assert_eq!(details.publisher_display_name, "Focus Entertainment");
        assert!(details.short_description.contains("ultimate off-road"));
        // No banner blurb here, so the short description is used.
        assert_eq!(details.description(), details.short_description.as_str());
        assert!(details.release_date().is_none());

        let tags: Vec<_> = details
            .tags
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|tag| tag.name.as_str())
            .collect();
        assert!(tags.contains(&"Single Player"));
        assert!(tags.contains(&"Cloud Saves"));

        let languages = details.supported_text.as_deref().unwrap_or_default();
        assert!(languages.contains(&"English".to_string()));

        let windows = details
            .technical_requirements
            .as_ref()
            .and_then(|reqs| reqs.windows.as_deref())
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
        assert!(
            details
                .technical_requirements
                .as_ref()
                .and_then(|reqs| reqs.macos.as_deref())
                .is_none()
        );
    }

    #[test]
    fn product_without_store_page_parses_and_is_detected() {
        // Legacy GTA 5: almost every field is explicit null.
        let bytes = serde_json::json!({
            "data": {
                "Product": {
                    "sandbox": {
                        "configuration": [{
                            "configs": {
                                "banner": null,
                                "developerDisplayName": null,
                                "effectiveDate": null,
                                "externalPlatformLaunchOptions": null,
                                "gameWebsite": null,
                                "legalText": null,
                                "pcReleaseDate": null,
                                "productDisplayName": "Grand Theft Auto V",
                                "privacyLink": null,
                                "publisherDisplayName": null,
                                "shortDescription": null,
                                "socialLinks": null,
                                "supportedAudio": null,
                                "supportedText": null,
                                "tags": [],
                                "technicalRequirements": null,
                                "theme": {
                                    "dark": {"accent": "#0074E4", "theme": "gray"},
                                    "light": {"accent": "#0074E4", "theme": "gray"},
                                    "preferredMode": "dark"
                                }
                            }
                        }]
                    }
                }
            }
        });
        let resp: DetailsGqlResponse =
            serde_json::from_value(bytes).expect("legacy GTA 5 response parses");
        let details = resp
            .data
            .expect("data")
            .product
            .sandbox
            .configuration
            .into_iter()
            .find_map(|entry| entry.configs)
            .expect("configs");

        assert_eq!(
            details.product_display_name,
            Some("Grand Theft Auto V".into())
        );
        assert!(!details.has_store_page());
        assert!(details.release_date().is_none());
        assert_eq!(details.description(), "");
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
}

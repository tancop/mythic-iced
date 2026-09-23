use anyhow::bail;
use const_format::formatcp;
use isahc::{
    AsyncReadResponseExt, Request,
    auth::{Authentication, Credentials},
    config::Configurable,
    http,
};
use serde::{Deserialize, Serialize};
use std::{io::Write, sync::Arc};

use crate::decode;

const USER_AGENT: &'static str =
    "UELauncher/11.0.1-14907503+++Portal+Release-Live Windows/10.0.19041.1.256.64bit";
const STORE_USER_AGENT: &'static str = "EpicGamesLauncher/14.0.8-22004686+++Portal+Release-Live";
// required for the oauth request;
const USER_BASIC: &'static str = "34a02cf8f4414e29b15921876da36f9a";
const PW_BASIC: &'static str = "daafbccc737745039dffe53d94fc76cf";
const LABEL: &'static str = "Live-EternalKnight";

const OAUTH_HOST: &'static str = "account-public-service-prod03.ol.epicgames.com";
const LAUNCHER_HOST: &'static str = "launcher-public-service-prod06.ol.epicgames.com";
const ENTITLEMENTS_HOST: &'static str = "entitlement-public-service-prod08.ol.epicgames.com";
const CATALOG_HOST: &'static str = "catalog-public-service-prod06.ol.epicgames.com";
const ECOMMERCE_HOST: &'static str =
    "ecommerceintegration-public-service-ecomprod02.ol.epicgames.com";
const LIBRARY_HOST: &'static str = "library-service.live.use1a.on.epicgames.com";

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

    Ok(data)
}

const GET_LIBRARY_URL: &'static str =
    formatcp!("https://{}/library/api/public/items", LIBRARY_HOST);

const GET_LIBRARY_WITH_METADATA_URL: &'static str =
    formatcp!("{}?includeMetadata=true", GET_LIBRARY_URL);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LibraryResponseMeta {
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LibraryResponse {
    records: Vec<LibraryItem>,
    response_metadata: Option<LibraryResponseMeta>,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItem {
    pub namespace: Arc<String>,
    pub catalog_item_id: Arc<String>,
    pub product_id: String,
    // app ID, or codename for older titles
    pub app_name: String,
    // internal app name
    pub sandbox_name: String,
    // public/private
    pub sandbox_type: String,
    // playable platforms, e.g. ["Windows"] or ["Win32", "Windows"]
    #[serde(default)]
    pub platform: Vec<String>,
}

const IGNORE_NAMESPACES: &[&str] = &[
    "ue",                               // Unreal Engine
    "89efe5924d3d467c839449ab6ab52e7f", // Fab assets
];

pub async fn get_library_items(
    client: &isahc::HttpClient,
    auth_token: &str,
) -> anyhow::Result<Vec<LibraryItem>> {
    let mut result = Vec::new();

    let mut url = GET_LIBRARY_WITH_METADATA_URL.to_string();

    loop {
        let req = Request::get(&url)
            .bearer_auth(auth_token)
            .user_agent(USER_AGENT)
            .body(())
            .unwrap();
        let mut res = client.send_async(req).await?;

        let res = res.bytes().await?;

        log::debug!("library items: {:?}", str::from_utf8(&res).unwrap());
        let mut file = std::fs::File::create("library_items.json")?;
        file.write_all(&res)?;

        let res = serde_json::from_slice::<LibraryResponse>(&res)?;

        result.extend(res.records.into_iter().filter(|item| {
            !IGNORE_NAMESPACES.contains(&item.namespace.as_str())
                && item.platform.iter().any(|p| p == "Windows")
        }));

        if let Some(meta) = res.response_metadata
            && let Some(cursor) = meta.next_cursor
        {
            url.truncate(GET_LIBRARY_WITH_METADATA_URL.len());
            url.push_str(&format!("&cursor={cursor}"));
        } else {
            // reached end of library
            break;
        }
    }

    Ok(result)
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
    pub md5: String,
    pub width: u16,
    pub height: u16,
    pub size: u32,
    pub uploaded_date: UtcDateTime,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct DlcRef {
    pub id: String,
    pub namespace: String,
    pub unsearchable: bool,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogItem {
    pub id: String,
    pub title: String,
    pub description: String,
    pub key_images: Vec<KeyImage>,
    #[serde(deserialize_with = "crate::decode::deserialize_path_list")]
    pub categories: Vec<String>,

    pub creation_date: UtcDateTime,
    pub last_modified_date: UtcDateTime,

    pub developer: String,

    // None when the item is a DLC
    pub dlc_item_list: Option<Vec<DlcRef>>,
}

const GAME_INFO_URL: &'static str = formatcp!("https://{}/catalog/api/shared", CATALOG_HOST);

pub async fn get_game_info(
    client: &isahc::HttpClient,
    auth_token: &str,
    namespace: &str,
    catalog_id: &str,
) -> anyhow::Result<CatalogItem> {
    let url = format!(
        "{}/namespace/{}/bulk/items?id={}&includeDLCDetails=false\
            &includeMainGameDetails=true&country=US&locale=en",
        GAME_INFO_URL, namespace, catalog_id
    );

    let req = Request::get(url)
        .bearer_auth(auth_token)
        .user_agent(USER_AGENT)
        .body(())
        .unwrap();

    let mut res = client.send_async(req).await?;

    log::debug!("game info: {:?}", res);

    let res = res
        .json::<decode::SingleValueWrapper<CatalogItem>>()
        .await?;

    Ok(res.0)
}

use anyhow::bail;
use const_format::formatcp;
use isahc::{AsyncReadResponseExt, Request};
use serde::{Deserialize, Serialize};
use std::io::Write;

use super::{AuthData, ContentType, RequestBuilderExt, USER_AGENT};

// required for the oauth request;
const USER_BASIC: &'static str = "34a02cf8f4414e29b15921876da36f9a";
const PW_BASIC: &'static str = "daafbccc737745039dffe53d94fc76cf";
const LABEL: &'static str = "Live-EternalKnight";

const OAUTH_HOST: &'static str = "account-public-service-prod03.ol.epicgames.com";

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

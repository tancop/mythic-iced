use const_format::formatcp;
use isahc::{AsyncReadResponseExt, Request};

use super::{RequestBuilderExt, USER_AGENT};

const LAUNCHER_HOST: &'static str = "launcher-public-service-prod06.ol.epicgames.com";

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

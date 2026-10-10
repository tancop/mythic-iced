use isahc::{AsyncReadResponseExt, Request};
use serde::Deserialize;

use super::{ContentType, GqlError, RequestBuilderExt, STORE_GQL_URL, STORE_USER_AGENT};

const STARS_QUERY: &str = include_str!("../graphql/getProductStars.gql");

#[derive(Debug, Deserialize)]
struct StarsGqlResponse {
    data: Option<StarsData>,
    #[serde(default)]
    errors: Option<Vec<GqlError>>,
}

#[derive(Debug, Deserialize)]
struct StarsData {
    #[serde(rename = "RatingsPolls")]
    ratings_polls: Option<StarsPolls>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StarsPolls {
    #[serde(default)]
    get_product_result: Option<StarsResult>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StarsResult {
    #[serde(default)]
    average_rating: Option<f32>,
}

/// Parse one getProductStars response body into the Epic user rating
/// (stars out of 5, e.g. 4.67); `None` when the product has no rating.
#[cfg(test)]
fn parse_stars_response(bytes: &[u8]) -> anyhow::Result<Option<f32>> {
    let gql = serde_json::from_slice::<StarsGqlResponse>(bytes)?;
    Ok(gql
        .data
        .and_then(|d| d.ratings_polls)
        .and_then(|p| p.get_product_result)
        .and_then(|r| r.average_rating))
}

/// Fetch the Epic user rating (stars out of 5) for one product.
/// `sandbox_id` is the catalog `namespace`. Returns `None` when the
/// product has no rating. Same store GraphQL endpoint/auth as the
/// library fetch.
pub async fn get_product_stars(
    client: &isahc::HttpClient,
    auth_token: &str,
    sandbox_id: &str,
) -> anyhow::Result<Option<f32>> {
    let body = serde_json::to_vec(&serde_json::json!({
        "query": STARS_QUERY,
        "variables": {
            "sandboxId": sandbox_id,
            "locale": "en",
        },
    }))?;
    let req = Request::post(STORE_GQL_URL)
        .bearer_auth(auth_token)
        .user_agent(STORE_USER_AGENT)
        .content_type(ContentType::Json)
        .body(body)
        .unwrap();
    let mut res = client.send_async(req).await?;
    let bytes = res.bytes().await?;
    log::debug!(
        "product stars gql response for {sandbox_id}: {:?}",
        str::from_utf8(&bytes).unwrap_or("<non-utf8>")
    );
    let trimmed = super::trim_trailing_whitespace(&bytes);
    let gql = serde_json::from_slice::<StarsGqlResponse>(trimmed).map_err(|e| {
        anyhow::anyhow!(
            "failed to parse product stars response ({} bytes): {e}",
            bytes.len()
        )
    })?;
    if let Some(errors) = gql.errors
        && !errors.is_empty()
    {
        let msgs: Vec<_> = errors.iter().map(|e| e.message.as_str()).collect();
        anyhow::bail!("product stars GraphQL query failed: {}", msgs.join("; "));
    }
    Ok(gql
        .data
        .and_then(|d| d.ratings_polls)
        .and_then(|p| p.get_product_result)
        .and_then(|r| r.average_rating))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stars_parse_decimal_rating() {
        let body = serde_json::json!({
            "data": {
                "RatingsPolls": {
                    "getProductResult": { "averageRating": 4.67 }
                }
            }
        });
        let bytes = serde_json::to_vec(&body).unwrap();
        assert_eq!(parse_stars_response(&bytes).unwrap(), Some(4.67));
    }

    #[test]
    fn stars_missing_rating_yields_none() {
        for body in [
            serde_json::json!({"data": {"RatingsPolls": {"getProductResult": null}}}),
            serde_json::json!({"data": {"RatingsPolls": {"getProductResult": {}}}}),
            serde_json::json!({"data": {"RatingsPolls": {"getProductResult": {"averageRating": null}}}}),
            serde_json::json!({"data": null}),
        ] {
            let bytes = serde_json::to_vec(&body).unwrap();
            assert!(
                parse_stars_response(&bytes).unwrap().is_none(),
                "body should yield None: {body}"
            );
        }
    }
}

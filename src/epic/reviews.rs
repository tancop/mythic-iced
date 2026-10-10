use std::str::FromStr;

use anyhow::{Context, bail};
use isahc::{AsyncReadResponseExt, Request};
use serde::Deserialize;

use super::{CriticRating, CriticScore, RequestBuilderExt, STORE_USER_AGENT};

#[derive(Debug, Deserialize)]
struct EpicResponse {
    #[serde(default, rename = "criticReviews")]
    critic_reviews: Option<EpicCriticReviews>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EpicCriticReviews {
    #[serde(default)]
    critic_average: Option<i32>,
    #[serde(default)]
    recommend_percentage: Option<i32>,
    #[serde(default)]
    critic_rating: Option<CriticRating>,
    #[serde(default)]
    url: Option<String>,
}

impl CriticScore {
    /// Rating word on the OpenCritic scale (Mighty/Strong/Fair/Weak),
    /// derived from the average score.
    pub fn text_rating(&self) -> &'static str {
        match self.rating {
            CriticRating::Mighty => "Mighty",
            CriticRating::Strong => "Strong",
            CriticRating::Fair => "Fair",
            CriticRating::Weak => "Weak",
        }
    }
}

/// Parse one open-critic response body into a score; `None` when the product
/// has no critic data. Unknown fields are ignored.
fn parse_epic_response(bytes: &[u8]) -> anyhow::Result<Option<CriticScore>> {
    let resp = serde_json::from_slice::<EpicResponse>(bytes)?;
    let Some(reviews) = resp.critic_reviews else {
        return Ok(None);
    };
    let (Some(average), Some(recommend_percentage), Some(url), Some(rating)) = (
        reviews.critic_average,
        reviews.recommend_percentage,
        reviews.url,
        reviews.critic_rating,
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
        rating,
    }))
}

const OPENCRITIC_CODE: &'static str =
    const_base::encode_as_str!("GkAFGoQOVHzhQzZIuXkh9pe94AlYH2yt", const_base::Config::B64);

#[derive(Deserialize)]
struct SearchItem {
    id: u32,
    dist: f32,
}

type SearchResponse = Vec<SearchItem>;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DetailsResponse {
    percent_recommended: f32,
    top_critic_score: f32,
    tier: String,
    url: String,
}

impl From<DetailsResponse> for CriticScore {
    fn from(value: DetailsResponse) -> Self {
        Self {
            average: value.top_critic_score as i32,
            rating: CriticRating::from_str(&value.tier).unwrap(),
            recommend_percentage: value.percent_recommended as i32,
            url: value.url,
        }
    }
}

/// Fetch critic scores for `productId` using OpenCritic's API. This endpoint
/// returns larger records than Epic (15 kB) and requires an extra network
/// call, so we use it as a backup.
async fn get_oc_reviews(
    client: &isahc::HttpClient,
    product_name: &str,
) -> anyhow::Result<CriticScore> {
    let search_url = format!(
        "https://api.opencritic.com/api/meta/search?criteria={}",
        urlencoding::encode(product_name)
    );
    let req = Request::get(&search_url)
        .bearer_auth(OPENCRITIC_CODE)
        .body(())
        .unwrap();

    let res = client
        .send_async(req)
        .await
        .context("oc search")?
        .bytes()
        .await?;
    let items = serde_json::from_slice::<SearchResponse>(&res)?;
    let Some(item) = items.first() else {
        bail!("Search for '{}' returned no items", product_name);
    };

    if item.dist > 0.0 {
        bail!("No match found for name '{}'", product_name);
    }

    let detail_url = format!("https://api.opencritic.com/api/game/{}", item.id);
    let req = Request::get(&detail_url)
        .bearer_auth(OPENCRITIC_CODE)
        .body(())
        .unwrap();

    let res = client
        .send_async(req)
        .await
        .context("oc details")?
        .bytes()
        .await?;

    Ok(serde_json::from_slice::<DetailsResponse>(&res)?.into())
}

/// Fetch OpenCritic scores for one product using the Epic Games API.
/// Faster than loading directly from OpenCritic but not supported for
/// all games.
async fn get_epic_reviews(
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
    parse_epic_response(&bytes)
}

/// Fetch OpenCritic scores for one product (`productId` from the library
/// record). No authentication needed. Returns `None` when the product has
/// no critic data instead of failing.
pub async fn get_critic_reviews(
    client: &isahc::HttpClient,
    product_id: &str,
    product_name: &str,
) -> anyhow::Result<Option<CriticScore>> {
    match get_epic_reviews(client, product_id).await {
        Ok(Some(score)) => Ok(Some(score)),
        Ok(None) => get_oc_reviews(client, product_name).await.map(|r| Some(r)),
        Err(e) => {
            log::debug!(
                "Failed to load critic score from Epic, falling back to OC: {}",
                e
            );
            get_oc_reviews(client, product_name).await.map(|r| Some(r))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn critic_parses_from_example() {
        let bytes = std::fs::read("tests/open-critic.json").unwrap();
        let score = parse_epic_response(&bytes)
            .expect("parses")
            .expect("has reviews");
        assert_eq!(score.average, 89);
        assert_eq!(score.recommend_percentage, 92);
        assert_eq!(
            score.url,
            "https://opencritic.com/game/2719/sid-meiers-civilization-vi"
        );
        assert_eq!(score.text_rating(), "Mighty");
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
                parse_epic_response(body.as_bytes())
                    .expect("parses")
                    .is_none(),
                "body should yield None: {body}"
            );
        }
    }
}

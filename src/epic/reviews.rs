use isahc::{AsyncReadResponseExt, Request};
use serde::Deserialize;

use super::{CriticRating, CriticScore, RequestBuilderExt, STORE_USER_AGENT};

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
fn parse_critic_response(bytes: &[u8]) -> anyhow::Result<Option<CriticScore>> {
    let resp = serde_json::from_slice::<OpenCriticResponse>(bytes)?;
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

#[cfg(test)]
mod tests {
    use super::*;

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
                parse_critic_response(body.as_bytes())
                    .expect("parses")
                    .is_none(),
                "body should yield None: {body}"
            );
        }
    }
}

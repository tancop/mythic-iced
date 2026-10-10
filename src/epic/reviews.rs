use std::str::FromStr;

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

impl DetailsResponse {
    fn into_score(self) -> anyhow::Result<CriticScore> {
        Ok(CriticScore {
            average: self.top_critic_score as i32,
            rating: CriticRating::from_str(&self.tier)
                .map_err(|_| anyhow::anyhow!("game has no rating, probably too few top critics"))?,
            recommend_percentage: self.percent_recommended as i32,
            url: self.url,
        })
    }
}

/// Error from [`get_critic_reviews`]. `Transient` means the request never
/// got a usable answer because of throttling or a dropped connection, so
/// the caller should requeue the game and try again later. `Fatal` means
/// retrying is pointless (unknown game, bad response).
#[derive(Debug)]
pub enum CriticError {
    Transient(anyhow::Error),
    Fatal(anyhow::Error),
}

impl std::fmt::Display for CriticError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transient(e) | Self::Fatal(e) => write!(f, "{e:#}"),
        }
    }
}

/// Connection-level failures that say nothing about the game itself: the
/// request died before (or during) the transfer, typically because the host
/// stopped accepting our burst of connections. Worth retrying later.
fn is_transient_send_error(e: &anyhow::Error) -> bool {
    matches!(
        e.downcast_ref::<isahc::Error>().map(isahc::Error::kind),
        Some(
            isahc::error::ErrorKind::ConnectionFailed
                | isahc::error::ErrorKind::Timeout
                | isahc::error::ErrorKind::Unknown
                | isahc::error::ErrorKind::Io
        )
    )
}

async fn send_oc(
    client: &isahc::HttpClient,
    req: isahc::Request<()>,
    label: &str,
    product_name: &str,
) -> Result<isahc::Response<isahc::AsyncBody>, CriticError> {
    client.send_async(req).await.map_err(|e| {
        let e = anyhow::Error::new(e).context(format!("oc {label} for '{product_name}'"));
        if is_transient_send_error(&e) {
            CriticError::Transient(e)
        } else {
            CriticError::Fatal(e)
        }
    })
}

/// Read the full body, then enforce the status. Throttling (429) and
/// server errors are transient; anything else non-success is fatal. Both
/// log status + a body snippet so throttling shows up as itself instead
/// of a confusing serde error downstream.
async fn read_oc_body(
    label: &str,
    product_name: &str,
    mut res: isahc::Response<isahc::AsyncBody>,
) -> Result<Vec<u8>, CriticError> {
    let status = res.status();
    let bytes = res.bytes().await.map_err(|e| {
        let e = anyhow::Error::new(e).context(format!("oc {label} body for '{product_name}'"));
        if is_transient_send_error(&e) {
            CriticError::Transient(e)
        } else {
            CriticError::Fatal(e)
        }
    })?;
    if status.as_u16() == 429 || status.is_server_error() {
        return Err(CriticError::Transient(anyhow::anyhow!(
            "oc {label} for '{product_name}' throttled: HTTP {status}"
        )));
    }
    if !status.is_success() {
        let snippet: String = String::from_utf8_lossy(&bytes).chars().take(300).collect();
        return Err(CriticError::Fatal(anyhow::anyhow!(
            "oc {label} for '{product_name}' returned HTTP {status}: {snippet}"
        )));
    }
    Ok(bytes)
}

/// Run one OpenCritic meta search for `name`, enforcing the status.
/// Parse errors are fatal.
async fn search_oc(client: &isahc::HttpClient, name: &str) -> Result<SearchResponse, CriticError> {
    let search_url = format!(
        "https://api.opencritic.com/api/meta/search?criteria={}",
        urlencoding::encode(name)
    );
    let req = Request::get(&search_url)
        .bearer_auth(OPENCRITIC_CODE)
        .body(())
        .unwrap();

    let res = send_oc(client, req, "search", name).await?;
    let body = read_oc_body("search", name, res).await?;
    serde_json::from_slice(&body).map_err(|e| {
        CriticError::Fatal(anyhow::Error::new(e).context(format!("oc search for '{name}'")))
    })
}

/// Strip a trailing "<word> edition" or "game of the year edition" from a
/// store title, plus any leftover trailing `-` or `:`. Returns `None` when
/// there is no such suffix or nothing would remain.
///
/// Only used as a second chance after the full title found no exact match:
/// stripping up front would misattribute real "Edition" games (e.g.
/// "Mafia Definitive Edition" is a remake, not "Mafia" with extras).
fn strip_edition_suffix(name: &str) -> Option<String> {
    let words: Vec<&str> = name.split_whitespace().collect();
    let strip = if words.len() > 5
        && words[words.len() - 5..]
            .iter()
            .map(|w| w.to_lowercase())
            .collect::<Vec<_>>()
            == ["game", "of", "the", "year", "edition"]
    {
        5
    } else if words.len() > 2 && words[words.len() - 1].eq_ignore_ascii_case("edition") {
        2
    } else {
        return None;
    };
    let stripped: String = words[..words.len() - strip].join(" ");
    let stripped = stripped
        .trim_end()
        .trim_end_matches(['-', ':'])
        .trim_end()
        .to_string();
    (!stripped.is_empty()).then_some(stripped)
}
/// Fetch one game's detail record by OpenCritic id. Parse errors are fatal.
async fn fetch_oc_details(
    client: &isahc::HttpClient,
    game_id: u32,
    name: &str,
) -> Result<DetailsResponse, CriticError> {
    let detail_url = format!("https://api.opencritic.com/api/game/{game_id}");
    log::debug!("fetch details for {name} from {detail_url}");
    let req = Request::get(&detail_url)
        .bearer_auth(OPENCRITIC_CODE)
        .body(())
        .unwrap();

    let res = send_oc(client, req, "details", name).await?;
    let body = read_oc_body("details", name, res).await?;
    serde_json::from_slice(&body).map_err(|e| {
        CriticError::Fatal(anyhow::Error::new(e).context(format!("oc details for '{name}'")))
    })
}

/// Fetch critic scores for `product_name` using OpenCritic's API. This
/// endpoint returns larger records than Epic (15 kB) and requires an extra
/// network call, so we use it as a backup.
async fn get_oc_reviews(
    client: &isahc::HttpClient,
    product_name: &str,
) -> Result<Option<CriticScore>, CriticError> {
    // Computed up front (no network): the only second chance we get, used
    // when the full title finds no exact match or its entry has no score.
    let stripped = strip_edition_suffix(product_name);

    let mut items = search_oc(client, product_name).await?;
    let mut searched_stripped = false;
    if items.first().is_some_and(|item| item.dist > 0.0)
        && let Some(stripped) = &stripped
    {
        // Later store editions often carry a suffix OpenCritic doesn't
        // know ("Control Ultimate Edition" vs "Control"). The full title
        // was already tried first, so try once with it stripped.
        log::debug!("No exact OC match for '{product_name}', retrying as '{stripped}'");
        items = search_oc(client, stripped).await?;
        searched_stripped = true;
    }
    let Some(item) = items.first() else {
        log::debug!("OC search for '{product_name}' returned no items");
        return Ok(None);
    };

    if item.dist > 0.0 {
        log::debug!("No match found for name '{product_name}'");
        return Ok(None);
    }

    let details = fetch_oc_details(client, item.id, product_name).await?;
    match details.into_score() {
        Ok(score) => Ok(Some(score)),
        Err(e) => {
            // OpenCritic knows this edition entry but hasn't scored it
            // (e.g. one review, no average): fall back to the base game,
            // unless the stripped name is what already got us here.
            if !searched_stripped && let Some(stripped) = &stripped {
                log::debug!("OC has no score for '{product_name}', retrying as '{stripped}'");
                let items = search_oc(client, stripped).await?;
                if let Some(item) = items.first().filter(|item| item.dist == 0.0) {
                    let details = fetch_oc_details(client, item.id, stripped).await?;
                    return Ok(Some(details.into_score().map_err(CriticError::Fatal)?));
                }
            }
            Err(CriticError::Fatal(e))
        }
    }
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
/// record), trying Epic first and OpenCritic's own API as fallback.
/// Returns `None` when neither has critic data. Errors are split into
/// transient (throttled / connection dropped: requeue and retry later)
/// and fatal (unknown game, bad response: give up).
pub async fn get_critic_reviews(
    client: &isahc::HttpClient,
    product_id: &str,
    product_name: &str,
) -> Result<Option<CriticScore>, CriticError> {
    log::debug!("Loading critic reviews for {product_id} ({product_name})...");
    match get_epic_reviews(client, product_id).await {
        Ok(Some(score)) => Ok(Some(score)),
        Ok(None) => get_oc_reviews(client, product_name).await,
        Err(e) => {
            log::debug!("Failed to load critic score from Epic, falling back to OC: {e:#}",);
            get_oc_reviews(client, product_name).await
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

    #[test]
    fn edition_suffix_strips_for_second_search() {
        for (name, stripped) in [
            ("Control Ultimate Edition", "Control"),
            (
                "The Witcher 3: Wild Hunt - Complete Edition",
                "The Witcher 3: Wild Hunt",
            ),
            (
                "Batman: Arkham Knight: Premium Edition",
                "Batman: Arkham Knight",
            ),
            ("Elden Ring: Game of the Year Edition", "Elden Ring"),
            ("Hades GOTY Edition", "Hades"),
        ] {
            assert_eq!(
                strip_edition_suffix(name).as_deref(),
                Some(stripped),
                "should strip: {name}"
            );
        }
    }

    #[test]
    fn edition_suffix_leaves_real_titles_alone() {
        // No edition suffix, or nothing would remain: no second search.
        for name in [
            "Control",
            "Mafia", // bare base game, not an edition lookup
            "Edition",
            "Foo Edition",
            "Star Wars Jedi: Survivor",
            "Digital Deluxe Upgrade",
        ] {
            assert_eq!(strip_edition_suffix(name), None, "should not strip: {name}");
        }
    }
}

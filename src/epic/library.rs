use anyhow::bail;
use isahc::{AsyncReadResponseExt, Request};
use serde::{Deserialize, Serialize};

use super::{
    CatalogItem, ContentType, GqlError, LibraryCatalog, RequestBuilderExt, STORE_GQL_URL,
    STORE_USER_AGENT, UtcDateTime,
};

const IGNORE_NAMESPACES: &[&str] = &[
    "ue",                               // Unreal Engine
    "89efe5924d3d467c839449ab6ab52e7f", // Fab assets
];

const LIBRARY_QUERY: &str = include_str!("../graphql/getUserLibrary.gql");

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
    let trimmed = super::trim_trailing_whitespace(&bytes);
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
    use crate::epic::MainGameRef;

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
            user_rating: None,
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
        assert_eq!(catalog.items[0].user_rating, None);
    }
}

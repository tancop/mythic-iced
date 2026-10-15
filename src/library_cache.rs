use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::epic::{CatalogItem, UtcDateTime};

#[derive(Serialize, Deserialize)]
struct LibraryCacheFile {
    items: Vec<CatalogItem>,
    #[serde(default)]
    purchase_dates: HashMap<String, UtcDateTime>,
}

pub fn cache_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap()
        .join("mythic")
        .join("library.json")
}

/// Load the cached library. Returns `None` when missing, empty, or corrupt.
/// `search_key` is skipped in the file, so callers must rebuild it.
pub fn load(path: &PathBuf) -> Option<(Vec<CatalogItem>, HashMap<String, UtcDateTime>)> {
    let bytes = fs::read(path).ok()?;
    if bytes.is_empty() {
        return None;
    }
    match serde_json::from_slice::<LibraryCacheFile>(&bytes) {
        Ok(data) => {
            log::info!("Loaded library cache: {} games", data.items.len());
            Some((data.items, data.purchase_dates))
        }
        Err(e) => {
            log::warn!("Discarding corrupt library cache: {e}");
            None
        }
    }
}

// The cache stores base library data only; live ratings and the derived
// search key are re-applied/rebuilt on load.
fn core_equal(a: &CatalogItem, b: &CatalogItem) -> bool {
    a.id == b.id
        && a.namespace == b.namespace
        && a.title == b.title
        && a.description == b.description
        && a.key_images == b.key_images
        && a.categories == b.categories
        && a.creation_date == b.creation_date
        && a.last_modified_date == b.last_modified_date
        && a.developer == b.developer
        && a.dlc_item_list == b.dlc_item_list
        && a.main_game_item == b.main_game_item
        && a.release_info == b.release_info
        && a.product_id == b.product_id
}

fn is_same(
    cached: &(Vec<CatalogItem>, HashMap<String, UtcDateTime>),
    fresh_items: &[CatalogItem],
    fresh_dates: &HashMap<String, UtcDateTime>,
) -> bool {
    if cached.0.len() != fresh_items.len() || cached.1 != *fresh_dates {
        return false;
    }
    cached
        .0
        .iter()
        .zip(fresh_items.iter())
        .all(|(a, b)| core_equal(a, b))
}

/// Write the fresh library to disk when it differs from what's cached.
/// Returns true when the file was rewritten.
pub fn save_if_changed(
    fresh_items: &[CatalogItem],
    fresh_dates: &HashMap<String, UtcDateTime>,
    path: &PathBuf,
) -> bool {
    if let Ok(bytes) = fs::read(path)
        && !bytes.is_empty()
        && let Ok(cached) = serde_json::from_slice::<LibraryCacheFile>(&bytes)
        && is_same(
            &(cached.items, cached.purchase_dates),
            fresh_items,
            fresh_dates,
        )
    {
        return false;
    }
    let file = LibraryCacheFile {
        items: fresh_items.to_vec(),
        purchase_dates: fresh_dates.clone(),
    };
    match serde_json::to_vec(&file) {
        Ok(bytes) => {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            match fs::write(path, bytes) {
                Ok(()) => {
                    log::info!("Saved library cache: {} games", fresh_items.len());
                    true
                }
                Err(e) => {
                    log::warn!("Failed to save library cache: {e:#}");
                    false
                }
            }
        }
        Err(e) => {
            log::warn!("Failed to serialize library cache: {e:#}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, title: &str) -> CatalogItem {
        CatalogItem {
            id: id.to_string(),
            namespace: "ns".to_string(),
            title: title.to_string(),
            description: String::new(),
            key_images: Vec::new(),
            categories: vec!["games".to_string()],
            creation_date: chrono::Utc::now(),
            last_modified_date: chrono::Utc::now(),
            developer: String::new(),
            dlc_item_list: None,
            main_game_item: None,
            release_info: Vec::new(),
            product_id: Some("prod".to_string()),
            critic: None,
            user_rating: None,
            search_key: crate::search::build_search_key(title),
        }
    }

    #[test]
    fn save_load_round_trip() {
        let dir = std::env::temp_dir().join("mythic-test-library");
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("library.json");

        let items = vec![item("a", "Game A"), item("b", "Game B")];
        let mut dates = HashMap::new();
        dates.insert("a".to_string(), chrono::Utc::now());
        assert!(save_if_changed(&items, &dates, &path));

        let (loaded_items, loaded_dates) = load(&path).expect("cache");
        assert!(is_same(&(loaded_items, loaded_dates), &items, &dates));
        // Same content again: no rewrite.
        assert!(!save_if_changed(&items, &dates, &path));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn changed_library_rewrites() {
        let dir = std::env::temp_dir().join("mythic-test-library-changed");
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("library.json");

        let items = vec![item("a", "Game A")];
        let dates = HashMap::new();
        assert!(save_if_changed(&items, &dates, &path));

        let mut grown = items.clone();
        grown.push(item("b", "Game B"));
        assert!(!is_same(&load(&path).expect("cache"), &grown, &dates));
        assert!(save_if_changed(&grown, &dates, &path));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn transient_fields_ignored() {
        let mut a = item("a", "Game A");
        let mut b = a.clone();
        b.search_key = "different".to_string();
        b.user_rating = Some(4.5);
        assert!(core_equal(&a, &b));
        a.title = "Renamed".to_string();
        assert!(!core_equal(&a, &b));
    }

    #[test]
    fn corrupt_cache_returns_none() {
        let dir = std::env::temp_dir().join("mythic-test-library-corrupt");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("library.json");
        fs::write(&path, b"not json").unwrap();
        assert!(load(&path).is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}

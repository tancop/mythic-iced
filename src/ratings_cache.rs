use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rkyv::{Archive, Deserialize, Serialize};

use crate::epic::{CatalogItem, CriticRating, CriticScore};

// Revalidate ratings in the background once the on-disk cache is older
// than this. The timestamp lives inside the cache file itself.
pub const STALE_AFTER_SECS: i64 = 24 * 60 * 60;

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn rating_to_u8(rating: CriticRating) -> u8 {
    match rating {
        CriticRating::Weak => 0,
        CriticRating::Fair => 1,
        CriticRating::Strong => 2,
        CriticRating::Mighty => 3,
    }
}

fn u8_to_rating(value: u8) -> Option<CriticRating> {
    match value {
        0 => Some(CriticRating::Weak),
        1 => Some(CriticRating::Fair),
        2 => Some(CriticRating::Strong),
        3 => Some(CriticRating::Mighty),
        _ => None,
    }
}

// Plain-data copy of `epic::CriticScore` so the cache file doesn't depend
// on the API types' layout.
#[derive(Archive, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CachedCritic {
    pub average: i32,
    pub recommend_percentage: i32,
    pub rating: u8,
    pub url: String,
}

impl CachedCritic {
    fn from_score(score: &CriticScore) -> Self {
        Self {
            average: score.average,
            recommend_percentage: score.recommend_percentage,
            rating: rating_to_u8(score.rating),
            url: score.url.clone(),
        }
    }

    fn to_score(&self) -> Option<CriticScore> {
        Some(CriticScore {
            average: self.average,
            rating: u8_to_rating(self.rating)?,
            recommend_percentage: self.recommend_percentage,
            url: self.url.clone(),
        })
    }
}

// Both ratings for one catalog id. Either side may be missing: entries
// with neither side are never stored, so unrated games are refetched on
// a later launch in case they gained a rating.
#[derive(Archive, Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct CachedRating {
    pub critic: Option<CachedCritic>,
    pub user_rating: Option<f32>,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
pub struct RatingsCacheData {
    pub saved_at: i64,
    pub entries: HashMap<String, CachedRating>,
}

pub struct RatingsCache {
    live: HashMap<String, CachedRating>,
    saved_at: i64,
    // Write coalescing: at most one background save runs at a time.
    // Arrivals during a save only set `save_pending`; the save completion
    // re-saves once if anything changed meanwhile.
    save_inflight: bool,
    save_pending: bool,
}

impl RatingsCache {
    pub fn load(path: &PathBuf) -> Self {
        match fs::read(path) {
            Ok(bytes) if !bytes.is_empty() => {
                match rkyv::from_bytes::<RatingsCacheData, rkyv::rancor::Error>(&bytes) {
                    Ok(data) => {
                        log::info!("Loaded ratings cache: {} games", data.entries.len());
                        Self {
                            live: data.entries,
                            saved_at: data.saved_at,
                            save_inflight: false,
                            save_pending: false,
                        }
                    }
                    Err(e) => {
                        log::warn!("Discarding corrupt ratings cache: {e}");
                        Self::empty()
                    }
                }
            }
            _ => Self::empty(),
        }
    }

    pub fn empty() -> Self {
        Self {
            live: HashMap::new(),
            saved_at: 0,
            save_inflight: false,
            save_pending: false,
        }
    }

    pub fn get(&self, id: &str) -> Option<&CachedRating> {
        self.live.get(id)
    }

    pub fn has_critic(&self, id: &str) -> bool {
        self.live
            .get(id)
            .is_some_and(|entry| entry.critic.is_some())
    }

    pub fn has_user_rating(&self, id: &str) -> bool {
        self.live
            .get(id)
            .is_some_and(|entry| entry.user_rating.is_some())
    }

    // Copy cached scores onto a freshly loaded item so ratings show
    // instantly without waiting on the network.
    pub fn apply(&self, item: &mut CatalogItem) {
        let Some(entry) = self.live.get(&item.id) else {
            return;
        };
        if let Some(critic) = entry.critic.as_ref().and_then(CachedCritic::to_score) {
            item.critic = Some(critic);
        }
        if let Some(rating) = entry.user_rating {
            item.user_rating = Some(rating);
        }
    }

    // Record a freshly fetched critic score. `None` (unrated or failed
    // fetch) leaves the cache untouched so the game is retried on a later
    // launch. Returns true when the caller should persist the cache.
    pub fn update_critic(&mut self, id: String, score: Option<&CriticScore>) -> bool {
        let Some(score) = score else {
            return false;
        };
        let cached = CachedCritic::from_score(score);
        let entry = self.live.entry(id).or_default();
        if entry.critic.as_ref() == Some(&cached) {
            return false;
        }
        entry.critic = Some(cached);
        true
    }

    // Same contract as `update_critic`, for the Epic user rating.
    pub fn update_user_rating(&mut self, id: String, rating: Option<f32>) -> bool {
        let Some(rating) = rating else {
            return false;
        };
        let entry = self.live.entry(id).or_default();
        if entry.user_rating == Some(rating) {
            return false;
        }
        entry.user_rating = Some(rating);
        true
    }

    pub fn is_stale(&self) -> bool {
        now_unix().saturating_sub(self.saved_at) > STALE_AFTER_SECS
    }

    pub fn save(&mut self, path: &PathBuf) -> anyhow::Result<()> {
        let data = self.snapshot().expect("save on idle cache");
        save_snapshot(&data, path)
    }

    // Copy out the current entries for a background save, stamping the
    // cache time. Returns `None` when a save is already running; the
    // arrival is remembered and picked up by `save_finished` instead, so a
    // full library load costs a couple of writes rather than one per game.
    pub fn snapshot(&mut self) -> Option<RatingsCacheData> {
        if self.save_inflight {
            self.save_pending = true;
            return None;
        }
        self.save_inflight = true;
        self.saved_at = now_unix();
        Some(RatingsCacheData {
            saved_at: self.saved_at,
            entries: self.live.clone(),
        })
    }

    // Called when a background save completes. Yields a fresh snapshot when
    // scores arrived during the save, `None` when the cache is persisted.
    pub fn save_finished(&mut self) -> Option<RatingsCacheData> {
        self.save_inflight = false;
        if self.save_pending {
            self.save_pending = false;
            return self.snapshot();
        }
        None
    }
}

pub fn cache_path() -> PathBuf {
    dirs::cache_dir().unwrap().join("mythic").join("ratings.db")
}

// Serialize + write one snapshot. Runs on a background task, never on the
// UI thread: a full-library rewrite per arriving score froze first loads.
pub fn save_snapshot(data: &RatingsCacheData, path: &PathBuf) -> anyhow::Result<()> {
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(data)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, &*bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(average: i32) -> CriticScore {
        CriticScore {
            average,
            rating: CriticRating::Mighty,
            recommend_percentage: 92,
            url: "https://opencritic.com/game/1/example".to_string(),
        }
    }

    #[test]
    fn none_updates_leave_cache_untouched() {
        let mut cache = RatingsCache::empty();
        assert!(!cache.update_critic("a".to_string(), None));
        assert!(!cache.update_user_rating("a".to_string(), None));
        assert!(cache.get("a").is_none());
    }

    #[test]
    fn some_updates_create_entries_and_report_change() {
        let mut cache = RatingsCache::empty();
        assert!(cache.update_critic("a".to_string(), Some(&score(89))));
        // Same value twice: no need to save again.
        assert!(!cache.update_critic("a".to_string(), Some(&score(89))));
        assert!(cache.has_critic("a"));
        assert!(!cache.has_user_rating("a"));

        assert!(cache.update_user_rating("a".to_string(), Some(4.5)));
        assert!(cache.has_user_rating("a"));
    }

    #[test]
    fn save_load_round_trip_preserves_both_ratings() {
        let path = std::env::temp_dir().join("mythic-test-ratings.db");
        let _ = fs::remove_file(&path);

        let mut cache = RatingsCache::empty();
        assert!(cache.update_critic("a".to_string(), Some(&score(89))));
        assert!(cache.update_user_rating("a".to_string(), Some(4.5)));
        // Unrated game stays out of the file.
        assert!(!cache.update_critic("b".to_string(), None));
        cache.save(&path).unwrap();

        let loaded = RatingsCache::load(&path);
        assert!(!loaded.is_stale());
        let entry = loaded.get("a").expect("entry");
        assert_eq!(
            entry.critic.as_ref().and_then(CachedCritic::to_score),
            Some(score(89))
        );
        assert_eq!(entry.user_rating, Some(4.5));
        assert!(loaded.get("b").is_none());

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn concurrent_arrivals_coalesce_to_one_in_flight_save() {
        let mut cache = RatingsCache::empty();
        cache.update_critic("a".to_string(), Some(&score(80)));
        // First snapshot starts the background save.
        let first = cache.snapshot();
        assert!(first.is_some());
        // Arrivals during the save don't start another one...
        cache.update_critic("b".to_string(), Some(&score(81)));
        assert!(cache.snapshot().is_none());
        // ...but the completion re-saves once to pick them up.
        let second = cache.save_finished();
        let second = second.expect("pending changes re-saved");
        assert!(second.entries.contains_key("b"));
        assert!(cache.save_finished().is_none());
        // A save round-trips through the coalesced snapshots.
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&second).unwrap();
        let back = rkyv::from_bytes::<RatingsCacheData, rkyv::rancor::Error>(&bytes).unwrap();
        assert_eq!(back.entries.len(), 2);
    }

    #[test]
    fn corrupt_or_missing_cache_loads_empty_and_stale() {
        let path = std::env::temp_dir().join("mythic-test-ratings-corrupt.db");
        fs::write(&path, b"not a cache file at all").unwrap();
        let loaded = RatingsCache::load(&path);
        assert!(loaded.get("a").is_none());
        assert!(loaded.is_stale());
        let _ = fs::remove_file(&path);

        let missing = RatingsCache::load(&PathBuf::from("definitely-not-here.db"));
        assert!(missing.is_stale());
    }

    #[test]
    fn old_timestamp_counts_as_stale() {
        let old = RatingsCache {
            live: HashMap::new(),
            saved_at: now_unix() - STALE_AFTER_SECS - 1,
            ..RatingsCache::empty()
        };
        assert!(old.is_stale());
    }

    #[test]
    fn unknown_rating_discriminant_yields_no_score() {
        let cached = CachedCritic {
            average: 80,
            recommend_percentage: 90,
            rating: 99,
            url: "https://example.com".to_string(),
        };
        assert!(cached.to_score().is_none());
    }
}

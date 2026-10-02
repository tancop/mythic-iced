use std::cmp::Ordering;
use std::fmt;

use iced::Task;

use crate::{Message, State, epic::CatalogItem};

pub const SORT_KEYS: [SortKey; 3] = [SortKey::ReleaseDate, SortKey::PurchaseDate, SortKey::Title];
pub const DLC_FILTERS: [FilterRule; 3] = [FilterRule::Allow, FilterRule::Only, FilterRule::Block];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortKey {
    ReleaseDate,
    PurchaseDate,
    #[default]
    Title,
    // Active while the search box is non-empty. Never offered in the
    // dropdown; the pick list just displays it as the current mode.
    Search,
}

impl SortKey {
    /// Direction-aware dropdown label, so the active direction is visible in
    /// the options themselves (e.g. "Title A-Z" vs "Title Z-A").
    pub fn label(self, reversed: bool) -> &'static str {
        match (self, reversed) {
            (SortKey::ReleaseDate, false) => "First released",
            (SortKey::ReleaseDate, true) => "Last released",
            (SortKey::PurchaseDate, false) => "First purchased",
            (SortKey::PurchaseDate, true) => "Last purchased",
            (SortKey::Title, false) => "Title A-Z",
            (SortKey::Title, true) => "Title Z-A",
            (SortKey::Search, _) => "Search",
        }
    }
}

/// A sort key paired with the current direction for the dropdown: `pick_list`
/// renders options via `Display`, so the direction has to be part of the
/// option value itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SortOption {
    pub key: SortKey,
    pub reversed: bool,
}

impl fmt::Display for SortOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key.label(self.reversed))
    }
}

/// Dropdown options reflecting the current direction.
pub fn sort_options(reversed: bool) -> [SortOption; 3] {
    SORT_KEYS.map(|key| SortOption { key, reversed })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FilterRule {
    #[default]
    Allow, // no preference
    Block, // only items that don't meet condition
    Only,  // only items that meet condition
}

impl FilterRule {
    // Returns true if the item should be included based on the condition and filter rule.
    pub fn allows(&self, condition: bool) -> bool {
        match self {
            FilterRule::Allow => true,
            FilterRule::Block => !condition,
            FilterRule::Only => condition,
        }
    }
}

impl fmt::Display for FilterRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FilterRule::Allow => write!(f, "Games + DLC"),
            FilterRule::Only => write!(f, "DLC only"),
            FilterRule::Block => write!(f, "Games only"),
        }
    }
}

pub fn is_included(state: &State, item: &CatalogItem) -> bool {
    if !state.filter_dlc.allows(item.is_dlc()) {
        return false;
    }

    true
}

// Indexes into `State::catalog_items` in display order (filtered, then
// sorted). The vec itself is never reordered; rebuild after every change.
pub fn rebuild_order(state: &mut State) {
    let order = match &state.catalog_items {
        None => Vec::new(),
        Some(items) => {
            let mut order: Vec<usize> = (0..items.len())
                .filter(|&i| is_included(state, &items[i]))
                .collect();
            order.sort_by(|&a, &b| compare(state, &items[a], &items[b]));
            if state.sort_reverse {
                order.reverse();
            }
            order
        }
    };
    state.order = order;
}

fn compare(state: &State, a: &CatalogItem, b: &CatalogItem) -> Ordering {
    match effective_sort_key(state) {
        SortKey::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
        SortKey::ReleaseDate => a.creation_date.cmp(&b.creation_date),
        SortKey::PurchaseDate => state
            .purchase_dates
            .get(&a.id)
            .cmp(&state.purchase_dates.get(&b.id)),
        // Best match first.
        SortKey::Search => relevance_score(state, b).total_cmp(&relevance_score(state, a)),
    }
}

// The search box overrides the dropdown selection while it holds text.
pub fn effective_sort_key(state: &State) -> SortKey {
    if state.search_query.is_empty() {
        state.sort_key
    } else {
        SortKey::Search
    }
}

/// Resets the tracked offset, drives the real scrollbar to the top, and
/// refreshes the visible cards.
fn scroll_and_refresh(state: &mut State) -> Task<Message> {
    state.scroll_offset = 0.0;
    Task::batch([
        crate::ui::virtual_grid::scroll_to_top(),
        crate::library::refresh_visible(state),
    ])
}

pub fn set_sort_key(state: &mut State, key: SortKey) -> Task<Message> {
    state.sort_key = key;
    // Searching is a sort (vs filter in Epic Store) so it should get replaced
    state.search_query.clear();
    rebuild_order(state);
    scroll_and_refresh(state)
}

pub fn set_sort_reverse(state: &mut State, reverse: bool) -> Task<Message> {
    state.sort_reverse = reverse;
    rebuild_order(state);
    scroll_and_refresh(state)
}

pub fn set_dlc_filter(state: &mut State, rule: FilterRule) -> Task<Message> {
    state.filter_dlc = rule;
    rebuild_order(state);
    scroll_and_refresh(state)
}

pub fn set_search_query(state: &mut State, query: String) -> Task<Message> {
    // Stored but not filtered on yet; rebuild keeps every toolbar control
    // refreshing the order, so real search only needs an is_included case.
    state.search_query = query;
    rebuild_order(state);
    scroll_and_refresh(state)
}

const WORD_BONUS_SCALE: f64 = 0.1;
const MIN_WORD_SCORE: f64 = 0.7;

const CLOSE_MATCH_BONUS: f64 = 0.067;
const CLOSE_MATCH_DISTANCE: usize = 3;

const NOISE_WORDS: &[&str] = &["and", "the", "of", "a", "an"];

/// Copyright / trademark symbols stripped from titles before searching.
const MARK_SYMBOLS: &[char] = &['©', '®', '™'];

/// Precompute the string search compares against: the lowercased title with
/// noise words, punctuation, and ©/®/™ stripped, words joined by single
/// spaces. Kept as one string; callers split on `' '` when scoring.
pub fn build_search_key(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .filter(|c| !MARK_SYMBOLS.contains(c))
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .filter(|word| !NOISE_WORDS.contains(word))
        .collect::<Vec<_>>()
        .join(" ")
}

// Higher means more similar (case-insensitive jaro-winkler); best matches
// sort first. Nothing is hidden by the query yet.
pub fn relevance_score(state: &State, item: &CatalogItem) -> f64 {
    // `search_key` is already lowercased with noise words stripped, so no
    // per-item normalization is needed here.
    let query = state.search_query.to_lowercase();

    // Base similarity
    let mut score = strsim::jaro_winkler(&item.search_key, &query);

    for word in item.search_key.split(' ') {
        // Per-word similarity
        let word_score = strsim::jaro_winkler(word, &query);
        if word_score >= MIN_WORD_SCORE {
            score += word_score * WORD_BONUS_SCALE
        }

        // Reward close match
        let distance = strsim::levenshtein(word, &query);
        if distance < CLOSE_MATCH_DISTANCE {
            score += (CLOSE_MATCH_DISTANCE - distance) as f64 * CLOSE_MATCH_BONUS;
        }
    }

    score
}

#[cfg(test)]
mod label_tests {
    use super::*;

    #[test]
    fn labels_flip_with_direction() {
        assert_eq!(SortKey::Title.label(false), "Title A-Z");
        assert_eq!(SortKey::Title.label(true), "Title Z-A");
        assert_eq!(SortKey::PurchaseDate.label(false), "First purchased");
        assert_eq!(SortKey::PurchaseDate.label(true), "Last purchased");
        assert_eq!(SortKey::ReleaseDate.label(false), "First released");
        assert_eq!(SortKey::ReleaseDate.label(true), "Last released");
        assert_eq!(SortKey::Search.label(false), "Search");
        assert_eq!(SortKey::Search.label(true), "Search");
    }

    #[test]
    fn sort_options_carry_direction() {
        let options = sort_options(true);
        assert_eq!(
            options.map(|o| o.to_string()),
            ["Last released", "Last purchased", "Title Z-A"]
        );
        assert!(options.iter().all(|o| o.reversed));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scored_item(title: &str) -> CatalogItem {
        CatalogItem {
            id: title.to_string(),
            namespace: String::new(),
            title: title.to_string(),
            description: String::new(),
            key_images: Vec::new(),
            categories: Vec::new(),
            creation_date: chrono::Utc::now(),
            last_modified_date: chrono::Utc::now(),
            developer: String::new(),
            dlc_item_list: None,
            main_game_item: None,
            release_info: Vec::new(),
            product_id: None,
            critic: None,
            search_key: build_search_key(title),
        }
    }

    #[test]
    fn search_key_normalizes_titles() {
        assert_eq!(
            build_search_key("Sid Meier's Civilization VI"),
            "sid meier s civilization vi"
        );
        assert_eq!(build_search_key("The Long Dark"), "long dark");
        assert_eq!(build_search_key("3 out of 10, EP 4"), "3 out 10 ep 4");
        assert_eq!(build_search_key("Rocket League®"), "rocket league");
        assert_eq!(build_search_key("© 2020 Game (Beta)!"), "2020 game beta");
        assert_eq!(build_search_key("  Spaced   Out  "), "spaced out");
        assert_eq!(build_search_key("Pokémon"), "pokémon");
        // Nothing left after noise-word removal.
        assert_eq!(build_search_key("The"), "");
        assert_eq!(build_search_key(""), "");
    }

    #[test]
    fn relevance_prefers_closer_search_key() {
        let mut state = State::default();
        state.search_query = "civ".to_string();

        let civ = scored_item("Sid Meier's Civilization VI");
        let torchlight = scored_item("Torchlight II");
        assert!(relevance_score(&state, &civ) > relevance_score(&state, &torchlight));
    }

    #[test]
    fn relevance_handles_empty_search_key() {
        let mut state = State::default();
        state.search_query = "the".to_string();

        let score = relevance_score(&state, &scored_item("The"));
        assert!(score.is_finite());
    }
}

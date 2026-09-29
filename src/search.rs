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

impl fmt::Display for SortKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SortKey::ReleaseDate => write!(f, "Release date"),
            SortKey::PurchaseDate => write!(f, "Purchase date"),
            SortKey::Title => write!(f, "Title A-Z"),
            SortKey::Search => write!(f, "Search"),
        }
    }
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

pub fn set_sort_key(state: &mut State, key: SortKey) -> Task<Message> {
    state.sort_key = key;
    rebuild_order(state);
    crate::library::refresh_visible(state)
}

pub fn set_sort_reverse(state: &mut State, reverse: bool) -> Task<Message> {
    state.sort_reverse = reverse;
    rebuild_order(state);
    crate::library::refresh_visible(state)
}

pub fn set_dlc_filter(state: &mut State, rule: FilterRule) -> Task<Message> {
    state.filter_dlc = rule;
    rebuild_order(state);
    crate::library::refresh_visible(state)
}

pub fn set_search_query(state: &mut State, query: String) -> Task<Message> {
    // Stored but not filtered on yet; rebuild keeps every toolbar control
    // refreshing the order, so real search only needs an is_included case.
    state.search_query = query;
    rebuild_order(state);
    crate::library::refresh_visible(state)
}

const WORD_BONUS_SCALE: f64 = 0.1;
const MIN_WORD_SCORE: f64 = 0.7;

const CLOSE_MATCH_BONUS: f64 = 0.067;
const CLOSE_MATCH_DISTANCE: usize = 3;

const NOISE_WORDS: &[&str] = &["and", "the", "of", "a", "an"];

// Higher means more similar (case-insensitive jaro-winkler); best matches
// sort first. Nothing is hidden by the query yet.
pub fn relevance_score(state: &State, item: &CatalogItem) -> f64 {
    let title = item.title.to_lowercase();
    let query = state.search_query.to_lowercase();

    // Base similarity
    let mut score = strsim::jaro_winkler(&title, &query);

    for word in title.split(' ') {
        // Exclude prepositions to avoid false match
        if NOISE_WORDS.contains(&word) {
            continue;
        }

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

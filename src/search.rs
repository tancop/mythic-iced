use crate::{State, epic::CatalogItem};

pub fn is_excluded(state: &State, item: &CatalogItem) -> bool {
    if state.filter_dlc && item.is_dlc() {
        false
    } else {
        true
    }
}

const MAX_EDIT_DISTANCE: usize = 3;
const WORD_SCORE_WEIGHT: f64 = 0.1;

// Lower score means the item's name is more similar to the query.
// Caps out at 1.0 and may be negative.
pub fn relevance_score(state: &State, name: &CatalogItem) -> f64 {
    let name = &name.title;
    let query = &state.search_query;

    let mut word_score = 0.0;

    for word in name.split(' ') {
        if word.len() == query.len() {
            if strsim::hamming(word, query).unwrap() < MAX_EDIT_DISTANCE {
                word_score += WORD_SCORE_WEIGHT;
            }
        }
    }

    strsim::jaro_winkler(name, query) - word_score
}

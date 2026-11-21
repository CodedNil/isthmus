use crate::platform::DesktopApp;
use std::mem::replace;

const MIN_CONFIDENCE: f32 = 0.6;

#[derive(Default)]
pub struct AppCatalog {
    pub apps: Vec<DesktopApp>,
    terms: Vec<Vec<Term>>,
}

struct Term {
    text: String,
    length: usize,
    weight: f32,
}

impl AppCatalog {
    pub fn new(mut apps: Vec<DesktopApp>) -> Self {
        apps.sort_by_cached_key(|app| (app.name.to_lowercase(), app.id.clone()));
        let terms = apps
            .iter()
            .map(|app| {
                [(app.name.as_str(), 1.0), (app.comment.as_str(), 0.9)]
                    .into_iter()
                    .chain(app.search_terms.iter().map(|text| (text.as_str(), 0.95)))
                    .filter(|(text, _)| !text.is_empty())
                    .map(|(text, weight)| {
                        let text = text.to_lowercase();
                        Term { length: text.chars().count(), text, weight }
                    })
                    .collect()
            })
            .collect();
        Self { apps, terms }
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<(usize, f32)> {
        let query = query.trim().to_lowercase();
        let length = query.chars().count();
        if length == 0 || limit == 0 {
            return Vec::new();
        }
        let mut scores: Vec<f32> = self
            .terms
            .iter()
            .map(|terms| {
                terms
                    .iter()
                    .map(|term| {
                        let coverage = length as f32 / term.length as f32;
                        let score = if term.text.starts_with(&query) {
                            0.8 + 0.2 * coverage
                        } else if term.text.contains(&query) {
                            MIN_CONFIDENCE + 0.2 * coverage
                        } else {
                            0.0
                        };
                        score * term.weight
                    })
                    .fold(0.0, f32::max)
            })
            .collect();
        // Fuzzy matches fill leftover rows; the length gap bounds edit distance, so most skip scoring.
        if scores.iter().filter(|&&score| score >= MIN_CONFIDENCE).count() < limit {
            for (score, terms) in scores.iter_mut().zip(&self.terms).filter(|(score, _)| **score < MIN_CONFIDENCE) {
                for term in terms {
                    let best = length.min(term.length) as f32 / length.max(term.length) as f32 * term.weight;
                    if best >= MIN_CONFIDENCE {
                        *score = score.max(similarity(&query, &term.text) * term.weight);
                    }
                }
            }
        }
        let mut matches: Vec<_> =
            scores.into_iter().enumerate().filter(|&(_, score)| score >= MIN_CONFIDENCE).collect();
        matches.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        matches.truncate(limit);
        matches
    }
}

/// Unicode scalar similarity, from 0 for no match to 1 for identical nonempty strings.
pub fn similarity(a: &str, b: &str) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let (a_len, b_len) = (a.chars().count(), b.chars().count());
    let (short, long) = if a_len <= b_len { (a, b) } else { (b, a) };
    let mut costs: Vec<_> = (1..=a_len.min(b_len)).collect();
    for (row, left) in long.chars().enumerate() {
        let mut diagonal = row;
        let mut beside = row + 1;
        for (above, right) in costs.iter_mut().zip(short.chars()) {
            beside = (diagonal + usize::from(left != right)).min(1 + beside.min(*above));
            diagonal = replace(above, beside);
        }
    }
    1.0 - costs[costs.len() - 1] as f32 / a_len.max(b_len) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, name: &str) -> DesktopApp {
        DesktopApp { id: id.into(), name: name.into(), exec: vec![id.into()], ..Default::default() }
    }

    #[test]
    fn searches_names_aliases_unicode_and_typos() {
        let mut editor = app("gimp", "GNU Image Editor");
        editor.search_terms = vec!["org.gimp.GIMP".into(), "photo".into()];
        let catalog = AppCatalog::new(vec![editor, app("steam", "Steam"), app("notes", "Écriture")]);
        for query in ["stea", "STEAM", "  stea  ", "stema"] {
            let results = catalog.search(query, 8);
            assert_eq!(catalog.apps[results[0].0].id, "steam", "{query}");
        }
        for query in ["gimp", "photo", "org.gimp", "GNU Image Editor"] {
            let results = catalog.search(query, 8);
            assert_eq!(catalog.apps[results[0].0].id, "gimp", "{query}");
        }
        assert_eq!(catalog.apps[catalog.search("écri", 8)[0].0].id, "notes");
        assert_eq!(catalog.search("unrelated", 8), []);
        assert_eq!(catalog.search("", 8), []);
    }
}

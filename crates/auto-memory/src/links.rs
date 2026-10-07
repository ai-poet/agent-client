// Translated from dsh-auto-memory (MIT, Copyright (c) 2026 AskTheWay); see NOTICE.md.

//! `[[name]]` cross-links: one level, at most [`LINK_LIMIT`], self-links and
//! broken links skipped.

use std::sync::LazyLock;

use regex::Regex;

/// Links resolved per read.
pub const LINK_LIMIT: usize = 3;

static LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\[([a-z0-9]+(?:-[a-z0-9]+)*)\]\]").expect("link pattern"));

/// A linked memory's summary, appended to a read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedSummary {
    pub name: String,
    pub description: String,
}

/// Resolve the links in `body` through `lookup` (name → description).
/// Returns the summaries and whether some links were left out.
pub fn expand_links(
    body: &str,
    self_name: &str,
    mut lookup: impl FnMut(&str) -> Option<String>,
) -> (Vec<LinkedSummary>, bool) {
    let mut names: Vec<&str> = Vec::new();
    for captures in LINK.captures_iter(body) {
        let name = captures.get(1).map_or("", |m| m.as_str());
        if name != self_name && !names.contains(&name) {
            names.push(name);
        }
    }
    let mut linked = Vec::new();
    for name in &names {
        if linked.len() >= LINK_LIMIT {
            break;
        }
        if let Some(description) = lookup(name) {
            linked.push(LinkedSummary {
                name: (*name).to_owned(),
                description,
            });
        }
    }
    let truncated = names.len() > linked.len();
    (linked, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(name: &str) -> Option<String> {
        name.starts_with('k').then(|| format!("about {name}"))
    }

    #[test]
    fn expands_one_level_skipping_self_broken_and_duplicates() {
        let (linked, truncated) =
            expand_links("[[k-a]] [[self]] [[missing]] [[k-a]] [[k-b]]", "self", known);
        assert_eq!(
            linked.iter().map(|link| link.name.as_str()).collect::<Vec<_>>(),
            vec!["k-a", "k-b"]
        );
        assert!(truncated, "the broken link counts as left out");
    }

    #[test]
    fn stops_at_the_limit() {
        let body = (0..6).map(|i| format!("[[k-{i}]]")).collect::<Vec<_>>().join(" ");
        let (linked, truncated) = expand_links(&body, "x", known);
        assert_eq!(linked.len(), LINK_LIMIT);
        assert!(truncated);
    }

    #[test]
    fn ignores_malformed_links() {
        let (linked, truncated) = expand_links("[[Not-Kebab]] [[k_bad]] [k-one]", "x", known);
        assert!(linked.is_empty());
        assert!(!truncated);
    }
}

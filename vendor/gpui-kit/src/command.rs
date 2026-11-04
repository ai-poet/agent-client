//! Selection and grouping helpers shared by command-palette-like components.

/// Restore a selected value after a result list is rebuilt, falling back to the
/// caller's existing default when the value disappeared.
pub fn restore_selected_index<T: Eq>(items: &[T], selected: Option<&T>, default: usize) -> usize {
    restore_selected_index_by(items, selected, default, |item| item)
}

/// [`restore_selected_index`] for lists whose item and selection key differ.
pub fn restore_selected_index_by<T, K: Eq>(
    items: &[T],
    selected: Option<&K>,
    default: usize,
    key: impl Fn(&T) -> &K,
) -> usize {
    selected
        .and_then(|selected| items.iter().position(|item| key(item) == selected))
        .unwrap_or(default)
}

/// Convert a selected item index into a list row index by counting one header
/// for each new section. Invalid and empty selections safely return zero.
pub fn grouped_scroll_index<T: Eq>(sections: &[T], selected: usize) -> usize {
    grouped_scroll_index_by(sections, selected, |section| section, |_| true)
}

/// [`grouped_scroll_index`] with caller-owned section projection and a rule for
/// sections that do not render a header.
pub fn grouped_scroll_index_by<'a, T, S: Eq>(
    items: &'a [T],
    selected: usize,
    section: impl Fn(&'a T) -> S,
    mut has_header: impl FnMut(&S) -> bool,
) -> usize {
    if items.is_empty() {
        return 0;
    }
    let selected = selected.min(items.len() - 1);
    let mut headers = 0;
    let mut previous = None;
    for item in items.iter().take(selected + 1) {
        let section = section(item);
        if previous.as_ref() != Some(&section) {
            if has_header(&section) {
                headers += 1;
            }
            previous = Some(section);
        }
    }
    selected + headers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_follows_identity_across_reordering_and_insertion() {
        let items = ["new", "selected", "other"];
        assert_eq!(restore_selected_index(&items, Some(&"selected"), 0), 1);
        let inserted = ["provider", "new", "selected"];
        assert_eq!(restore_selected_index(&inserted, Some(&"selected"), 0), 2);
        assert_eq!(restore_selected_index(&inserted, Some(&"missing"), 1), 1);
        assert_eq!(restore_selected_index(&inserted, None, 0), 0);
    }

    #[test]
    fn scroll_index_counts_each_section_header_once() {
        let sections = ["tasks", "tasks", "commands", "settings"];
        assert_eq!(grouped_scroll_index(&sections, 0), 1);
        assert_eq!(grouped_scroll_index(&sections, 1), 2);
        assert_eq!(grouped_scroll_index(&sections, 2), 4);
        assert_eq!(grouped_scroll_index(&sections, 3), 6);
        assert_eq!(grouped_scroll_index::<&str>(&[], 3), 0);
        assert_eq!(
            grouped_scroll_index_by(
                &sections,
                3,
                |section| *section,
                |section| { *section != "providers" }
            ),
            6
        );
        let without_provider_header = ["tasks", "providers", "settings"];
        assert_eq!(
            grouped_scroll_index_by(
                &without_provider_header,
                2,
                |section| *section,
                |section| *section != "providers"
            ),
            4
        );
    }
}

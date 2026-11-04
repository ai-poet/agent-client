//! Pure diff selection/highlight helpers adapted from Longbridge GPUI Kit
//! (Apache-2.0). They operate without GPUI Kit's document, theme or Root types.

use std::{collections::BTreeSet, ops::Range};

use gpui::{Bounds, HighlightStyle, Pixels, Point};

/// Clip a highlight to a rendered chunk and rebase it to chunk-local offsets.
pub fn clip_highlight(range: Range<usize>, chunk: &Range<usize>) -> Option<Range<usize>> {
    let start = range.start.max(chunk.start);
    let end = range.end.min(chunk.end);
    (start < end).then(|| start - chunk.start..end - chunk.start)
}

/// Prefer the half-open line band before proximity for multi-click endpoints.
pub fn line_distance(point: Point<Pixels>, bounds: Bounds<Pixels>) -> (bool, u32) {
    let outside = point.y < bounds.top() || point.y >= bounds.bottom();
    let distance: f32 = (point.y - bounds.center().y).abs().into();
    (outside, distance.to_bits())
}

/// Bounded native-like word selection across rendering chunks.
pub fn word_range(text: &str, mut offset: usize) -> Option<Range<usize>> {
    offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let character = text[offset..].chars().next()?;
    let class = |character: char| {
        if character == '_'
            || character.is_ascii_alphanumeric()
            || matches!(character, '\u{00C0}'..='\u{024F}' | '\u{0400}'..='\u{04FF}' | '\u{1E00}'..='\u{1EFF}' | '\u{0300}'..='\u{036F}')
        {
            1
        } else if matches!(character, '\n' | '\r') {
            0
        } else if character.is_whitespace() {
            2
        } else {
            0
        }
    };
    let kind = class(character);
    let connects = |character: &char| kind != 0 && class(*character) == kind;
    let start = text[..offset]
        .chars()
        .rev()
        .take(128)
        .take_while(connects)
        .fold(offset, |offset, character| offset - character.len_utf8());
    let end = offset + character.len_utf8();
    let end = text[end..]
        .chars()
        .take(128)
        .take_while(connects)
        .fold(end, |offset, character| offset + character.len_utf8());
    Some(start..end)
}

/// Merge highlight layers with later layers winning independently per field.
pub fn merge_highlights(
    len: usize,
    layers: Vec<(Range<usize>, HighlightStyle)>,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut events = Vec::with_capacity(layers.len() * 2 + 2);
    for (ix, (range, _)) in layers.iter().enumerate() {
        let start = range.start.min(len);
        let end = range.end.min(len);
        if start < end {
            events.push((start, ix, true));
            events.push((end, ix, false));
        }
    }
    events.sort_unstable();
    let mut active: [BTreeSet<usize>; 7] = Default::default();
    let mut output = Vec::with_capacity(events.len() + 1);
    let mut position = 0;
    let mut event_ix = 0;
    while position < len {
        while event_ix < events.len() && events[event_ix].0 == position {
            let (_, ix, entering) = events[event_ix];
            let style = &layers[ix].1;
            let specified = [
                style.color.is_some(),
                style.background_color.is_some(),
                style.font_weight.is_some(),
                style.font_style.is_some(),
                style.underline.is_some(),
                style.strikethrough.is_some(),
                style.fade_out.is_some(),
            ];
            for (field, specified) in specified.into_iter().enumerate() {
                if specified {
                    if entering {
                        active[field].insert(ix);
                    } else {
                        active[field].remove(&ix);
                    }
                }
            }
            event_ix += 1;
        }
        let end = events.get(event_ix).map_or(len, |event| event.0);
        let mut style = HighlightStyle::default();
        macro_rules! resolve {
            ($field:ident, $ix:expr) => {
                if let Some(ix) = active[$ix].last() {
                    style.$field = layers[*ix].1.$field;
                }
            };
        }
        resolve!(color, 0);
        resolve!(background_color, 1);
        resolve!(font_weight, 2);
        resolve!(font_style, 3);
        resolve!(underline, 4);
        resolve!(strikethrough, 5);
        resolve!(fade_out, 6);
        output.push((position..end, style));
        position = end;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px, rgb, size};

    #[test]
    fn chunks_clip_and_rebase_without_adjacent_overlap() {
        assert_eq!(clip_highlight(4..12, &(8..16)), Some(0..4));
        assert_eq!(clip_highlight(12..20, &(8..16)), Some(4..8));
        assert_eq!(clip_highlight(8..16, &(8..16)), Some(0..8));
        assert_eq!(clip_highlight(4..8, &(8..16)), None);
        assert_eq!(clip_highlight(16..20, &(8..16)), None);
    }

    #[test]
    fn overlapping_layers_preserve_syntax_and_field_precedence() {
        let syntax = rgb(0x336699).into();
        let inline = rgb(0x884422).into();
        let selection = rgb(0x225588).into();
        let highlights = merge_highlights(
            8,
            vec![
                (
                    0..8,
                    HighlightStyle {
                        color: Some(syntax),
                        ..Default::default()
                    },
                ),
                (
                    2..7,
                    HighlightStyle {
                        background_color: Some(inline),
                        ..Default::default()
                    },
                ),
                (
                    4..6,
                    HighlightStyle {
                        background_color: Some(selection),
                        ..Default::default()
                    },
                ),
            ],
        );
        assert_eq!(
            highlights
                .iter()
                .map(|(range, _)| range.clone())
                .collect::<Vec<_>>(),
            vec![0..2, 2..4, 4..6, 6..7, 7..8]
        );
        assert!(
            highlights
                .iter()
                .all(|(_, style)| style.color == Some(syntax))
        );
        assert_eq!(
            highlights
                .iter()
                .map(|(_, style)| style.background_color)
                .collect::<Vec<_>>(),
            vec![None, Some(inline), Some(selection), Some(inline), None]
        );
        assert!(merge_highlights(0, vec![]).is_empty());
    }

    #[test]
    fn word_selection_respects_unicode_and_flank_bounds() {
        let token = "a".repeat(1024);
        assert_eq!(word_range(&token, 512), Some(384..641));
        assert_eq!(word_range("  foo_bar  !", 4), Some(2..9));
        assert_eq!(word_range("  foo_bar  !", 9), Some(9..11));
        assert_eq!(word_range("éяa\u{0301}界", 2), Some(0..7));
    }

    #[test]
    fn line_top_prefers_current_line_over_previous_bottom() {
        let previous = Bounds::new(point(px(20.0), px(30.0)), size(px(100.0), px(20.0)));
        let current = Bounds::new(point(px(20.0), px(50.0)), size(px(100.0), px(20.0)));
        let endpoint = point(px(40.0), px(50.0));
        assert!(line_distance(endpoint, current) < line_distance(endpoint, previous));
    }
}

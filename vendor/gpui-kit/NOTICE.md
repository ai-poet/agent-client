# Third-party notices

## Longbridge GPUI Kit

Source: <https://github.com/longbridge/gpui-kit>
Upstream revision: `0ab9cf4ed6d237457e66efc56c8ffcf915e8ce05`
License: Apache-2.0; the license text is preserved in `LICENSE-LONGBRIDGE`.

Closely adapted sources:

- `src/message_scroller.rs`: `MessageScrollerState`, based on
  `crates/component/src/message_scroller.rs`. GPUI Kit's styled presentation,
  motion and button dependencies are intentionally excluded; Waku keeps its own
  transcript presentation and scroll policy.
- `src/diff.rs`: `clip_highlight`, `line_distance`, `word_range`,
  `merge_highlights` and their tests, adapted from
  `crates/component/src/diff/selection.rs` and `selection_tests.rs`. The port
  uses Waku's vendored GPUI types and excludes GPUI Kit's document, theme and
  Root integration.

`src/command.rs` implements Waku-owned selection/grouping helpers after reviewing
GPUI Kit's Command behavior; it does not contain a close source translation.

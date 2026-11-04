use gpui::{App, Pixels, ScrollHandle, px};

/// Keep a wheel gesture in a nested scrollable while it can consume the delta,
/// then chain it to the ancestor at either boundary.
pub fn contain_scroll(handle: &ScrollHandle, cx: &mut App) {
    if nested_scroll_consumed_delta(handle.offset().y, handle.max_offset().y) {
        cx.stop_propagation();
    }
}

fn nested_scroll_consumed_delta(offset: Pixels, max_offset: Pixels) -> bool {
    max_offset > px(0.5) && offset >= -max_offset && offset <= px(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_scroll_chains_only_after_reaching_a_boundary() {
        let max_offset = px(100.0);
        assert!(nested_scroll_consumed_delta(px(0.0), max_offset));
        assert!(nested_scroll_consumed_delta(px(-50.0), max_offset));
        assert!(nested_scroll_consumed_delta(px(-100.0), max_offset));
        assert!(!nested_scroll_consumed_delta(px(1.0), max_offset));
        assert!(!nested_scroll_consumed_delta(px(-101.0), max_offset));
        assert!(!nested_scroll_consumed_delta(px(-50.0), Pixels::ZERO));
    }
}

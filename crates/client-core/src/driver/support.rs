//! Helpers shared by the built-in driver.

/// Computer Use is optional: a helper/setup failure disables desktop control
/// for the session rather than preventing the session from starting.
pub(super) fn optional_computer_use<T>(setup: anyhow::Result<T>) -> Option<T> {
    setup
        .inspect_err(|error| {
            eprintln!("warning: desktop control is off for this session: {error:#}");
        })
        .ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn optional_computer_use_degrades_to_none() {
        let missing: anyhow::Result<()> = Err(anyhow::anyhow!("missing"));
        assert_eq!(super::optional_computer_use(missing), None);
        assert_eq!(super::optional_computer_use(anyhow::Ok(42)), Some(42));
    }
}

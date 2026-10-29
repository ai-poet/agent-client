//! Which external links the app hands to the operating system.
//!
//! Links in a transcript, a rendered document or a terminal are written by an
//! agent, or by whatever page or file the agent read, so a click must never
//! reach an arbitrary URL-scheme handler: `ms-msdt:`, `vscode:`,
//! `x-apple.systempreferences:` and their kind launch other programs with
//! arguments the link chose. Only explicit web addresses and mail drafts pass,
//! and nothing is repaired on the way through — an address with controls,
//! credentials, backslashes or malformed escapes is refused, not cleaned up.
//!
//! The rule is translated from Zeron's `transcript_address` /
//! `allowed_navigation` (crates/ui/src/browser/model.rs; MIT, Copyright (c)
//! 2026 Wing), with `mailto:` added.

use gpui::App;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LinkRejection {
    /// Not an explicit `http(s)://` or `mailto:` address.
    NotWeb,
    /// Controls, whitespace, backslashes, a missing host or a bad escape.
    Invalid,
    /// A username or password embedded in the address.
    Credentials,
}

impl LinkRejection {
    pub(crate) fn message(self) -> String {
        match self {
            Self::NotWeb => tr!("errors.link_not_web"),
            Self::Invalid => tr!("errors.link_invalid"),
            Self::Credentials => tr!("errors.link_credentials"),
        }
    }
}

/// The address to open for `input`, or why it is refused.
pub(crate) fn checked_external_link(input: &str) -> Result<String, LinkRejection> {
    if input.chars().any(|c| c.is_control() || c.is_whitespace()) || input.contains('\\') {
        return Err(LinkRejection::Invalid);
    }
    if !escapes_are_well_formed(input) {
        return Err(LinkRejection::Invalid);
    }

    let lower = input.to_ascii_lowercase();
    if let Some(authority) = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
    {
        if authority.is_empty() || authority.starts_with(['/', '?', '#']) {
            return Err(LinkRejection::Invalid);
        }
        let parsed = url::Url::parse(input).map_err(|_| LinkRejection::Invalid)?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
            return Err(LinkRejection::Invalid);
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(LinkRejection::Credentials);
        }
        return Ok(parsed.into());
    }

    if let Some(recipient) = lower.strip_prefix("mailto:") {
        if recipient.is_empty() || recipient.starts_with(['?', '#']) {
            return Err(LinkRejection::Invalid);
        }
        let parsed = url::Url::parse(input).map_err(|_| LinkRejection::Invalid)?;
        return Ok(parsed.into());
    }

    Err(LinkRejection::NotWeb)
}

/// Open `input` if the policy allows it.
pub(crate) fn open_checked(input: &str, cx: &mut App) -> Result<(), LinkRejection> {
    let address = checked_external_link(input)?;
    cx.open_url(&address);
    Ok(())
}

/// Open `input` if the policy allows it, logging a refusal. For surfaces with
/// no place to say why (documents without a link handler, the terminal).
pub(crate) fn open_checked_or_log(input: &str, cx: &mut App) {
    if let Err(reason) = open_checked(input, cx) {
        eprintln!(
            "refused to open link ({reason:?}): {}",
            input.escape_debug()
        );
    }
}

fn escapes_are_well_formed(input: &str) -> bool {
    let bytes = input.as_bytes();
    bytes.iter().enumerate().all(|(index, byte)| {
        *byte != b'%'
            || bytes
                .get(index + 1..index + 3)
                .is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_and_mail_addresses_pass() {
        assert_eq!(
            checked_external_link("https://example.com/docs?q=1#top"),
            Ok("https://example.com/docs?q=1#top".to_owned())
        );
        assert_eq!(
            checked_external_link("HTTPS://Example.com"),
            Ok("https://example.com/".to_owned())
        );
        assert_eq!(
            checked_external_link("http://localhost:3000/a%20b"),
            Ok("http://localhost:3000/a%20b".to_owned())
        );
        assert_eq!(
            checked_external_link("mailto:support@example.com?subject=Hi"),
            Ok("mailto:support@example.com?subject=Hi".to_owned())
        );
    }

    #[test]
    fn other_schemes_are_refused() {
        for input in [
            "javascript:alert(1)",
            "vscode://file/etc/passwd",
            "ms-msdt:/id%20PCWDiagnostic",
            "x-apple.systempreferences:com.apple.preference.security",
            "data:text/html,hi",
            "file:///etc/passwd",
            "ftp://example.com",
            "example.com",
            "www.example.com",
        ] {
            assert_eq!(
                checked_external_link(input),
                Err(LinkRejection::NotWeb),
                "{input}"
            );
        }
    }

    #[test]
    fn malformed_addresses_are_refused_not_repaired() {
        for input in [
            "http:///x",
            "https://",
            "https://?q",
            "https://#frag",
            "https://exa mple.com",
            "https://example.com/\u{0007}",
            "https://example.com\\@evil.test",
            "https://example.com/%zz",
            "https://example.com/%4",
            "mailto:",
            "mailto:?subject=hi",
        ] {
            assert_eq!(
                checked_external_link(input),
                Err(LinkRejection::Invalid),
                "{input}"
            );
        }
    }

    #[test]
    fn embedded_credentials_are_refused() {
        assert_eq!(
            checked_external_link("http://user:pass@example.com"),
            Err(LinkRejection::Credentials)
        );
        assert_eq!(
            checked_external_link("https://user@example.com"),
            Err(LinkRejection::Credentials)
        );
    }
}

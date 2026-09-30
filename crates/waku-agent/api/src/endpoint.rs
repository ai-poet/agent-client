//! Where a request goes on a configured base URL.
//!
//! Fork departure (Waku). The adapters built every URL as
//! `<base>/v1/<path>`, which is right for a bare origin and for every relay
//! that copies OpenAI's layout — and wrong for the providers that version
//! their API somewhere else: Zhipu's `https://open.bigmodel.cn/api/paas/v4`
//! became `…/v4/v1/chat/completions` and answered 404, which the engine then
//! reported as "Model not found". A base whose last segment already names a
//! version is taken as the version root; anything else gets `/v1` as before.
//!
//! Only the last segment counts, so a path that merely contains `/v2/`
//! somewhere keeps the old behaviour. The desktop mirrors this rule in
//! `sub2api::gateway::versioned_url`; keep the two in step.

/// `<base>/<path>` when `base` already ends in a version segment, else
/// `<base>/v1/<path>`. Trailing slashes on `base` and leading ones on `path`
/// are ignored.
pub fn versioned_url(base: &str, path: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    let path = path.trim_start_matches('/');
    if ends_with_version_segment(base) {
        format!("{base}/{path}")
    } else {
        format!("{base}/v1/{path}")
    }
}

/// Whether the last path segment of `base` is an API version: `v` and
/// digits, optionally followed by `alpha` or `beta` and more digits (`v1`,
/// `v4`, `v1beta`, `v2alpha1`). A bare origin has no path and answers false.
pub fn ends_with_version_segment(base: &str) -> bool {
    let base = base.trim().trim_end_matches('/');
    let after_scheme = base.split_once("://").map_or(base, |(_, rest)| rest);
    let Some((_, path)) = after_scheme.split_once('/') else {
        return false;
    };
    let Some(segment) = path.rsplit('/').next() else {
        return false;
    };
    is_version_segment(segment)
}

fn is_version_segment(segment: &str) -> bool {
    let segment = segment.to_ascii_lowercase();
    let Some(rest) = segment.strip_prefix('v') else {
        return false;
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return false;
    }
    let rest = &rest[digits..];
    if rest.is_empty() {
        return true;
    }
    let Some(tail) = rest
        .strip_prefix("alpha")
        .or_else(|| rest.strip_prefix("beta"))
    else {
        return false;
    };
    tail.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_origin_gets_v1() {
        assert_eq!(
            versioned_url("https://gateway.example.org", "chat/completions"),
            "https://gateway.example.org/v1/chat/completions"
        );
        assert_eq!(
            versioned_url("http://127.0.0.1:8080/", "/messages"),
            "http://127.0.0.1:8080/v1/messages"
        );
    }

    #[test]
    fn a_v1_base_is_not_doubled() {
        assert_eq!(
            versioned_url("https://relay.example.org/v1", "responses"),
            "https://relay.example.org/v1/responses"
        );
        assert_eq!(
            versioned_url("https://relay.example.org/openai/v1/", "models"),
            "https://relay.example.org/openai/v1/models"
        );
    }

    #[test]
    fn a_versioned_base_keeps_its_version() {
        assert_eq!(
            versioned_url("https://open.bigmodel.cn/api/paas/v4", "chat/completions"),
            "https://open.bigmodel.cn/api/paas/v4/chat/completions"
        );
        assert_eq!(
            versioned_url("https://ark.cn-beijing.volces.com/api/v3", "chat/completions"),
            "https://ark.cn-beijing.volces.com/api/v3/chat/completions"
        );
        assert_eq!(
            versioned_url("https://example.org/V1BETA", "models"),
            "https://example.org/V1BETA/models"
        );
        assert!(ends_with_version_segment("https://example.org/api/v2alpha1"));
    }

    #[test]
    fn a_word_starting_with_v_is_not_a_version() {
        for base in [
            "https://example.org/v2proxy",
            "https://example.org/video",
            "https://example.org/api",
            "https://example.org/v",
            "https://example.org/v1/anthropic",
            "https://v1.example.org",
            "example.org:8443",
        ] {
            assert!(!ends_with_version_segment(base), "{base}");
        }
    }
}

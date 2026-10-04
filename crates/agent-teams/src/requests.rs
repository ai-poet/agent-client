//! The id of a permission dialog a team member raised.
//!
//! A member's dialog surfaces in its captain's session, which may have no
//! turn running when it arrives. The id says so — `team:<uuid>:<member>` —
//! so the desktop can show it outside a turn and keep it across the
//! captain's turn ends, and the driver can name the member asking.

const PREFIX: &str = "team:";

/// A fresh request id for a dialog `member` raised.
pub fn permission_request_id(member: &str) -> String {
    format!("{PREFIX}{}:{member}", uuid::Uuid::new_v4())
}

/// Whether a dialog came from a team member.
pub fn is_member_request(request_id: &str) -> bool {
    request_id.starts_with(PREFIX)
}

/// The member that raised a dialog.
pub fn requesting_member(request_id: &str) -> Option<&str> {
    request_id
        .strip_prefix(PREFIX)?
        .split_once(':')
        .map(|(_, member)| member)
        .filter(|member| !member.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_names_with_colons() {
        let id = permission_request_id("dev: backend");
        assert!(is_member_request(&id));
        assert_eq!(requesting_member(&id), Some("dev: backend"));
        assert!(!is_member_request("3f2a-…"));
        assert_eq!(requesting_member("team:"), None);
    }
}

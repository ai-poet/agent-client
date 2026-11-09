//! UTF-8 and command-output helpers copied from Sailry's process/output.rs.
pub mod output {
    pub fn edges(bytes: &[u8], head: usize, tail: usize) -> (usize, usize) {
        let mut head = head.min(bytes.len());
        while head > 0 && bytes.get(head).is_some_and(|byte| byte & 0xc0 == 0x80) {
            head -= 1;
        }
        if let Some(start) = (head.saturating_sub(3)..head)
            .rev()
            .find(|index| bytes[*index] & 0xc0 != 0x80)
        {
            let width = match bytes[start] {
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf4 => 4,
                _ => 1,
            };
            if head - start < width {
                head = start;
            }
        }
        let mut tail = bytes.len().saturating_sub(tail).max(head);
        while bytes.get(tail).is_some_and(|byte| byte & 0xc0 == 0x80) {
            tail += 1;
        }
        (head, tail)
    }

    pub fn utf8_page(
        bytes: &[u8],
        offset: usize,
        limit: usize,
    ) -> Result<std::ops::Range<usize>, sailry_protocol::Fault> {
        if offset > bytes.len() || limit == 0 {
            return Err(sailry_protocol::Fault::invalid("output page is invalid"));
        }
        if offset > 0 && bytes.get(offset).is_some_and(|byte| byte & 0xc0 == 0x80) {
            return Err(sailry_protocol::Fault::invalid(
                "output page offset splits a UTF-8 character",
            ));
        }
        let mut end = offset.saturating_add(limit).min(bytes.len());
        while end > offset && bytes.get(end).is_some_and(|byte| byte & 0xc0 == 0x80) {
            end -= 1;
        }
        if end == offset && end < bytes.len() {
            end += 1;
            while bytes.get(end).is_some_and(|byte| byte & 0xc0 == 0x80) {
                end += 1;
            }
        }
        Ok(offset..end)
    }

    pub fn command_id(value: &serde_json::Value) -> Option<&str> {
        value
            .pointer("/sailry_command/id")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                matches!(
                    value["kind"].as_str(),
                    Some("command_result" | "command_output")
                )
                .then(|| {
                    value
                        .pointer("/data/info/id")
                        .or_else(|| value.pointer("/info/id"))
                })
                .flatten()
                .and_then(serde_json::Value::as_str)
            })
    }
}

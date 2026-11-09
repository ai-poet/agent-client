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

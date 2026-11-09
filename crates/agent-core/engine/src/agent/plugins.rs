pub mod search {
    use serde_json::Value;

    pub const NAME: &str = "tool_search";

    pub fn discovered(output: &Value) -> impl Iterator<Item = &str> {
        output["tools"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|tool| tool["name"].as_str())
    }
}

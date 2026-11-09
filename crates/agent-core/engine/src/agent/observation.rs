//! Observation hook retained for the original remaining-tool operation.
use sailry_protocol::tool::Failure;

#[derive(Default)]
pub struct Timeline;

impl Timeline {
    pub fn fail_tool(&self, _: &str, _: &str, _: &str, _: Failure, _: Option<String>) {}
}

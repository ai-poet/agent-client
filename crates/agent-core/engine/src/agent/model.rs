//! Sailry's model contract and request-scoped recovery, without its Node factory.

pub mod observation;
pub mod request;
pub mod retry;
pub mod summary;
pub mod types;

pub struct Binding {
    pub provider: Arc<dyn Model>,
    pub config: Config,
    pub owner: String,
}

pub use sailry_protocol::Effort;
use sailry_protocol::{SessionConfig, conversation::Provider};
use std::sync::Arc;

pub use types::{
    Config, Content, Error, ErrorKind, Event, Finish, Message, Model, ModelStream, Request, Role,
    Tool, Usage,
};

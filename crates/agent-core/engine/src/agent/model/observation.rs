use sailry_protocol::conversation::observation::CategoryKind;

mod context;
pub use context::Context;

#[derive(Clone)]
pub struct Source {
    pub kind: CategoryKind,
    pub text: String,
}

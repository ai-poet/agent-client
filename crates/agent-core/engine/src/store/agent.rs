//! In-memory host storage for Sailry's canonical engine effects.
use crate::agent::canonical::{Context, Record};
use crate::agent::engine::{ConversationEffect, Effect};
use crate::agent::model::{Message, Usage};
use sailry_protocol::{Fault, SessionId};
use std::sync::Arc;

#[derive(Clone)]
pub struct RuntimeSession {
    pub id: String,
    pub records: Vec<Record>,
    pub conversation: Vec<Message>,
    pub context: Context,
    pub capability: Option<String>,
    pub reviewed: bool,
    pub thinking: Option<sailry_protocol::Effort>,
}

impl RuntimeSession {
    pub fn new(id: SessionId, conversation: Vec<Message>) -> Self {
        Self {
            id: id.to_string(),
            records: Vec::new(),
            conversation,
            context: Context::default(),
            capability: None,
            reviewed: false,
            thinking: None,
        }
    }
}

#[derive(Clone)]
pub struct Sessions {
    inner: Arc<tokio::sync::Mutex<RuntimeSession>>,
}

impl Sessions {
    pub fn new(session: RuntimeSession) -> Self {
        Self {
            inner: Arc::new(tokio::sync::Mutex::new(session)),
        }
    }

    pub async fn load(&self, id: &str) -> Result<RuntimeSession, Fault> {
        let session = self.inner.lock().await.clone();
        if session.id != id {
            return Err(Fault::invalid("session belongs to another engine"));
        }
        Ok(session)
    }

    pub async fn snapshot(&self) -> RuntimeSession {
        self.inner.lock().await.clone()
    }

    pub async fn apply_effects(
        &self,
        _session: &RuntimeSession,
        effects: &mut [Effect],
    ) -> anyhow::Result<()> {
        let mut current = self.inner.lock().await;
        for effect in effects {
            effect.identify();
            match effect {
                Effect::Conversation(ConversationEffect::AppendMessage(message)) => {
                    current.conversation.push(message.clone());
                }
                Effect::Conversation(ConversationEffect::PatchToolRequestMeta {
                    tool_call_id,
                    patch,
                }) => {
                    for call in current
                        .conversation
                        .iter_mut()
                        .flat_map(|message| message.tool_calls_mut())
                    {
                        if call.id == *tool_call_id {
                            let target =
                                call.tool_meta.get_or_insert_with(|| serde_json::json!({}));
                            merge(target, patch.clone());
                        }
                    }
                }
                Effect::Conversation(ConversationEffect::SetMessageVisibility {
                    message_id,
                    user_visible,
                    agent_visible,
                }) => {
                    if let Some(message) = current
                        .conversation
                        .iter_mut()
                        .find(|message| message.id.as_deref() == Some(message_id))
                    {
                        message.metadata.user_visible = *user_visible;
                        message.metadata.agent_visible = *agent_visible;
                    }
                }
                Effect::Usage(usage) => set_usage(&mut current.conversation, usage),
                Effect::Compaction {
                    summary,
                    usage,
                    tools,
                    ..
                } => {
                    current.conversation.clear();
                    current.records.clear();
                    current.context.loaded_tools = Some(tools.clone());
                    let mut compact_usage = usage.clone();
                    compact_usage.is_compaction = true;
                    summary.metadata.usage = Some(Box::new(compact_usage));
                    current.conversation.push((**summary).clone());
                }
                Effect::Records(records) => current.records.extend(records.clone()),
                Effect::Approval { .. } => {
                    return Err(anyhow::anyhow!(
                        "approval effects require a host approval adapter"
                    ));
                }
            }
        }
        Ok(())
    }
}

fn set_usage(messages: &mut [Message], usage: &Usage) {
    if let Some(message) = messages
        .iter_mut()
        .rev()
        .find(|message| message.role == crate::agent::model::Role::Assistant)
    {
        message.metadata.usage = Some(Box::new(usage.clone()));
    }
}

fn merge(target: &mut serde_json::Value, patch: serde_json::Value) {
    if let (Some(target), Some(patch)) = (target.as_object_mut(), patch.as_object()) {
        for (key, value) in patch {
            target.insert(key.clone(), value.clone());
        }
    }
}

#[derive(Clone)]
pub struct ApprovalBinding;

#[derive(Clone)]
pub struct Child {
    pub role: Option<sailry_protocol::role::Profile>,
}

pub struct Invocation {
    pub turn: sailry_protocol::QueuedTurn,
    pub compaction_percent: u8,
    pub child: Option<Child>,
}

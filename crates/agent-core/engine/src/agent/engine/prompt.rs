//! Host-neutral request preparation for the copied Sailry inference engine.
use super::Session;
use crate::agent::model::Message;
use std::collections::BTreeSet;
use tokio_util::sync::CancellationToken;

mod contexts;

#[derive(Default)]
pub struct InferenceInput {
    pub tools: Vec<rmcp::model::Tool>,
    pub instructions: Vec<(String, String)>,
    pub contexts: Vec<String>,
}

pub struct PreparedInferenceRequest {
    pub system_prompt: String,
    pub tools: Vec<rmcp::model::Tool>,
    pub additional_messages: Vec<Message>,
}

pub struct Prompt {
    pub instruction: String,
    pub context: String,
    pub parts: Vec<String>,
}

impl Prompt {
    pub async fn instructions(&self, session: &Session) -> anyhow::Result<Vec<(String, String)>> {
        let mut parts = vec![("instruction".into(), self.instruction.clone())];
        parts.extend(
            [
                ("policy", &session.context.policy),
                ("personal", &session.context.personal),
            ]
            .into_iter()
            .filter_map(|(name, text)| {
                text.as_ref()
                    .filter(|text| !text.is_empty())
                    .map(|text| (name.into(), text.clone()))
            }),
        );
        parts.extend(
            session
                .context
                .guides
                .iter()
                .map(|(key, guide)| (key.clone(), guide.content.clone())),
        );
        if let Some(capability) = &session.capability {
            parts.push(("capabilities".into(), capability.clone()));
        }
        Ok(parts)
    }

    pub async fn contexts(&self, session: &Session) -> anyhow::Result<Vec<String>> {
        let mut parts = session
            .context
            .environment
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        parts.push(self.context.clone());
        parts.extend(self.parts.iter().cloned());
        Ok(parts.into_iter().filter(|text| !text.is_empty()).collect())
    }
}

pub struct Preparer {
    pub stop: CancellationToken,
    pub deferred: BTreeSet<String>,
    pub output: tokio::sync::Mutex<crate::agent::output::Cache>,
}

impl Preparer {
    pub async fn prepare_messages(&self, session: &Session) -> anyhow::Result<Vec<Message>> {
        let mut messages = session
            .conversation
            .iter()
            .cloned()
            .filter_map(super::conversation::visible)
            .collect::<Vec<_>>();
        self.output.lock().await.project(&mut messages);
        Ok(messages)
    }

    pub async fn prepare(
        &self,
        session: &Session,
        conversation: &[Message],
        input: InferenceInput,
    ) -> anyhow::Result<PreparedInferenceRequest> {
        let mut input = input;
        input.contexts = contexts::pending(conversation, &[], std::mem::take(&mut input.contexts));
        let loaded = session.context.loaded_tools.clone().unwrap_or_default();
        Ok(prepare(input, &loaded, &self.deferred, &[]))
    }

    pub async fn hydrate(&self, _: &mut [Message]) -> Result<(), sailry_protocol::Fault> {
        Ok(())
    }
}

fn prepare(
    input: InferenceInput,
    loaded: &[String],
    deferred: &BTreeSet<String>,
    owned: &[&str],
) -> PreparedInferenceRequest {
    let mut tools = Vec::new();
    let mut additional = std::collections::BTreeMap::new();
    for tool in input.tools {
        if deferred.contains(tool.name.as_ref()) {
            additional.insert(tool.name.to_string(), tool);
        } else {
            tools.push(tool);
        }
    }
    for name in loaded {
        if let Some(tool) = additional.remove(name) {
            tools.push(tool);
        }
    }
    let parts = input
        .instructions
        .into_iter()
        .map(|(_, part)| part)
        .collect::<Vec<_>>();
    PreparedInferenceRequest {
        system_prompt: parts.join("\n\n"),
        tools,
        additional_messages: contexts::message(&input.contexts, owned)
            .into_iter()
            .collect(),
    }
}

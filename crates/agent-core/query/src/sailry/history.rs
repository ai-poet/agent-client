//! Lossless history conversion between the existing engine and Sailry's model types.
use agent_core::types::{ContentBlock, Message, MessageContent, Role, ToolResultContent};
use agent_core_engine::agent::model as engine;
use serde_json::{Map, Value};

const HOST: &str = "waku";

pub fn to_engine(message: &Message) -> engine::Message {
    let mut host = Map::new();
    host.insert(
        "content".into(),
        serde_json::to_value(&message.content).unwrap_or(Value::Null),
    );
    host.insert(
        "cost".into(),
        serde_json::to_value(&message.cost).unwrap_or(Value::Null),
    );
    host.insert(
        "snapshot_patch".into(),
        serde_json::to_value(&message.snapshot_patch).unwrap_or(Value::Null),
    );
    let mut output = engine::Message {
        id: message.uuid.clone(),
        created: 0,
        role: match message.role {
            Role::User => engine::Role::User,
            Role::Assistant => engine::Role::Assistant,
        },
        content: blocks(&message.content)
            .into_iter()
            .map(|content| to_engine_content(&content))
            .collect(),
        metadata: engine::Metadata::default(),
    };
    output.metadata.operations = Some(std::collections::BTreeMap::from([(HOST.into(), host)]));
    output
}

pub fn from_engine(message: &engine::Message) -> Message {
    let host = message
        .metadata
        .operations
        .as_ref()
        .and_then(|operations| operations.get(HOST));
    let content = host
        .and_then(|host| host.get("content"))
        .and_then(|value| serde_json::from_value::<MessageContent>(value.clone()).ok())
        .unwrap_or_else(|| {
            MessageContent::Blocks(message.content.iter().map(from_engine_content).collect())
        });
    Message {
        role: match message.role {
            engine::Role::User => Role::User,
            engine::Role::Assistant => Role::Assistant,
        },
        content,
        uuid: message.id.clone(),
        cost: host
            .and_then(|host| host.get("cost"))
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .flatten(),
        snapshot_patch: host
            .and_then(|host| host.get("snapshot_patch"))
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .flatten(),
    }
}

fn blocks(content: &MessageContent) -> Vec<ContentBlock> {
    match content {
        MessageContent::Text(text) => vec![ContentBlock::Text { text: text.clone() }],
        MessageContent::Blocks(blocks) => blocks.clone(),
    }
}

fn to_engine_content(content: &ContentBlock) -> engine::Content {
    match content {
        ContentBlock::Text { text } => engine::Content::Text { text: text.clone() },
        ContentBlock::Thinking {
            thinking,
            signature,
        } => engine::Content::Thinking {
            text: thinking.clone(),
            signature: Some(signature.clone()),
        },
        ContentBlock::RedactedThinking { data } => {
            engine::Content::RedactedThinking { data: data.clone() }
        }
        ContentBlock::ToolUse {
            id,
            name,
            input,
            thought_signature,
        } => engine::Content::ToolCall(engine::ToolCall {
            id: id.clone(),
            name: name.clone(),
            arguments: input.clone(),
            metadata: thought_signature.as_ref().map(|signature| {
                Map::from_iter([("thought_signature".into(), signature.clone().into())])
            }),
            tool_meta: None,
            error: None,
        }),
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => {
            let content = match content {
                ToolResultContent::Text(text) => {
                    vec![rmcp::model::ContentBlock::text(text.clone())]
                }
                ToolResultContent::Blocks(blocks) => blocks
                    .iter()
                    .filter_map(|block| match block {
                        ContentBlock::Text { text } => {
                            Some(rmcp::model::ContentBlock::text(text.clone()))
                        }
                        ContentBlock::Image { source } => source.data.as_ref().map(|data| {
                            rmcp::model::ContentBlock::Image(rmcp::model::ImageContent::new(
                                data.clone(),
                                source
                                    .media_type
                                    .clone()
                                    .unwrap_or_else(|| "image/png".into()),
                            ))
                        }),
                        other => Some(rmcp::model::ContentBlock::text(
                            serde_json::to_string(other).unwrap_or_default(),
                        )),
                    })
                    .collect(),
            };
            engine::Content::ToolResult(engine::ToolResult {
                id: tool_use_id.clone(),
                name: String::new(),
                result: Ok(if is_error.unwrap_or(false) {
                    rmcp::model::CallToolResult::error(content)
                } else {
                    rmcp::model::CallToolResult::success(content)
                }),
                metadata: None,
            })
        }
        ContentBlock::Image { source } => engine::Content::Image {
            media_type: source
                .media_type
                .clone()
                .unwrap_or_else(|| "image/png".into()),
            data: match (&source.data, &source.url) {
                (Some(data), _) => engine::Image::Inline {
                    data_base64: data.clone(),
                },
                (None, Some(url)) => engine::Image::Url { url: url.clone() },
                _ => engine::Image::Inline {
                    data_base64: String::new(),
                },
            },
            annotations: None,
        },
        ContentBlock::Document { source, title, .. } => engine::Content::Document {
            media_type: source
                .media_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".into()),
            name: title.clone(),
            data_base64: source
                .data
                .clone()
                .or_else(|| source.url.clone())
                .unwrap_or_default(),
        },
        other => engine::Content::Text {
            text: serde_json::to_string(other).unwrap_or_default(),
        },
    }
}

fn from_engine_content(content: &engine::Content) -> ContentBlock {
    match content {
        engine::Content::Text { text } => ContentBlock::Text { text: text.clone() },
        engine::Content::Thinking { text, signature } => ContentBlock::Thinking {
            thinking: text.clone(),
            signature: signature.clone().unwrap_or_default(),
        },
        engine::Content::RedactedThinking { data } => {
            ContentBlock::RedactedThinking { data: data.clone() }
        }
        engine::Content::ToolCall(call) => ContentBlock::ToolUse {
            id: call.id.clone(),
            name: call.name.clone(),
            input: call.arguments.clone(),
            thought_signature: call
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("thought_signature"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
        engine::Content::ToolResult(result) => {
            let (content, failed) = match &result.result {
                Ok(result) => (
                    result
                        .content
                        .iter()
                        .map(from_rmcp_content)
                        .collect::<Vec<_>>(),
                    result.is_error.unwrap_or(false),
                ),
                Err(error) => (
                    vec![ContentBlock::Text {
                        text: error.message.to_string(),
                    }],
                    true,
                ),
            };
            ContentBlock::ToolResult {
                tool_use_id: result.id.clone(),
                content: ToolResultContent::Blocks(content),
                is_error: Some(failed),
            }
        }
        engine::Content::Image {
            media_type, data, ..
        } => ContentBlock::Image {
            source: agent_core::types::ImageSource {
                source_type: match data {
                    engine::Image::Inline { .. } => "base64".into(),
                    engine::Image::Url { .. } => "url".into(),
                },
                media_type: Some(media_type.clone()),
                data: match data {
                    engine::Image::Inline { data_base64 } => Some(data_base64.clone()),
                    _ => None,
                },
                url: match data {
                    engine::Image::Url { url } => Some(url.clone()),
                    _ => None,
                },
            },
        },
        engine::Content::Document {
            media_type,
            name,
            data_base64,
        } => ContentBlock::Document {
            source: agent_core::types::DocumentSource {
                source_type: "base64".into(),
                media_type: Some(media_type.clone()),
                data: Some(data_base64.clone()),
                url: None,
            },
            title: name.clone(),
            context: None,
            citations: None,
        },
        engine::Content::Error { message, .. } => ContentBlock::Text {
            text: message.clone(),
        },
    }
}

fn from_rmcp_content(content: &rmcp::model::ContentBlock) -> ContentBlock {
    if let Some(text) = content.as_text() {
        return ContentBlock::Text {
            text: text.text.clone(),
        };
    }
    if let rmcp::model::ContentBlock::Image(image) = content {
        return ContentBlock::Image {
            source: agent_core::types::ImageSource {
                source_type: "base64".into(),
                media_type: Some(image.mime_type.clone()),
                data: Some(image.data.clone()),
                url: None,
            },
        };
    }
    ContentBlock::Text {
        text: serde_json::to_string(content).unwrap_or_else(|_| "unsupported tool content".into()),
    }
}

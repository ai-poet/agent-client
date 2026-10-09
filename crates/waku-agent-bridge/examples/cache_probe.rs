//! Temporary diagnostic (not for commit): run each argument as one turn of a
//! real built-in agent session configured by `CLAURST_HOME`.
use std::sync::mpsc;
use waku_agent_bridge::{AccessMode, AgentEvent, AgentSession, AgentStartOptions, EventSink, WireFormat};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let cwd = args.next().expect("cwd");
    let model = args.next().expect("model");
    let prompts: Vec<String> = args.collect();
    let (tx, rx) = mpsc::channel();
    let sink = EventSink::new(move |event| {
        let _ = tx.send(event);
    });
    let session = AgentSession::start(
        AgentStartOptions {
            cwd: cwd.into(),
            access_mode: AccessMode::FullAccess,
            plan_mode: false,
            model: Some(model),
            platform: Some("openai".into()),
            wire_format: Some(WireFormat::Responses),
            reasoning_effort: Some("low".into()),
            narration_language: Some("Simplified Chinese".into()),
            history: Vec::new(),
            computer_use: None,
            session_id: Some(uuid::Uuid::new_v4().to_string()),
            browser_tools: false,
        },
        sink,
    )?;
    for prompt in prompts {
        eprintln!(">>> {prompt}");
        session.prompt(prompt);
        loop {
            match rx.recv()? {
                AgentEvent::ToolStarted { name, input, .. } => eprintln!("  tool {name} {input}"),
                AgentEvent::ToolFinished { name, failed, output, .. } => {
                    let text = output.to_string();
                    eprintln!("  done {name} failed={failed} {}", text.chars().take(80).collect::<String>())
                }
                AgentEvent::Error(message) => eprintln!("  error {message}"),
                AgentEvent::TurnFinished { success, summary } => {
                    eprintln!("  turn finished success={success} {summary:?}");
                    break;
                }
                _ => {}
            }
        }
    }
    Ok(())
}

//! Privacy-conscious product analytics for release builds.
//!
//! Event creation is a bounded `try_send` on the UI thread. A dedicated
//! worker owns both the Tokio runtime required by `rust-umami`/Reqwest and a
//! single Umami session, so networking, TLS, and response-cache bookkeeping
//! never enter a frame path. Events deliberately contain no prompts, project
//! names or paths, provider output, or provider-account identity. A random
//! installation-scoped ID keeps aggregate sessions coherent across launches.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::time::Duration;

use rust_umami::{Client, Context};
use serde_json::{Value, json};
use uuid::Uuid;

const EVENT_QUEUE_CAPACITY: usize = 128;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[cfg(not(debug_assertions))]
const ENDPOINT: Option<&str> = option_env!("WAKU_ANALYTICS_ENDPOINT");
#[cfg(debug_assertions)]
const ENDPOINT: Option<&str> = None;

#[cfg(not(debug_assertions))]
const WEBSITE_ID: Option<&str> = option_env!("WAKU_ANALYTICS_WEBSITE_ID");
#[cfg(debug_assertions)]
const WEBSITE_ID: Option<&str> = None;

/// A cheap handle to the background analytics worker.
#[derive(Clone)]
pub struct Analytics {
    events: SyncSender<Event>,
    enabled: Arc<AtomicBool>,
    available: bool,
}

impl Analytics {
    /// Starts analytics only for release builds with configuration embedded
    /// at compile time. Any build can opt out with
    /// `WAKU_DISABLE_ANALYTICS=1`.
    pub fn new(language: &'static str, distinct_id: Uuid, sharing_enabled: bool) -> Self {
        let (events, receiver) = sync_channel(EVENT_QUEUE_CAPACITY);
        let available = analytics_available();
        let enabled = Arc::new(AtomicBool::new(available && sharing_enabled));
        if available {
            // If the thread cannot be created, its captured receiver is
            // dropped and future `try_send`s simply observe disconnection.
            // Analytics is never allowed to affect app startup or actions.
            let worker_enabled = Arc::clone(&enabled);
            let _ = std::thread::Builder::new()
                .name("waku-analytics".into())
                .spawn(move || run(receiver, language, distinct_id, worker_enabled));
        } else {
            drop(receiver);
        }
        Self {
            events,
            enabled,
            available,
        }
    }

    /// Enqueues an event without waiting for queue space or network I/O.
    pub fn track(&self, event: Event) {
        if !self.enabled.load(Ordering::Acquire) {
            return;
        }
        let _ = self.events.try_send(event);
    }

    /// Applies the user's persisted sharing preference. The worker also checks
    /// this flag before sending, so disabling drops events already queued.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled
            .store(self.available && enabled, Ordering::Release);
    }
}

/// The deliberately small product vocabulary sent to Umami.
pub enum Event {
    AppLaunched {
        task_count: usize,
        project_count: usize,
    },
    ProjectAdded,
    RightPanelOpened,
    TurnSubmitted {
        provider: &'static str,
        model: String,
        turn_number: usize,
        workspace: &'static str,
        projectless: bool,
        attachment_count: usize,
        has_input: bool,
    },
    TurnFinished {
        provider: &'static str,
        turn_number: usize,
        outcome: TurnOutcome,
        duration_seconds: u64,
    },
    PermissionResponded {
        provider: &'static str,
        kind: &'static str,
        decision: &'static str,
    },
    ConversationRolledBack {
        provider: &'static str,
        turns: usize,
    },
    ResponseForked {
        provider: &'static str,
        turn_number: usize,
    },
}

#[derive(Clone, Copy)]
pub enum TurnOutcome {
    Completed,
    Failed,
    Cancelled,
    ProcessExited,
    StartFailed,
    PreparationFailed,
}

impl TurnOutcome {
    fn id(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::ProcessExited => "process_exited",
            Self::StartFailed => "start_failed",
            Self::PreparationFailed => "preparation_failed",
        }
    }
}

impl Event {
    fn into_track(self) -> (&'static str, Value) {
        let (name, mut data) = match self {
            Self::AppLaunched {
                task_count,
                project_count,
            } => (
                "app.launched",
                json!({
                    "taskCount": task_count,
                    "projectCount": project_count,
                }),
            ),
            Self::ProjectAdded => ("project.added", json!({})),
            Self::RightPanelOpened => ("right_panel.opened", json!({})),
            Self::TurnSubmitted {
                provider,
                model,
                turn_number,
                workspace,
                projectless,
                attachment_count,
                has_input,
            } => (
                "provider.turn.sent",
                json!({
                    "provider": provider,
                    "model": model,
                    "turnNumber": turn_number,
                    "workspace": workspace,
                    "projectless": projectless,
                    "attachmentCount": attachment_count,
                    "hasInput": has_input,
                }),
            ),
            Self::TurnFinished {
                provider,
                turn_number,
                outcome,
                duration_seconds,
            } => (
                "provider.turn.finished",
                json!({
                    "provider": provider,
                    "turnNumber": turn_number,
                    "outcome": outcome.id(),
                    "durationSeconds": duration_seconds,
                }),
            ),
            Self::PermissionResponded {
                provider,
                kind,
                decision,
            } => (
                "provider.request.responded",
                json!({
                    "provider": provider,
                    "kind": kind,
                    "decision": decision,
                }),
            ),
            Self::ConversationRolledBack { provider, turns } => (
                "provider.conversation.rolled_back",
                json!({
                    "provider": provider,
                    "turns": turns,
                }),
            ),
            Self::ResponseForked {
                provider,
                turn_number,
            } => (
                "provider.response.forked",
                json!({
                    "provider": provider,
                    "turnNumber": turn_number,
                }),
            ),
        };

        let properties = data
            .as_object_mut()
            .expect("analytics event data is always an object");
        properties.insert("clientType".into(), json!("desktop"));
        properties.insert("version".into(), json!(env!("CARGO_PKG_VERSION")));
        properties.insert("platform".into(), json!(std::env::consts::OS));
        properties.insert("arch".into(), json!(std::env::consts::ARCH));
        properties.insert(
            "build".into(),
            json!(if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            }),
        );
        (name, data)
    }
}

fn analytics_available() -> bool {
    !cfg!(debug_assertions)
        && !env_flag("WAKU_DISABLE_ANALYTICS")
        && ENDPOINT.is_some_and(|value| !value.trim().is_empty())
        && WEBSITE_ID.is_some_and(|value| !value.trim().is_empty())
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn run(
    receiver: Receiver<Event>,
    language: &'static str,
    distinct_id: Uuid,
    enabled: Arc<AtomicBool>,
) {
    // Fork change: telemetry is off unless the build explicitly opts in. This
    // fork has no analytics endpoint of its own, and reporting to upstream's
    // would both pollute their data and disclose our users' activity to a third
    // party they never agreed to.
    if !sub2api::brand::ANALYTICS_ENABLED {
        return;
    }
    let (Some(endpoint), Some(website_id)) = (ENDPOINT, WEBSITE_ID) else {
        return;
    };
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };

    // Client construction may initialize runtime-aware networking helpers,
    // so enter the worker's runtime even though the builder itself is sync.
    let client = {
        let _runtime = runtime.enter();
        Client::builder(endpoint, website_id)
            .default_context(
                Context::new()
                    .hostname(sub2api::brand::WEBSITE_URL.trim_start_matches("https://"))
                    .url("/desktop")
                    .title(sub2api::brand::DISPLAY_NAME)
                    .language(language)
                    .os(std::env::consts::OS)
                    .device("desktop"),
            )
            .user_agent(sub2api::brand::user_agent(env!("CARGO_PKG_VERSION")))
            .timeout(REQUEST_TIMEOUT)
            .build()
    };
    let Ok(client) = client else {
        return;
    };
    let session = client.session().distinct_id(distinct_id.to_string());

    while let Ok(event) = receiver.recv() {
        if !enabled.load(Ordering::Acquire) {
            continue;
        }
        let (name, data) = event.into_track();
        // A failed analytics request is intentionally terminal only for this
        // event. The next product action gets an independent best-effort send.
        let _ = runtime.block_on(session.event(name).data(data).send());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(debug_assertions)]
    #[test]
    fn debug_builds_never_enable_analytics() {
        assert!(!analytics_available());
    }

    #[test]
    fn sharing_preference_gates_events_before_the_queue() {
        let (events, receiver) = sync_channel(1);
        let analytics = Analytics {
            events,
            enabled: Arc::new(AtomicBool::new(false)),
            available: true,
        };

        analytics.track(Event::ProjectAdded);
        assert!(receiver.try_recv().is_err());

        analytics.set_enabled(true);
        analytics.track(Event::ProjectAdded);
        assert!(receiver.try_recv().is_ok());

        analytics.set_enabled(false);
        analytics.track(Event::ProjectAdded);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn turn_events_expose_only_coarse_product_metadata() {
        let (name, data) = Event::TurnSubmitted {
            provider: "codex",
            model: "gpt-5".into(),
            turn_number: 2,
            workspace: "worktree",
            projectless: false,
            attachment_count: 1,
            has_input: true,
        }
        .into_track();

        assert_eq!(name, "provider.turn.sent");
        assert_eq!(data["provider"], "codex");
        assert_eq!(data["model"], "gpt-5");
        assert_eq!(data["workspace"], "worktree");
        assert_eq!(data["clientType"], "desktop");
        assert!(data.get("prompt").is_none());
        assert!(data.get("project_path").is_none());
        assert!(data.get("user_id").is_none());
    }
}

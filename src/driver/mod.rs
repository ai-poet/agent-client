//! Desktop proxy for the provider runtime owned by `client-daemon`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::computer_use::ComputerToolRequest;
use crate::model::{
    BackgroundWorkKey, DriverEvent, ProviderKind, ProviderResumeCursor, RuntimeEventCursor,
};
use crossbeam_channel::{Sender, bounded, select};
use parking_lot::Mutex;

pub use client::driver::{
    DriverControl, DriverEventSender, DriverHandle, DriverStartOptions, SessionOptions,
    event_channel,
};

pub(crate) fn start_remote(
    daemon: client::DaemonSupervisor,
    session_id: uuid::Uuid,
    provider: ProviderKind,
    options: DriverStartOptions,
    events: DriverEventSender,
) -> anyhow::Result<DriverHandle> {
    let client = daemon.client();
    let runtime_id = uuid::Uuid::new_v4();
    let command = client::Command::Start {
        options: client::WireDriverStartOptions {
            provider: client::encode_enum(provider)?,
            binary: options.binary,
            cwd: options.cwd,
            mode: client::encode_enum(options.mode)?,
            interaction_mode: client::encode_enum(options.interaction_mode)?,
            model: options.model,
            reasoning_effort: options.reasoning_effort,
            service_tier: options.service_tier,
            context_window: options.context_window,
            agent_preset: options.agent_preset,
            computer_use_enabled: options.computer_use_enabled,
            provider_cursor: options
                .provider_cursor
                .map(serde_json::to_value)
                .transpose()?,
        },
    };
    let supports_steer = match client.request(session_id, runtime_id, command) {
        Ok(client::ResponsePayload::Started { supports_steer }) => supports_steer,
        Ok(_) => anyhow::bail!("the daemon returned an invalid start response"),
        Err(error) => return Err(error),
    };
    connect_remote(
        daemon,
        client,
        session_id,
        runtime_id,
        supports_steer,
        None,
        events,
    )
}

pub(crate) fn attach_remote(
    daemon: client::DaemonSupervisor,
    client: client::DaemonClient,
    session_id: uuid::Uuid,
    runtime_id: uuid::Uuid,
    supports_steer: bool,
    replay_cursor: Option<RuntimeEventCursor>,
    events: DriverEventSender,
) -> anyhow::Result<DriverHandle> {
    connect_remote(
        daemon,
        client,
        session_id,
        runtime_id,
        supports_steer,
        replay_cursor,
        events,
    )
}

fn connect_remote(
    daemon: client::DaemonSupervisor,
    initial_client: client::DaemonClient,
    session_id: uuid::Uuid,
    runtime_id: uuid::Uuid,
    supports_steer: bool,
    replay_cursor: Option<RuntimeEventCursor>,
    events: DriverEventSender,
) -> anyhow::Result<DriverHandle> {
    let client_updates = daemon.subscribe_clients();
    let active_client = Arc::new(Mutex::new(initial_client.clone()));
    let forwarding_client = active_client.clone();
    let closed = Arc::new(AtomicBool::new(false));
    let forwarding_closed = closed.clone();
    let (shutdown, forwarding_shutdown) = bounded(1);
    let forwarding_events = events.clone();
    let thread_initial_client = initial_client.clone();
    let spawn = std::thread::Builder::new()
        .name(format!("client-daemon-session-{session_id}"))
        .spawn(move || {
            let mut client = thread_initial_client;
            let mut remote_events = client.subscribe(session_id, runtime_id);
            loop {
                let disconnected = loop {
                    select! {
                        recv(forwarding_shutdown) -> _ => return,
                        recv(remote_events) -> sequenced => {
                            let Ok(sequenced) = sequenced else {
                                break true;
                            };
                            if replay_cursor.is_some_and(|cursor| {
                                cursor.runtime_id == sequenced.runtime_id
                                    && cursor.epoch == sequenced.epoch
                                    && cursor.sequence >= sequenced.sequence
                            }) {
                                continue;
                            }
                            let cursor = RuntimeEventCursor {
                                runtime_id: sequenced.runtime_id,
                                epoch: sequenced.epoch,
                                sequence: sequenced.sequence,
                            };
                            let event = match client::event_from_wire(sequenced.event) {
                                Ok(event) => event,
                                // Fork: a newer daemon's event this build does not
                                // know is skipped, not reported as a broken daemon;
                                // the events around it still make sense.
                                Err(error) if error.is::<client::UnsupportedDriverEvent>() => {
                                    if forwarding_events
                                        .send(DriverEvent::RuntimeEventCursorAdvanced(cursor))
                                        .is_err()
                                    {
                                        return;
                                    }
                                    continue;
                                }
                                Err(error) => DriverEvent::Error(format!(
                                    "the daemon sent an invalid event: {error}"
                                )),
                            };
                            let process_exited = matches!(&event, DriverEvent::ProcessExited);
                            if forwarding_events.send(event).is_err()
                                || forwarding_events
                                    .send(DriverEvent::RuntimeEventCursorAdvanced(cursor))
                                    .is_err()
                            {
                                return;
                            }
                            if process_exited {
                                break false;
                            }
                        }
                    }
                };
                client.unsubscribe(session_id, runtime_id);
                if !disconnected || forwarding_closed.load(Ordering::Acquire) {
                    break;
                }

                // A closed WebSocket says nothing about the provider process.
                // Wait for the supervisor's next connection, then ask that
                // daemon whether this exact runtime survived before changing
                // the task status.
                let replacement = loop {
                    select! {
                        recv(forwarding_shutdown) -> _ => return,
                        recv(client_updates) -> replacement => {
                            let Ok(replacement) = replacement else {
                                return;
                            };
                            if !client.same_connection(&replacement) {
                                break replacement;
                            }
                        }
                    }
                };
                let attached = replacement.request(
                    session_id,
                    uuid::Uuid::nil(),
                    client::Command::AttachSession,
                );
                match attached {
                    Ok(client::ResponsePayload::SessionRuntime {
                        runtime_id: Some(attached_runtime_id),
                        ..
                    }) if attached_runtime_id == runtime_id => {
                        *forwarding_client.lock() = replacement.clone();
                        client = replacement;
                        remote_events = client.subscribe(session_id, runtime_id);
                    }
                    Ok(client::ResponsePayload::SessionRuntime { .. }) => {
                        let _ = forwarding_events.send(DriverEvent::ProcessExited);
                        break;
                    }
                    Ok(_) => {
                        let _ = forwarding_events.send(DriverEvent::Error(
                            "the daemon returned an invalid runtime attachment response".into(),
                        ));
                        break;
                    }
                    Err(_) => {
                        // This replacement also disappeared. Its client
                        // channel will be followed by another supervisor
                        // publication; keep the task live until one can answer
                        // authoritatively.
                        client = replacement;
                    }
                }
            }
        });
    if let Err(error) = spawn {
        initial_client.unsubscribe(session_id, runtime_id);
        return Err(error.into());
    }
    Ok(DriverHandle::from_control(Arc::new(RemoteDriverControl {
        client: active_client,
        session_id,
        runtime_id,
        supports_steer,
        events,
        closed,
        shutdown,
    })))
}

struct RemoteDriverControl {
    client: Arc<Mutex<client::DaemonClient>>,
    session_id: uuid::Uuid,
    runtime_id: uuid::Uuid,
    supports_steer: bool,
    events: DriverEventSender,
    closed: Arc<AtomicBool>,
    shutdown: Sender<()>,
}

impl RemoteDriverControl {
    fn notify(&self, command: client::Command) {
        let client = self.client.lock().clone();
        if let Err(error) = client.notify(self.session_id, self.runtime_id, command)
            && let Some(notice) = transport_failure_notice(&error.to_string())
        {
            let _ = self.events.send(DriverEvent::Error(notice));
        }
    }
}

/// What a failed fire-and-forget command tells the session.
///
/// Fork addition. A daemon answer is always reported. A dropped socket is
/// not: the supervisor is already redialing or restarting, the connection
/// banner says so, and every command sent during the outage would otherwise
/// raise the same notice again. The background-work poll alone did it every
/// five seconds, and each notice also marked the task failed.
fn transport_failure_notice(error: &str) -> Option<String> {
    (!client::is_daemon_transport_error(error))
        .then(|| format!("the daemon command failed: {error}"))
}

impl DriverControl for RemoteDriverControl {
    fn prompt(&self, prompt: String) {
        self.notify(client::Command::Prompt { prompt });
    }

    fn supports_steer(&self) -> bool {
        self.supports_steer
    }

    fn steer(&self, prompt: String) {
        self.notify(client::Command::Steer { prompt });
    }

    fn cancel(&self) {
        self.notify(client::Command::Cancel);
    }

    fn cancel_computer_use(&self) {
        self.notify(client::Command::CancelComputerUse);
    }

    fn refresh_background_work(&self) {
        self.notify(client::Command::RefreshBackgroundWork);
    }

    fn suspend_background_work(&self, key: BackgroundWorkKey, control_id: String) {
        match serde_json::to_value(key) {
            Ok(key) => self.notify(client::Command::SuspendBackgroundWork { key, control_id }),
            Err(error) => {
                let _ = self.events.send(DriverEvent::Error(format!(
                    "could not encode background-work command: {error}"
                )));
            }
        }
    }

    fn stop_background_work(&self, key: BackgroundWorkKey, control_id: String) {
        match serde_json::to_value(key) {
            Ok(key) => self.notify(client::Command::StopBackgroundWork { key, control_id }),
            Err(error) => {
                let _ = self.events.send(DriverEvent::Error(format!(
                    "could not encode background-work command: {error}"
                )));
            }
        }
    }

    fn respond(&self, request_id: String, option_id: String) {
        self.notify(client::Command::Respond {
            request_id,
            option_id,
        });
    }

    fn respond_user_input(
        &self,
        request_id: String,
        answers: Vec<client_protocol::model::UserInputAnswer>,
    ) {
        self.notify(client::Command::RespondUserInput {
            request_id,
            answers,
        });
    }

    // Fork addition: the in-app browser's answer to the built-in agent.
    fn browser_result(&self, request_id: String, result: serde_json::Value) {
        self.notify(client::Command::BrowserResult { request_id, result });
    }

    fn goal(&self, operation: client_protocol::model::GoalOperation) {
        self.notify(client::Command::Goal { operation });
    }

    fn run_computer_tool(&self, request: ComputerToolRequest) {
        self.notify(client::Command::RunComputerTool {
            request: client::WireComputerToolRequest {
                call_id: request.call_id,
                tool: request.tool,
                arguments: request.arguments,
            },
        });
    }

    fn reject_computer_tool(&self, request: ComputerToolRequest, reason: String) {
        self.notify(client::Command::RejectComputerTool {
            request: client::WireComputerToolRequest {
                call_id: request.call_id,
                tool: request.tool,
                arguments: request.arguments,
            },
            reason,
        });
    }

    fn apply_options(&self, options: SessionOptions) -> bool {
        let options = (|| {
            Ok::<_, anyhow::Error>(client::WireSessionOptions {
                mode: client::encode_enum(options.mode)?,
                interaction_mode: client::encode_enum(options.interaction_mode)?,
                model: options.model,
                reasoning_effort: options.reasoning_effort,
                service_tier: options.service_tier,
                context_window: options.context_window,
            })
        })();
        let Ok(options) = options else {
            return false;
        };
        let client = self.client.lock().clone();
        matches!(
            client.request(
                self.session_id,
                self.runtime_id,
                client::Command::ApplyOptions { options }
            ),
            Ok(client::ResponsePayload::OptionsApplied { applied: true })
        )
    }

    fn rollback(&self, turns: usize) -> anyhow::Result<Option<ProviderResumeCursor>> {
        let client = self.client.lock().clone();
        match client.request(
            self.session_id,
            self.runtime_id,
            client::Command::Rollback { turns },
        )? {
            client::ResponsePayload::Cursor { cursor } => cursor
                .map(serde_json::from_value)
                .transpose()
                .map_err(Into::into),
            _ => anyhow::bail!("the daemon returned an invalid rollback response"),
        }
    }

    fn fork(&self, turns_to_remove: usize) -> anyhow::Result<ProviderResumeCursor> {
        let client = self.client.lock().clone();
        match client.request(
            self.session_id,
            self.runtime_id,
            client::Command::Fork { turns_to_remove },
        )? {
            client::ResponsePayload::Cursor {
                cursor: Some(cursor),
            } => serde_json::from_value(cursor).map_err(Into::into),
            _ => anyhow::bail!("the daemon returned an invalid fork response"),
        }
    }

    fn close(&self) {
        self.notify(client::Command::CloseSession);
    }
}

impl Drop for RemoteDriverControl {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        let _ = self.shutdown.try_send(());
        let client = self.client.lock().clone();
        client.unsubscribe(self.session_id, self.runtime_id);
    }
}

#[cfg(test)]
mod tests {
    use super::transport_failure_notice;

    #[test]
    fn a_dropped_socket_is_not_a_command_failure_but_a_daemon_answer_is() {
        assert_eq!(transport_failure_notice(client::DAEMON_DISCONNECTED), None);
        assert_eq!(transport_failure_notice(client::DAEMON_DROPPED), None);
        assert_eq!(
            transport_failure_notice(client::DAEMON_CONNECTION_CLOSED),
            None
        );
        assert_eq!(
            transport_failure_notice("provider said no").as_deref(),
            Some("the daemon command failed: provider said no")
        );
    }
}

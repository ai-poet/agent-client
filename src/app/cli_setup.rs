//! Settings → Providers: detection state and the install machinery behind
//! the per-CLI cards in `providers_page`.
//!
//! Fork addition. Upstream tells the user a CLI was "not detected" and stops
//! there; this fork detects Node and the agent CLIs the way a terminal would
//! (`sub2api::cli_detect`, on a background pass — never on a frame), installs
//! what is missing, and verifies each install against the binary rather than
//! npm's exit status (`sub2api::cli_install`). Both crates are unit-tested
//! without GPUI; this file owns the view state, the background scheduling,
//! and the install runs. Rendering lives in `providers_page.rs`.

use std::time::{Duration, Instant};

use super::*;

/// View state for the setup section.
#[derive(Default)]
pub(super) struct CliSetupState {
    /// Provider id whose install is currently running; `"node"` for the
    /// runtime itself.
    pub running: Option<String>,
    /// Tail of the last failed install, kept on screen so the user can read
    /// npm's own reason rather than a generic failure.
    pub last_error: Option<String>,
    /// How each CLI's last install ended, by provider id — verified against
    /// the binary, not npm's exit status. Cleared when that CLI is retried.
    pub install_results:
        std::collections::HashMap<String, sub2api::cli_install::InstallVerdict>,
    /// Stage label for the Node install, written by the background installer
    /// and read by a UI poll loop while it runs.
    pub node_stage: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    /// CLI ids ticked for installation. Interior mutability: the default tick
    /// is seeded during render, which only has `&self`.
    pub selected: std::cell::RefCell<std::collections::HashSet<String>>,
    /// The last detection pass. Filled in by a background task — detection
    /// spawns processes, which a frame must never do — and read by render.
    pub snapshot: Option<std::sync::Arc<sub2api::cli_detect::EnvironmentSnapshot>>,
    /// Bumped per refresh; a result from a superseded pass is discarded.
    snapshot_generation: std::cell::Cell<u64>,
    /// A pass is in flight. `Cell` because render schedules the first one.
    snapshot_pending: std::cell::Cell<bool>,
    /// When the current snapshot landed; `None` forces the next refresh.
    snapshot_at: std::cell::Cell<Option<Instant>>,
    /// Cached stored custom endpoints; render reads this instead of the
    /// file. Refilled lazily, replaced on save.
    pub custom_cache: std::cell::RefCell<Option<sub2api::custom_api::CustomApiConfig>>,
    /// Per-CLI endpoint form state for the Providers page.
    pub page: super::providers_page::ProvidersPageState,
}

impl CliSetupState {
    pub fn snapshot_pending(&self) -> bool {
        self.snapshot_pending.get()
    }

}

/// How long a detection result stays good before render schedules another
/// pass. Long: a pass spawns up to five processes, and the installs that
/// change the answer refresh it explicitly.
const DETECTION_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// Every directory the desktop searches for Node and the agent CLIs: the
/// user's shell `PATH` (resolved once, off-thread), the process `PATH`, the
/// version-manager and managed-runtime directories, and the client's own
/// tool directories — the same set the daemon's provider rows search, so the
/// two never disagree about the same machine. Spawns a shell on first use.
fn cli_search_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = sub2api::cli_detect::default_search_dirs();
    if let Some(client_path) = crate::command_env::executable_search_path() {
        for dir in std::env::split_paths(&client_path) {
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }
    dirs
}

/// `sk-abcdef...xyz` — the Electron client's masking, enough to recognise a
/// key without exposing it.
pub(super) fn mask_api_key(key: &str) -> String {
    let trimmed = key.trim();
    if trimmed.len() <= 12 {
        return "***".to_owned();
    }
    format!("{}...{}", &trimmed[..8], &trimmed[trimmed.len() - 4..])
}

/// Human label for a CLI install stage.
fn install_stage_label(reported: sub2api::cli_install::InstallStage) -> String {
    match reported {
        sub2api::cli_install::InstallStage::Running { attempt } => {
            tr!("cli_setup.stage_installing", method = attempt)
        }
        sub2api::cli_install::InstallStage::Verifying => tr!("cli_setup.stage_verifying"),
    }
}

/// What to show for an install that did not simply succeed: npm's own tail,
/// plus the fix when the failure has a known one.
pub(super) fn install_verdict_detail(verdict: &sub2api::cli_install::InstallVerdict) -> String {
    use sub2api::cli_install::{InstallHint, InstallVerdict, permission_fix_commands};
    match verdict {
        InstallVerdict::Installed { path, version } => {
            format!("{} ({})", path.display(), version)
        }
        InstallVerdict::InstalledNotOnPath { bin_dir, .. } => tr!(
            "cli_setup.installed_not_on_path",
            name = "",
            dir = bin_dir.display().to_string()
        ),
        InstallVerdict::InstalledNotRunnable { path, diagnostic } => tr!(
            "cli_setup.installed_not_runnable",
            path = path.display().to_string(),
            detail = diagnostic
        ),
        InstallVerdict::Failed { output, hint } => {
            let mut text = output.clone();
            match hint {
                Some(InstallHint::Permission) => {
                    text.push('\n');
                    text.push_str(&tr!("cli_setup.hint_permission"));
                    for command in permission_fix_commands() {
                        text.push('\n');
                        text.push_str(&command);
                    }
                }
                Some(InstallHint::Network) => {
                    text.push('\n');
                    text.push_str(&tr!("cli_setup.hint_network"));
                }
                None => {}
            }
            text
        }
    }
}

/// Human label for an installer stage.
fn stage_label(reported: sub2api::node_install::NodeStage) -> String {
    match reported {
        sub2api::node_install::NodeStage::ResolvingDownload => tr!("cli_setup.stage_resolving"),
        sub2api::node_install::NodeStage::Downloading => tr!("cli_setup.stage_downloading"),
        sub2api::node_install::NodeStage::Installing { method } => {
            tr!("cli_setup.stage_installing", method = method)
        }
        sub2api::node_install::NodeStage::Verifying => tr!("cli_setup.stage_verifying"),
    }
}


impl Waku {
    /// Schedule a detection pass unless one is running or the current
    /// snapshot is recent. Called from render, so it only schedules.
    pub(super) fn ensure_cli_environment_fresh(&self, cx: &mut Context<Self>) {
        if self.cli_setup.snapshot_pending.get() {
            return;
        }
        let fresh = self
            .cli_setup
            .snapshot_at
            .get()
            .is_some_and(|at| at.elapsed() < DETECTION_REFRESH_INTERVAL);
        if fresh {
            return;
        }
        self.schedule_cli_environment_refresh(cx);
    }

    /// Run a detection pass now, superseding any in flight.
    pub(super) fn refresh_cli_environment(&mut self, cx: &mut Context<Self>) {
        self.schedule_cli_environment_refresh(cx);
    }

    fn schedule_cli_environment_refresh(&self, cx: &mut Context<Self>) {
        let generation = self.cli_setup.snapshot_generation.get() + 1;
        self.cli_setup.snapshot_generation.set(generation);
        self.cli_setup.snapshot_pending.set(true);
        cx.spawn(async move |this, cx| {
            let snapshot = cx
                .background_executor()
                .spawn(async move { sub2api::cli_detect::snapshot(&cli_search_dirs()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.cli_setup.snapshot_generation.get() != generation {
                    return;
                }
                this.cli_setup.snapshot = Some(std::sync::Arc::new(snapshot));
                this.cli_setup.snapshot_at.set(Some(Instant::now()));
                this.cli_setup.snapshot_pending.set(false);
                cx.notify();
            });
        })
        .detach();
    }

    /// Install Node unattended, reporting each stage as it starts.
    pub(super) fn run_node_install(&mut self, cx: &mut Context<Self>) {
        if self.cli_setup.running.is_some() {
            return;
        }
        self.cli_setup.running = Some("node".to_owned());
        self.cli_setup.last_error = None;
        *self.cli_setup.node_stage.lock().unwrap() = Some(tr!("cli_setup.stage_resolving"));
        cx.notify();

        // The installer reports stages from the background thread; this poll
        // loop moves them onto the screen. 300ms is imperceptible next to a
        // download measured in tens of seconds.
        let poll_id = "node";
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(300))
                    .await;
                let still_running = this.update(cx, |this, cx| {
                    let running = this.cli_setup.running.as_deref() == Some(poll_id);
                    if running {
                        cx.notify();
                    }
                    running
                });
                if !matches!(still_running, Ok(true)) {
                    break;
                }
            }
        })
        .detach();

        let stage = self.cli_setup.node_stage.clone();
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    sub2api::node_install::install_node(|reported| {
                        *stage.lock().unwrap() = Some(stage_label(reported));
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.cli_setup.running = None;
                *this.cli_setup.node_stage.lock().unwrap() = None;
                this.refresh_cli_environment(cx);
                if outcome.success {
                    this.show_toast(tr!("cli_setup.installed"));
                } else {
                    this.cli_setup.last_error = Some(outcome.output);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Install every ticked CLI, resolving prerequisites first.
    ///
    /// Component-installer semantics: the user picks agents; Node is a
    /// dependency of every npm install, so a missing Node is installed as part
    /// of the batch rather than being a separate chore. Items run
    /// sequentially — npm's global prefix is not safe for concurrent writes —
    /// with the active row marked as it goes.
    pub(super) fn run_selected_cli_installs(&mut self, cx: &mut Context<Self>) {
        if self.cli_setup.running.is_some() {
            return;
        }
        // Descriptor order, not hash order, so the run is deterministic.
        let queue: Vec<&'static sub2api::cli_install::CliDescriptor> = {
            let selected = self.cli_setup.selected.borrow();
            sub2api::cli_install::DESCRIPTORS
                .iter()
                .filter(|descriptor| selected.contains(descriptor.id))
                .collect()
        };
        if queue.is_empty() {
            return;
        }
        let detected_node_dir = self
            .cli_setup
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.node_bin_dir());
        // From the last detection pass, not a fresh probe: a click handler
        // must not block on `node --version`.
        let needs_node = !self
            .cli_setup
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.node.version())
            .is_some_and(sub2api::cli_install::node_is_supported);

        self.cli_setup.last_error = None;
        for descriptor in &queue {
            self.cli_setup.install_results.remove(descriptor.id);
        }
        self.cli_setup.running = Some(if needs_node {
            "node".to_owned()
        } else {
            queue[0].id.to_owned()
        });
        cx.notify();

        // One poll loop for the whole batch keeps the stage text and the
        // per-row "Installing…" marker moving.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(300))
                    .await;
                let still_running = this.update(cx, |this, cx| {
                    let running = this.cli_setup.running.is_some();
                    if running {
                        cx.notify();
                    }
                    running
                });
                if !matches!(still_running, Ok(true)) {
                    break;
                }
            }
        })
        .detach();

        let stage = self.cli_setup.node_stage.clone();
        cx.spawn(async move |this, cx| {
            let mut failures: Vec<String> = Vec::new();
            let mut installed = 0usize;

            if needs_node {
                let stage = stage.clone();
                let outcome = cx
                    .background_executor()
                    .spawn(async move {
                        sub2api::node_install::install_node(|reported| {
                            *stage.lock().unwrap() = Some(stage_label(reported));
                        })
                    })
                    .await;
                if !outcome.success {
                    // Without Node every npm item would fail identically, so
                    // stop here with the one error that matters.
                    let _ = this.update(cx, |this, cx| {
                        this.cli_setup.running = None;
                        *this.cli_setup.node_stage.lock().unwrap() = None;
                        this.refresh_cli_environment(cx);
                        this.cli_setup.last_error = Some(outcome.output);
                        cx.notify();
                    });
                    return;
                }
                let _ = this.update(cx, |this, _| {
                    *this.cli_setup.node_stage.lock().unwrap() = None;
                });
            }

            // The install context is built once, off-thread: resolving the
            // shell PATH may spawn a shell. A Node installed a moment ago
            // outranks whatever detection saw before it existed.
            let context = cx
                .background_executor()
                .spawn(async move {
                    let managed = sub2api::node_install::managed_node_bin_dir()
                        .filter(|dir| dir.is_dir());
                    let node_bin_dir = if needs_node {
                        managed.or(detected_node_dir)
                    } else {
                        detected_node_dir.or(managed)
                    };
                    sub2api::cli_install::InstallContext::new(node_bin_dir, cli_search_dirs())
                })
                .await;

            let mut notes: Vec<String> = Vec::new();
            for descriptor in queue {
                let id = descriptor.id.to_owned();
                let _ = this.update(cx, |this, cx| {
                    this.cli_setup.running = Some(id.clone());
                    cx.notify();
                });
                let context = context.clone();
                let stage = stage.clone();
                let verdict = cx
                    .background_executor()
                    .spawn(async move {
                        sub2api::cli_install::install_cli(&context, descriptor, |reported| {
                            *stage.lock().unwrap() = Some(install_stage_label(reported));
                        })
                    })
                    .await;
                let _ = this.update(cx, |this, _| {
                    *this.cli_setup.node_stage.lock().unwrap() = None;
                    if verdict.is_usable() {
                        this.cli_setup.selected.borrow_mut().remove(&id);
                    }
                    this.cli_setup
                        .install_results
                        .insert(id.clone(), verdict.clone());
                });
                match verdict {
                    sub2api::cli_install::InstallVerdict::Installed { .. } => installed += 1,
                    sub2api::cli_install::InstallVerdict::InstalledNotOnPath { bin_dir, .. } => {
                        installed += 1;
                        notes.push(tr!(
                            "cli_setup.installed_not_on_path",
                            name = descriptor.display_name,
                            dir = bin_dir.display().to_string()
                        ));
                    }
                    other => failures.push(format!(
                        "{}:\n{}",
                        descriptor.display_name,
                        install_verdict_detail(&other)
                    )),
                }
            }
            // A CLI that works only from a remembered directory is still a
            // success; the note tells the user their own terminal may differ.
            failures.extend(notes);

            let _ = this.update(cx, |this, cx| {
                this.cli_setup.running = None;
                this.refresh_cli_environment(cx);
                // A fresh install must show up everywhere without a manual
                // refresh: the Providers rows above and the model picker.
                if installed > 0 {
                    this.refresh_provider_detection(None);
                }
                if failures.is_empty() {
                    this.show_toast(tr!("cli_setup.installed"));
                } else {
                    if installed > 0 {
                        this.show_toast(tr!("cli_setup.installed"));
                    }
                    this.cli_setup.last_error = Some(failures.join("\n\n"));
                }
                cx.notify();
            });
        })
        .detach();
    }
    /// The stored custom endpoints, cached — reading a file on every render
    /// frame would not do. Invalidated on save.
    pub(super) fn custom_api_snapshot(&self) -> sub2api::custom_api::CustomApiConfig {
        if let Some(config) = self.cli_setup.custom_cache.borrow().clone() {
            return config;
        }
        let config = sub2api::custom_api::load();
        *self.cli_setup.custom_cache.borrow_mut() = Some(config.clone());
        config
    }

}

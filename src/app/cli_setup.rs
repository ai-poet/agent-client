//! Settings → Providers: what to install when an agent CLI is missing.
//!
//! Fork addition. Upstream tells the user a CLI was "not detected" and stops
//! there; this section says what to run. The plan itself — which CLIs are
//! missing, whether Node is new enough, and the exact command per platform —
//! comes from the `sub2api::cli_install` module, which is unit-tested without
//! GPUI.
//!
//! The section renders nothing when the machine is already set up, so a
//! correctly configured install sees no extra chrome.

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
    /// Stage label for the Node install, written by the background installer
    /// and read by a UI poll loop while it runs.
    pub node_stage: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    /// CLI ids ticked for installation. Interior mutability: the default tick
    /// is seeded during render, which only has `&self`.
    pub selected: std::cell::RefCell<std::collections::HashSet<String>>,
    /// The default tick happens once; after that the user's choices stand,
    /// even across detection refreshes.
    selection_seeded: std::cell::Cell<bool>,
    /// Cached detection result. Interior mutability because detection is
    /// filled in during render, which only has `&self`.
    cache: std::cell::RefCell<Option<CliSetupSnapshot>>,
    cached_at: std::cell::Cell<Option<Instant>>,
}

impl CliSetupState {
    /// Throw the cache away; the next render re-detects.
    pub fn invalidate(&self) {
        self.cached_at.set(None);
    }
}

/// What detection found, at one point in time.
#[derive(Clone)]
struct CliSetupSnapshot {
    node_version: Option<String>,
    plan: Vec<sub2api::cli_install::SetupStep>,
}

/// How long a detection result stays good.
///
/// Detection spawns `node --version`, and render runs on every notify — typing
/// in any field on the page would otherwise start a process per keystroke.
const DETECTION_TTL: Duration = Duration::from_secs(5);

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

/// Which unattended toolchain installer to run.
#[derive(Clone, Copy)]
pub(super) enum ToolchainKind {
    Node,
    Git,
}

impl ToolchainKind {
    /// Id used in the `running` field and the progress poll.
    fn id(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Git => "git",
        }
    }
}

impl Waku {
    /// Install Node or Git unattended, reporting each stage as it starts.
    pub(super) fn run_toolchain_install(&mut self, kind: ToolchainKind, cx: &mut Context<Self>) {
        if self.cli_setup.running.is_some() {
            return;
        }
        self.cli_setup.running = Some(kind.id().to_owned());
        self.cli_setup.last_error = None;
        *self.cli_setup.node_stage.lock().unwrap() = Some(tr!("cli_setup.stage_resolving"));
        cx.notify();

        // The installer reports stages from the background thread; this poll
        // loop moves them onto the screen. 300ms is imperceptible next to a
        // download measured in tens of seconds.
        let poll_id = kind.id();
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
                    let report = |reported: sub2api::node_install::NodeStage| {
                        let label = match reported {
                            sub2api::node_install::NodeStage::ResolvingDownload => {
                                tr!("cli_setup.stage_resolving")
                            }
                            sub2api::node_install::NodeStage::Downloading => {
                                tr!("cli_setup.stage_downloading")
                            }
                            sub2api::node_install::NodeStage::Installing { method } => {
                                tr!("cli_setup.stage_installing", method = method)
                            }
                            sub2api::node_install::NodeStage::Verifying => {
                                tr!("cli_setup.stage_verifying")
                            }
                        };
                        *stage.lock().unwrap() = Some(label);
                    };
                    match kind {
                        ToolchainKind::Node => sub2api::node_install::install_node(report),
                        ToolchainKind::Git => sub2api::git_install::install_git(report),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.cli_setup.running = None;
                *this.cli_setup.node_stage.lock().unwrap() = None;
                this.cli_setup.invalidate();
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

    /// Toggle one CLI's tick.
    pub(super) fn toggle_cli_selection(&mut self, id: String, cx: &mut Context<Self>) {
        let mut selected = self.cli_setup.selected.borrow_mut();
        if !selected.remove(&id) {
            selected.insert(id);
        }
        drop(selected);
        cx.notify();
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
        let queue: Vec<(String, Vec<String>)> = {
            let selected = self.cli_setup.selected.borrow();
            sub2api::cli_install::DESCRIPTORS
                .iter()
                .filter(|descriptor| selected.contains(descriptor.id))
                .map(|descriptor| {
                    (
                        descriptor.id.to_owned(),
                        sub2api::cli_install::install_candidates(descriptor.package),
                    )
                })
                .collect()
        };
        if queue.is_empty() {
            return;
        }
        let needs_node = !sub2api::node_install::detect_node()
            .as_deref()
            .is_some_and(sub2api::cli_install::node_is_supported);

        self.cli_setup.last_error = None;
        self.cli_setup.running = Some(if needs_node {
            "node".to_owned()
        } else {
            queue[0].0.clone()
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
                        this.cli_setup.invalidate();
                        this.cli_setup.last_error = Some(outcome.output);
                        cx.notify();
                    });
                    return;
                }
                let _ = this.update(cx, |this, _| {
                    *this.cli_setup.node_stage.lock().unwrap() = None;
                });
            }

            for (id, commands) in queue {
                let _ = this.update(cx, |this, cx| {
                    this.cli_setup.running = Some(id.clone());
                    cx.notify();
                });
                let outcome = cx
                    .background_executor()
                    .spawn(async move { sub2api::cli_install::run_candidates(&commands) })
                    .await;
                if outcome.success {
                    installed += 1;
                    let _ = this.update(cx, |this, _| {
                        this.cli_setup.selected.borrow_mut().remove(&id);
                    });
                } else {
                    failures.push(format!("{id}:\n{}", outcome.output));
                }
            }

            let _ = this.update(cx, |this, cx| {
                this.cli_setup.running = None;
                this.cli_setup.invalidate();
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
    /// Detect what is missing and render the commands that fix it.
    pub(super) fn render_cli_setup_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::current(cx);
        let snapshot = self.cli_setup_snapshot();
        let node_version = snapshot.node_version;
        let plan = snapshot.plan;
        if plan.is_empty() {
            return div().into_any_element();
        }

        let mut section = div()
            .mt(px(18.0))
            .w_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .text_size(sp(13.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(tr!("cli_setup.title")),
            );

        // The default tick: the two agents this product is built around.
        // Seeded once; after that the user's choices stand.
        if !self.cli_setup.selection_seeded.get() {
            let mut selected = self.cli_setup.selected.borrow_mut();
            for step in &plan {
                if let sub2api::cli_install::SetupStep::InstallCli { id, .. } = step
                    && matches!(*id, "claude" | "codex")
                {
                    selected.insert((*id).to_owned());
                }
            }
            self.cli_setup.selection_seeded.set(true);
        }

        let busy = self.cli_setup.running.is_some();
        let mut selectable = 0usize;
        let mut ticked = 0usize;

        for step in plan {
            match step {
                sub2api::cli_install::SetupStep::InstallNode { required_major } => {
                    let running = self.cli_setup.running.as_deref() == Some("node");
                    // While the installer runs, the row narrates its progress.
                    let detail = if running {
                        self.cli_setup
                            .node_stage
                            .lock()
                            .unwrap()
                            .clone()
                            .unwrap_or_else(|| tr!("cli_setup.installing"))
                    } else {
                        match node_version.as_deref() {
                            Some(version) => {
                                tr!("cli_setup.node_found", version = version.trim())
                            }
                            None => tr!("cli_setup.node_missing"),
                        }
                    };
                    // Linux stays report-only: distro package managers own
                    // Node there, same as the old client.
                    let action = sub2api::node_install::install_supported()
                        .then_some(ToolchainKind::Node);
                    section = section.child(toolchain_row(
                        theme,
                        tr!("cli_setup.node_requirement", major = required_major),
                        detail,
                        action,
                        running,
                        busy,
                        cx,
                    ));
                }
                sub2api::cli_install::SetupStep::InstallGit => {
                    let running = self.cli_setup.running.as_deref() == Some("git");
                    let detail = if running {
                        self.cli_setup
                            .node_stage
                            .lock()
                            .unwrap()
                            .clone()
                            .unwrap_or_else(|| tr!("cli_setup.installing"))
                    } else {
                        tr!("cli_setup.git_missing")
                    };
                    let action = sub2api::git_install::install_supported()
                        .then_some(ToolchainKind::Git);
                    section = section.child(toolchain_row(
                        theme,
                        tr!("cli_setup.git_requirement"),
                        detail,
                        action,
                        running,
                        busy,
                        cx,
                    ));
                }
                sub2api::cli_install::SetupStep::InstallCli {
                    id,
                    display_name,
                    commands,
                } => {
                    selectable += 1;
                    let checked = self.cli_setup.selected.borrow().contains(id);
                    if checked {
                        ticked += 1;
                    }
                    let running = self.cli_setup.running.as_deref() == Some(id);
                    section = section.child(cli_row(
                        theme,
                        id,
                        display_name,
                        commands,
                        checked,
                        running,
                        busy,
                        cx,
                    ));
                }
            }
        }

        // One action for all ticked agents, like an installer's component
        // page — five separate install buttons were five decisions too many.
        if selectable > 0 {
            let disabled = busy || ticked == 0;
            section = section.child(
                div().flex().justify_end().child(
                    div()
                        .id("install-selected-clis")
                        .tab_index(0)
                        .h(px(29.0))
                        .px(px(12.0))
                        .rounded(px(7.0))
                        .border_1()
                        .border_color(theme.border_strong)
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_default()
                        .text_size(sp(12.5))
                        .text_color(theme.text_secondary)
                        .opacity(if disabled { 0.55 } else { 1.0 })
                        .child(if busy {
                            tr!("cli_setup.installing")
                        } else {
                            tr!("cli_setup.install_selected", count = ticked)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if disabled {
                                return;
                            }
                            this.run_selected_cli_installs(cx);
                        })),
                ),
            );
        }

        if let Some(error) = self.cli_setup.last_error.clone() {
            section = section.child(
                div()
                    .w_full()
                    .px(px(20.0))
                    .py(px(14.0))
                    .rounded(px(13.0))
                    .bg(theme.raised)
                    .child(
                        div()
                            .text_size(sp(12.0))
                            .line_height(sp(17.0))
                            .text_color(theme.text_secondary)
                            .child(error),
                    ),
            );
        }

        section.into_any_element()
    }

    /// Current detection, recomputed at most once per [`DETECTION_TTL`].
    fn cli_setup_snapshot(&self) -> CliSetupSnapshot {
        let fresh = self
            .cli_setup
            .cached_at
            .get()
            .is_some_and(|at| at.elapsed() < DETECTION_TTL);
        if fresh && let Some(snapshot) = self.cli_setup.cache.borrow().clone() {
            return snapshot;
        }
        // detect_node probes the managed runtime and the MSI location too, so
        // a Node installed a moment ago turns the row green without a restart.
        let node_version = sub2api::node_install::detect_node();
        let git_complete = sub2api::git_install::git_is_complete();
        let detections = sub2api::cli_install::detect_all();
        let plan =
            sub2api::cli_install::setup_plan(node_version.as_deref(), git_complete, &detections);
        let snapshot = CliSetupSnapshot { node_version, plan };
        *self.cli_setup.cache.borrow_mut() = Some(snapshot.clone());
        self.cli_setup.cached_at.set(Some(Instant::now()));
        snapshot
    }
}

/// A prerequisite row (Node, Git): title, status, and its own Install button.
fn toolchain_row(
    theme: Theme,
    title: String,
    detail: String,
    action: Option<ToolchainKind>,
    running: bool,
    busy: bool,
    cx: &mut Context<Waku>,
) -> Div {
    let row = div()
        .w_full()
        .px(px(20.0))
        .py(px(14.0))
        .rounded(px(13.0))
        .bg(theme.raised)
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(sp(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.text)
                        .child(title),
                )
                .child(
                    div()
                        .mt(px(4.0))
                        .text_size(sp(12.5))
                        .line_height(sp(18.0))
                        .text_color(theme.text_secondary)
                        .truncate()
                        .child(detail),
                ),
        );

    let Some(kind) = action else {
        // No unattended route on this platform; the row states the
        // requirement and leaves the choice of package manager to the user.
        return row;
    };
    row.child(
        div()
            .id("run-toolchain-install")
            .tab_index(0)
            .h(px(28.0))
            .px(px(11.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(theme.border_strong)
            .flex()
            .items_center()
            .justify_center()
            .cursor_default()
            .text_size(sp(12.5))
            .text_color(theme.text_secondary)
            .opacity(if busy { 0.55 } else { 1.0 })
            .child(if running {
                tr!("cli_setup.installing")
            } else {
                tr!("cli_setup.install")
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                if busy {
                    return;
                }
                this.run_toolchain_install(kind, cx);
            })),
    )
}

/// A selectable agent row: checkbox, name, command preview, and a copy button.
#[allow(clippy::too_many_arguments)]
fn cli_row(
    theme: Theme,
    id: &'static str,
    display_name: &'static str,
    commands: Vec<String>,
    checked: bool,
    running: bool,
    busy: bool,
    cx: &mut Context<Waku>,
) -> impl IntoElement {
    let command_preview = commands.first().cloned().unwrap_or_default();
    let copy_command = command_preview.clone();

    // The whole row toggles; a 15px box alone is a needlessly hard target.
    let checkbox = div()
        .flex_none()
        .size(px(15.0))
        .rounded(px(4.0))
        .border_1()
        .border_color(if checked {
            theme.accent
        } else {
            theme.border_strong
        })
        .when(checked, |element| element.bg(theme.accent))
        .flex()
        .items_center()
        .justify_center()
        .when(checked, |element| {
            element.child(
                div()
                    .text_size(sp(10.0))
                    .text_color(theme.raised)
                    .child("\u{2713}"),
            )
        });

    div()
        .id(SharedString::from(format!("cli-select-{id}")))
        .tab_index(0)
        .w_full()
        .px(px(20.0))
        .py(px(12.0))
        .rounded(px(13.0))
        .bg(theme.raised)
        .cursor_default()
        .opacity(if busy && !running { 0.7 } else { 1.0 })
        .flex()
        .items_center()
        .gap(px(12.0))
        .child(checkbox)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(sp(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.text)
                        .child(display_name),
                )
                .child(
                    div()
                        .mt(px(3.0))
                        .text_size(sp(12.0))
                        .text_color(theme.text_ghost)
                        .truncate()
                        .child(if running {
                            tr!("cli_setup.installing")
                        } else {
                            command_preview
                        }),
                ),
        )
        // Copy stays: when an install fails for reasons the app cannot fix —
        // a proxy, a root-owned prefix — the user needs the exact command.
        .child(
            div()
                .id(SharedString::from(format!("cli-copy-{id}")))
                .tab_index(0)
                .h(px(26.0))
                .px(px(9.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(theme.border_strong)
                .flex()
                .items_center()
                .justify_center()
                .cursor_default()
                .text_size(sp(12.0))
                .text_color(theme.text_secondary)
                .child(tr!("cli_setup.copy"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    cx.write_to_clipboard(ClipboardItem::new_string(copy_command.clone()));
                    this.show_toast(tr!("cli_setup.copied"));
                })),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            if busy {
                return;
            }
            this.toggle_cli_selection(id.to_owned(), cx);
        }))
}

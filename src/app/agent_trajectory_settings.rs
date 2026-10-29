//! Settings → Agent → Trajectory: whether the built-in agent records one.
//!
//! Fork addition (Trajectory, `trajectory_panel.rs`). The switch lives in its
//! own `trajectory.json` beside the engine's `settings.json`, as AgentTeams'
//! and auto-memory's do — the engine's save drops keys it does not know —
//! and the bridge reads it when a session starts. Read and written off the
//! UI thread; a frame only reads what the last read left.

use super::*;

const FILE: &str = "trajectory.json";

#[derive(Default)]
pub(super) struct AgentTrajectoryPageState {
    /// `None` until the file has been read.
    enabled: Option<bool>,
    /// A read is under way (set from a frame, hence a `Cell`).
    loading: std::cell::Cell<bool>,
    saving: bool,
}

fn path() -> Option<PathBuf> {
    sub2api::global_config::native::config_dir().map(|dir| dir.join(FILE))
}

/// Whether the file switches recording on; absent or unreadable means on.
fn read_enabled() -> bool {
    path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| value.get("enabled").and_then(serde_json::Value::as_bool))
        .unwrap_or(true)
}

fn write_enabled(enabled: bool) -> std::io::Result<()> {
    let path = path().ok_or_else(|| std::io::Error::other("no settings directory"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::json!({ "enabled": enabled }).to_string())
}

impl Waku {
    fn load_agent_trajectory_setting(&self, cx: &mut Context<Self>) {
        let state = &self.agent_page.trajectory;
        if state.loading.get() || state.enabled.is_some() {
            return;
        }
        state.loading.set(true);
        cx.spawn(async move |this, cx| {
            let enabled = cx
                .background_executor()
                .spawn(async move { read_enabled() })
                .await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.agent_page.trajectory;
                state.loading.set(false);
                state.enabled = Some(enabled);
                cx.notify();
            });
        })
        .detach();
    }

    fn set_agent_trajectory_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let state = &mut self.agent_page.trajectory;
        if state.saving {
            return;
        }
        let previous = state.enabled;
        state.enabled = Some(enabled);
        state.saving = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let saved = cx
                .background_executor()
                .spawn(async move { write_enabled(enabled) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.agent_page.trajectory;
                state.saving = false;
                if let Err(error) = saved {
                    state.enabled = previous;
                    this.show_toast(error.to_string());
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_agent_trajectory_section(
        &self,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        self.load_agent_trajectory_setting(cx);
        let state = &self.agent_page.trajectory;
        let enabled = state.enabled.unwrap_or(true);
        let busy = state.saving || state.enabled.is_none();
        super::agent_page::section_card(theme)
            .child(super::agent_page::section_title(
                theme,
                tr!("trajectory.settings_title"),
            ))
            .child(super::agent_page::setting_row(
                theme,
                tr!("trajectory.settings_title"),
                tr!("trajectory.settings_detail"),
                toggle_switch(
                    "agent-trajectory-enabled",
                    enabled,
                    busy,
                    theme,
                    cx,
                    move |this, _, cx| this.set_agent_trajectory_enabled(!enabled, cx),
                )
                .into_any_element(),
            ))
    }
}

//! Settings → Agent → Memory: the built-in agent's persistent memory.
//!
//! Fork addition (`auto_memory`, translated from dsh-auto-memory). The
//! settings live in `auto-memory.json` beside the engine's own settings file
//! — the engine rewrites `settings.json` from its typed struct and would drop
//! a key it does not know. The memories are Markdown files under
//! `<config dir>/auto-memory/`; this section lists them by scope and edits,
//! pins and deletes them through the same `MemoryStore` the agent's tools
//! use, so the index is rebuilt and a project memory's Claude Code mirror
//! follows. Every read and write runs on the background executor; render
//! does no I/O. A running session keeps the settings it started with.
//!
//! The page reads this machine's config directory: with a remote daemon it
//! shows the local memories, not the remote's.

use std::path::{Path, PathBuf};

use auto_memory::scan::MemoryGroup;
use auto_memory::{MemoryConfig, MemoryDraft, MemoryRecord, MemoryScope, MemoryStore, MemoryType, ScopeDir};

use super::providers_page::card_button;
use super::settings::abbreviate_home_path;
use super::*;

use crate::ui::ActivationExt as _;
use crate::ui::text_field::TextField;

#[derive(Default)]
pub(super) struct AgentMemoryPageState {
    pub config: Option<MemoryConfig>,
    pub loading: bool,
    pub saving: bool,
    pub error: Option<String>,
    /// Every memory on disk, by scope; `None` until the first scan lands.
    pub groups: Option<Vec<MemoryGroup>>,
    /// A scan or a change to the files is in flight.
    pub busy: bool,
    /// Bumped by every scan; a result from an older one is dropped.
    pub generation: u64,
    /// The open row: its scope directory and the memory's name.
    pub expanded: Option<(PathBuf, String)>,
    /// The memory editor, while open.
    pub editor: Option<MemoryEditor>,
}

pub(super) struct MemoryEditor {
    pub dir: ScopeDir,
    pub name: String,
    pub kind: MemoryType,
    pub title: Entity<TextInput>,
    pub description: Entity<TextInput>,
    pub body: Entity<TextInput>,
    pub error: Option<String>,
}

fn config_dir() -> Result<PathBuf, String> {
    sub2api::global_config::native::config_dir().ok_or_else(|| "no home directory".to_owned())
}

fn load_config(dir: &Path) -> Result<MemoryConfig, String> {
    auto_memory::config::load(&dir.join(auto_memory::config::CONFIG_FILE))
        .map_err(|error| format!("{error:#}"))
}

fn save_config(config: &MemoryConfig) -> Result<(), String> {
    let dir = config_dir()?;
    auto_memory::config::save(&dir.join(auto_memory::config::CONFIG_FILE), config)
        .map_err(|error| format!("{error:#}"))
}

/// Run `op` against the store the settings describe, then scan — the scan
/// runs even when `op` fails, so the list shows what is really on disk.
fn change_and_scan(
    config: &MemoryConfig,
    op: impl FnOnce(&MemoryStore) -> anyhow::Result<()>,
) -> (Result<(), String>, Vec<MemoryGroup>) {
    let dir = match config_dir() {
        Ok(dir) => dir,
        Err(error) => return (Err(error), Vec::new()),
    };
    let store = auto_memory::store_for(&dir, config);
    let result = op(&store).map_err(|error| format!("{error:#}"));
    (result, auto_memory::scan::scan_groups(&store))
}

/// The memory as it is on disk now, not as the last scan saw it.
fn current(store: &MemoryStore, dir: &ScopeDir, name: &str) -> anyhow::Result<MemoryRecord> {
    store
        .read(dir, name)?
        .ok_or_else(|| anyhow::anyhow!(tr!("memory.settings.gone", name = name)))
}

fn format_kb(bytes: usize) -> String {
    format!("{:.1} KB", bytes as f64 / 1024.0)
}

fn type_label(kind: MemoryType) -> String {
    match kind {
        MemoryType::User => tr!("memory.settings.type_user"),
        MemoryType::Feedback => tr!("memory.settings.type_feedback"),
        MemoryType::Project => tr!("memory.settings.type_project"),
        MemoryType::Reference => tr!("memory.settings.type_reference"),
    }
}

impl Waku {
    pub(super) fn ensure_agent_memory_loaded(&mut self, cx: &mut Context<Self>) {
        let state = &mut self.agent_page.memory;
        if state.loading || state.config.is_some() {
            return;
        }
        state.loading = true;
        state.generation += 1;
        let generation = state.generation;
        cx.spawn(async move |this, cx| {
            let (config, error, groups) = cx
                .background_executor()
                .spawn(async {
                    let dir = match config_dir() {
                        Ok(dir) => dir,
                        Err(error) => return (MemoryConfig::default(), Some(error), Vec::new()),
                    };
                    let (config, error) = match load_config(&dir) {
                        Ok(config) => (config, None),
                        Err(error) => (MemoryConfig::default(), Some(error)),
                    };
                    let store = auto_memory::store_for(&dir, &config);
                    let groups = auto_memory::scan::scan_groups(&store);
                    (config, error, groups)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.agent_page.memory;
                state.loading = false;
                state.config = Some(config);
                state.error = error;
                if state.generation == generation {
                    state.groups = Some(groups);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Forget everything loaded and read it again.
    pub(super) fn reload_agent_memory(&mut self, cx: &mut Context<Self>) {
        let state = &mut self.agent_page.memory;
        state.config = None;
        state.error = None;
        state.editor = None;
        self.ensure_agent_memory_loaded(cx);
    }

    /// Apply a change to the settings and write the file.
    fn update_agent_memory_config(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut MemoryConfig)) {
        let state = &mut self.agent_page.memory;
        let Some(config) = state.config.as_mut() else {
            return;
        };
        let stale_before = config.stale_after_days();
        let mirror_before = config.mirror_to_claude_code;
        edit(config);
        let snapshot = config.clone();
        // Eviction is evaluated when an index is rebuilt, so a new stale
        // threshold rebuilds every index now rather than at the next write.
        let restale = snapshot.stale_after_days() != stale_before;
        // Memories written while the mirror was off get their copy now.
        let remirror = snapshot.mirror_to_claude_code && !mirror_before;
        state.saving = true;
        state.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { save_config(&snapshot) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.agent_page.memory.saving = false;
                match result {
                    Ok(()) if restale || remirror => this.change_agent_memory(cx, move |store| {
                        for group in auto_memory::scan::scan_groups(store) {
                            if restale {
                                store.refresh_index(&group.dir);
                            }
                            if remirror {
                                store.sync_mirror(&group.dir);
                            }
                        }
                        Ok(())
                    }),
                    Ok(()) => {}
                    Err(error) => this.agent_page.memory.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Change the files on the background executor, then scan again.
    fn change_agent_memory(
        &mut self,
        cx: &mut Context<Self>,
        op: impl FnOnce(&MemoryStore) -> anyhow::Result<()> + Send + 'static,
    ) {
        let state = &mut self.agent_page.memory;
        let Some(config) = state.config.clone() else {
            return;
        };
        state.busy = true;
        state.error = None;
        state.generation += 1;
        let generation = state.generation;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (result, groups) = cx
                .background_executor()
                .spawn(async move { change_and_scan(&config, op) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.agent_page.memory;
                if let Err(error) = result {
                    state.error = Some(error);
                }
                if state.generation == generation {
                    state.busy = false;
                    state.groups = Some(groups);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_agent_memory_row(&mut self, dir: PathBuf, name: String, cx: &mut Context<Self>) {
        let state = &mut self.agent_page.memory;
        let key = (dir, name);
        state.expanded = if state.expanded.as_ref() == Some(&key) {
            None
        } else {
            Some(key)
        };
        cx.notify();
    }

    fn pin_agent_memory(&mut self, dir: ScopeDir, name: String, cx: &mut Context<Self>) {
        self.change_agent_memory(cx, move |store| {
            let record = current(store, &dir, &name)?;
            store.write(
                &dir,
                MemoryDraft {
                    name: record.name,
                    title: record.title,
                    description: record.description,
                    kind: record.kind,
                    body: record.body,
                    pinned: Some(!record.pinned),
                },
            )?;
            Ok(())
        });
    }

    fn confirm_delete_agent_memory(&mut self, dir: ScopeDir, record: &MemoryRecord, cx: &mut Context<Self>) {
        let name = record.name.clone();
        let detail = if dir.scope == MemoryScope::Project {
            tr!("memory.settings.delete_detail_project")
        } else {
            tr!("memory.settings.delete_detail")
        };
        self.request_confirm(
            tr!("memory.settings.delete_title", name = record.heading()),
            Some(detail),
            tr!("memory.settings.delete"),
            true,
            cx,
            move |this, _, cx| {
                if this.agent_page.memory.editor.as_ref().is_some_and(|editor| editor.name == name) {
                    this.agent_page.memory.editor = None;
                }
                this.change_agent_memory(cx, move |store| store.delete(&dir, &name).map(|_| ()));
            },
        );
    }

    fn confirm_clear_agent_memory(&mut self, dir: ScopeDir, label: String, count: usize, cx: &mut Context<Self>) {
        let detail = if dir.scope == MemoryScope::Project {
            tr!("memory.settings.clear_detail_project", count = count)
        } else {
            tr!("memory.settings.clear_detail", count = count)
        };
        self.request_confirm(
            tr!("memory.settings.clear_title", scope = label),
            Some(detail),
            tr!("memory.settings.clear"),
            true,
            cx,
            move |this, _, cx| {
                this.agent_page.memory.editor = None;
                this.change_agent_memory(cx, move |store| store.clear(&dir).map(|_| ()));
            },
        );
    }

    fn open_agent_memory_editor(
        &mut self,
        dir: ScopeDir,
        record: &MemoryRecord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let field = |text: String, placeholder: String, multi_line: bool, window: &mut Window, cx: &mut Context<Self>| {
            let input = cx.new(|cx| {
                let mut input = TextInput::new(window, cx).placeholder(placeholder);
                if multi_line {
                    input = input.multi_line();
                }
                input.set_content(text, cx);
                input
            });
            cx.subscribe(&input, |_: &mut Self, _, _: &InputEvent, cx| cx.notify())
                .detach();
            input
        };
        let title = field(
            record.title.clone().unwrap_or_default(),
            tr!("memory.settings.edit_title_placeholder"),
            false,
            window,
            cx,
        );
        let description = field(
            record.description.clone(),
            tr!("memory.settings.edit_description_placeholder"),
            false,
            window,
            cx,
        );
        let body = field(
            record.body.clone(),
            tr!("memory.settings.edit_body_placeholder"),
            true,
            window,
            cx,
        );
        self.agent_page.memory.editor = Some(MemoryEditor {
            dir,
            name: record.name.clone(),
            kind: record.kind,
            title,
            description,
            body,
            error: None,
        });
        cx.notify();
    }

    fn commit_agent_memory_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.agent_page.memory.editor.as_mut() else {
            return;
        };
        let title = editor.title.read(cx).content().trim().to_owned();
        let description = editor.description.read(cx).content().trim().to_owned();
        let body = editor.body.read(cx).content().to_owned();
        if description.is_empty() {
            editor.error = Some(tr!("memory.settings.description_required"));
            cx.notify();
            return;
        }
        let Some(editor) = self.agent_page.memory.editor.take() else {
            return;
        };
        let draft = MemoryDraft {
            name: editor.name,
            title: (!title.is_empty()).then_some(title),
            description,
            kind: editor.kind,
            body,
            // The pin stays as it is on disk.
            pinned: None,
        };
        let dir = editor.dir;
        self.change_agent_memory(cx, move |store| store.write(&dir, draft).map(|_| ()));
    }

    // ---- render -----------------------------------------------------------

    pub(super) fn render_agent_memory_section(&self, theme: Theme, cx: &mut Context<Self>) -> Div {
        let state = &self.agent_page.memory;
        let Some(config) = state.config.as_ref() else {
            return super::agent_page::section_card(theme)
                .child(super::agent_page::section_title(
                    theme,
                    tr!("memory.settings.title"),
                ))
                .child(super::agent_page::section_description(
                    theme,
                    tr!("common.loading"),
                ));
        };
        let saving = state.saving;
        let enabled = config.enabled;
        let mut card = super::agent_page::section_card(theme)
            .child(super::agent_page::section_title(
                theme,
                tr!("memory.settings.title"),
            ))
            .child(super::agent_page::section_description(
                theme,
                tr!("memory.settings.description"),
            ))
            .child(super::agent_page::setting_row(
                theme,
                tr!("memory.settings.enabled"),
                tr!("memory.settings.enabled_description"),
                toggle_switch(
                    "agent-memory-enabled",
                    enabled,
                    saving,
                    theme,
                    cx,
                    move |this, _, cx| {
                        this.update_agent_memory_config(cx, move |config| config.enabled = !enabled)
                    },
                )
                .into_any_element(),
            ));
        if !enabled {
            return card.when_some(state.error.clone(), |card, error| {
                card.child(error_line(theme, error))
            });
        }

        let user_scope = config.enable_user_scope;
        card = card.child(super::agent_page::setting_row(
            theme,
            tr!("memory.settings.user_scope"),
            tr!("memory.settings.user_scope_description"),
            toggle_switch(
                "agent-memory-user-scope",
                user_scope,
                saving,
                theme,
                cx,
                move |this, _, cx| {
                    this.update_agent_memory_config(cx, move |config| {
                        config.enable_user_scope = !user_scope
                    })
                },
            )
            .into_any_element(),
        ));

        let auto = config.auto_summarize;
        card = card.child(super::agent_page::setting_row(
            theme,
            tr!("memory.settings.auto_summarize"),
            tr!(
                "memory.settings.auto_summarize_description",
                turns = config.auto_summarize_every_turns(),
                count = config.auto_summarize_max_memories()
            ),
            toggle_switch(
                "agent-memory-auto",
                auto,
                saving,
                theme,
                cx,
                move |this, _, cx| {
                    this.update_agent_memory_config(cx, move |config| config.auto_summarize = !auto)
                },
            )
            .into_any_element(),
        ));

        let mirror = config.mirror_to_claude_code;
        card = card.child(super::agent_page::setting_row(
            theme,
            tr!("memory.settings.mirror"),
            tr!("memory.settings.mirror_description"),
            toggle_switch(
                "agent-memory-mirror",
                mirror,
                saving,
                theme,
                cx,
                move |this, _, cx| {
                    this.update_agent_memory_config(cx, move |config| {
                        config.mirror_to_claude_code = !mirror
                    })
                },
            )
            .into_any_element(),
        ));

        let current_bytes = config.max_bytes();
        let mut budgets = div().flex().flex_wrap().gap(px(6.0));
        for bytes in auto_memory::config::MAX_BYTES_PRESETS {
            budgets = budgets.child(super::agent_page::preset_button(
                theme,
                SharedString::from(format!("agent-memory-budget-{bytes}")),
                format!("{} KB", bytes / 1024),
                bytes as usize == current_bytes,
                cx,
                move |this, _, cx| {
                    this.update_agent_memory_config(cx, move |config| config.max_bytes = bytes)
                },
            ));
        }
        card = card.child(super::agent_page::setting_row(
            theme,
            tr!("memory.settings.budget"),
            tr!("memory.settings.budget_description"),
            budgets.into_any_element(),
        ));

        let current_stale = config.stale_after_days();
        let mut stale = div().flex().flex_wrap().gap(px(6.0));
        for days in auto_memory::config::STALE_PRESETS {
            let label = if days == 0 {
                tr!("memory.settings.stale_off")
            } else {
                tr!("memory.settings.stale_days", days = days)
            };
            stale = stale.child(super::agent_page::preset_button(
                theme,
                SharedString::from(format!("agent-memory-stale-{days}")),
                label,
                days == current_stale,
                cx,
                move |this, _, cx| {
                    this.update_agent_memory_config(cx, move |config| {
                        config.stale_after_days = days
                    })
                },
            ));
        }
        card = card.child(super::agent_page::setting_row(
            theme,
            tr!("memory.settings.stale"),
            tr!("memory.settings.stale_description"),
            stale.into_any_element(),
        ));

        card = card.child(self.render_agent_memory_groups(config, theme, cx));
        card.when_some(state.error.clone(), |card, error| {
            card.child(error_line(theme, error))
        })
    }

    fn render_agent_memory_groups(&self, config: &MemoryConfig, theme: Theme, cx: &mut Context<Self>) -> Div {
        let state = &self.agent_page.memory;
        let busy = state.busy || state.saving;
        let budget = auto_memory::prompt::index_budget(config.max_bytes());
        let header = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(
                div()
                    .flex_1()
                    .text_size(sp(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(tr!("memory.settings.memories")),
            )
            .child(card_button(
                theme,
                SharedString::from("agent-memory-refresh"),
                tr!("common.refresh"),
                false,
                busy,
                cx,
                |this, _, cx| this.change_agent_memory(cx, |_| Ok(())),
            ));
        let mut block = div()
            .mt(px(16.0))
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .text_size(sp(12.0))
                    .line_height(sp(17.0))
                    .text_color(theme.text_tertiary)
                    .child(tr!("memory.settings.memories_description")),
            );
        let Some(groups) = state.groups.as_ref() else {
            return block.child(placeholder_line(theme, tr!("common.loading")));
        };
        if groups.is_empty() {
            return block.child(placeholder_line(theme, tr!("memory.settings.empty")));
        }
        let stale_days = config.stale_after_days();
        for group in groups {
            block = block.child(self.render_agent_memory_group(group, budget, stale_days, busy, theme, cx));
        }
        block
    }

    fn render_agent_memory_group(
        &self,
        group: &MemoryGroup,
        budget: usize,
        stale_days: u32,
        busy: bool,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let label = match group.scope() {
            MemoryScope::User => tr!("memory.settings.group_user"),
            MemoryScope::Project => group
                .project_path()
                .map(|path| abbreviate_home_path(path, self.home_directory.as_deref()))
                .or_else(|| group.key.clone())
                .unwrap_or_default(),
        };
        let count = group.memories.len();
        let key = group.key.clone().unwrap_or_else(|| "user".to_owned());
        let percent = if budget == 0 {
            100.0
        } else {
            group.index_bytes as f64 * 100.0 / budget as f64
        };
        let mut usage = tr!(
            "memory.settings.index_usage",
            used = format_kb(group.index_bytes),
            budget = format_kb(budget)
        );
        if group.index_bytes > budget {
            usage = format!("{usage} · {}", tr!("memory.settings.over_budget"));
        }
        if group.hidden > 0 {
            usage = format!(
                "{usage} · {}",
                tr!("memory.settings.hidden", count = group.hidden)
            );
        }

        let clear_dir = group.dir.clone();
        let clear_label = label.clone();
        let mut block = div()
            .mt(px(14.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_baseline()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(sp(12.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme.text)
                                    .child(label),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(sp(11.5))
                                    .text_color(theme.text_tertiary)
                                    .child(tr!("memory.settings.count", count = count)),
                            ),
                    )
                    .child(card_button(
                        theme,
                        SharedString::from(format!("agent-memory-clear-{key}")),
                        tr!("memory.settings.clear"),
                        false,
                        busy,
                        cx,
                        move |this, _, cx| {
                            this.confirm_clear_agent_memory(
                                clear_dir.clone(),
                                clear_label.clone(),
                                count,
                                cx,
                            )
                        },
                    )),
            )
            .child(super::usage_meter::meter_bar(&theme, percent))
            .child(
                div()
                    .text_size(sp(11.5))
                    .text_color(if group.index_bytes > budget {
                        theme.warning
                    } else {
                        theme.text_tertiary
                    })
                    .child(usage),
            );
        let now = auto_memory::now_ms();
        let mut rows = div().flex().flex_col();
        for record in &group.memories {
            let stale = auto_memory::store::is_stale(record, stale_days, now);
            rows = rows.child(self.render_agent_memory_row(group, &key, record, stale, busy, theme, cx));
        }
        block = block.child(rows);
        block
    }

    #[allow(clippy::too_many_arguments)]
    fn render_agent_memory_row(
        &self,
        group: &MemoryGroup,
        key: &str,
        record: &MemoryRecord,
        stale: bool,
        busy: bool,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let state = &self.agent_page.memory;
        let open = state
            .expanded
            .as_ref()
            .is_some_and(|(dir, name)| dir == &group.dir.dir && name == &record.name);
        let row_id = format!("agent-memory-row-{key}-{}", record.name);

        let mut chips = div().flex().flex_wrap().items_center().gap(px(6.0)).child(chip(theme, type_label(record.kind)));
        if record.pinned {
            chips = chips.child(chip(theme, tr!("memory.settings.pinned")));
        }
        if stale {
            chips = chips.child(chip(theme, tr!("memory.settings.stale_chip")));
        }
        let mut meta = record.name.clone();
        if let Some(reads) = record.reads.filter(|reads| *reads > 0) {
            meta = format!("{meta} · {}", tr!("memory.settings.reads", count = reads));
        }

        let toggle_dir = group.dir.dir.clone();
        let toggle_name = record.name.clone();
        let summary = div()
            .id(SharedString::from(row_id.clone()))
            .tab_index(0)
            .py(px(8.0))
            .px(px(6.0))
            .rounded(px(7.0))
            .flex()
            .items_start()
            .gap(px(8.0))
            .cursor_default()
            .hover(|element| element.bg(theme.overlay))
            .focus_visible(|style| style.bg(theme.overlay))
            .on_activation(cx, move |this, _, cx| {
                this.toggle_agent_memory_row(toggle_dir.clone(), toggle_name.clone(), cx)
            })
            .child(
                div().pt(px(2.0)).flex_none().child(icon(
                    if open {
                        "icons/chevron-down.svg"
                    } else {
                        "icons/chevron-right.svg"
                    },
                    12.0,
                    theme.text_tertiary,
                )),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .text_size(sp(12.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme.text)
                                    .child(record.heading().to_owned()),
                            )
                            .child(chips),
                    )
                    .child(
                        div()
                            .text_size(sp(12.0))
                            .line_height(sp(17.0))
                            .text_color(theme.text_secondary)
                            .child(record.description.clone()),
                    )
                    .child(
                        div()
                            .font_family(crate::md::render::MONO_FAMILY)
                            .text_size(sp(11.0))
                            .text_color(theme.text_ghost)
                            .child(meta),
                    ),
            );

        let mut row = div().flex().flex_col().border_b_1().border_color(theme.border).child(summary);
        if !open {
            return row;
        }
        let editing = state
            .editor
            .as_ref()
            .filter(|editor| editor.dir.dir == group.dir.dir && editor.name == record.name);
        if let Some(editor) = editing {
            return row.child(self.render_agent_memory_editor(editor, key, busy, theme, cx));
        }

        let body = record.body.trim();
        let edit_dir = group.dir.clone();
        let edit_record = record.clone();
        let pin_dir = group.dir.clone();
        let pin_name = record.name.clone();
        let delete_dir = group.dir.clone();
        let delete_record = record.clone();
        row = row.child(
            div()
                .pl(px(26.0))
                .pr(px(6.0))
                .pb(px(10.0))
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(
                    div()
                        .p(px(10.0))
                        .rounded(px(7.0))
                        .bg(theme.overlay)
                        .text_size(sp(12.0))
                        .line_height(sp(18.0))
                        .text_color(if body.is_empty() {
                            theme.text_tertiary
                        } else {
                            theme.text_secondary
                        })
                        .child(if body.is_empty() {
                            tr!("memory.settings.no_body")
                        } else {
                            body.to_owned()
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(8.0))
                        .child(card_button(
                            theme,
                            SharedString::from(format!("{row_id}-edit")),
                            tr!("common.edit"),
                            false,
                            busy,
                            cx,
                            move |this, window, cx| {
                                this.open_agent_memory_editor(edit_dir.clone(), &edit_record, window, cx)
                            },
                        ))
                        .child(card_button(
                            theme,
                            SharedString::from(format!("{row_id}-pin")),
                            if record.pinned {
                                tr!("memory.settings.unpin")
                            } else {
                                tr!("memory.settings.pin")
                            },
                            false,
                            busy,
                            cx,
                            move |this, _, cx| {
                                this.pin_agent_memory(pin_dir.clone(), pin_name.clone(), cx)
                            },
                        ))
                        .child(card_button(
                            theme,
                            SharedString::from(format!("{row_id}-delete")),
                            tr!("memory.settings.delete"),
                            false,
                            busy,
                            cx,
                            move |this, _, cx| {
                                this.confirm_delete_agent_memory(delete_dir.clone(), &delete_record, cx)
                            },
                        )),
                ),
        );
        row
    }

    fn render_agent_memory_editor(
        &self,
        editor: &MemoryEditor,
        key: &str,
        busy: bool,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let id = format!("agent-memory-editor-{key}-{}", editor.name);
        let mut kinds = div().flex().flex_wrap().gap(px(6.0));
        for kind in MemoryType::ALL {
            kinds = kinds.child(super::agent_page::preset_button(
                theme,
                SharedString::from(format!("{id}-kind-{}", kind.as_str())),
                type_label(kind),
                kind == editor.kind,
                cx,
                move |this, _, cx| {
                    if let Some(editor) = this.agent_page.memory.editor.as_mut() {
                        editor.kind = kind;
                        cx.notify();
                    }
                },
            ));
        }
        let label = |text: String| {
            div()
                .text_size(sp(12.0))
                .text_color(theme.text_tertiary)
                .child(text)
        };
        div()
            .ml(px(26.0))
            .mr(px(6.0))
            .mb(px(10.0))
            .p(px(12.0))
            .rounded(px(9.0))
            .bg(theme.overlay)
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(label(tr!("memory.settings.edit_title")))
            .child(TextField::new(
                SharedString::from(format!("{id}-title")),
                editor.title.clone(),
            ))
            .child(label(tr!("memory.settings.edit_description")))
            .child(TextField::new(
                SharedString::from(format!("{id}-description")),
                editor.description.clone(),
            ))
            .child(label(tr!("memory.settings.edit_type")))
            .child(kinds)
            .child(label(tr!("memory.settings.edit_body")))
            .child(div().min_h(px(140.0)).child(TextField::new(
                SharedString::from(format!("{id}-body")),
                editor.body.clone(),
            )))
            .when_some(editor.error.clone(), |element, error| {
                element.child(error_line(theme, error))
            })
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(card_button(
                        theme,
                        SharedString::from(format!("{id}-save")),
                        tr!("common.save"),
                        true,
                        busy,
                        cx,
                        |this, _, cx| this.commit_agent_memory_editor(cx),
                    ))
                    .child(card_button(
                        theme,
                        SharedString::from(format!("{id}-cancel")),
                        tr!("common.cancel"),
                        false,
                        false,
                        cx,
                        |this, _, cx| {
                            this.agent_page.memory.editor = None;
                            cx.notify();
                        },
                    )),
            )
    }
}

fn chip(theme: Theme, text: String) -> Div {
    div()
        .px(px(6.0))
        .py(px(1.0))
        .rounded(px(5.0))
        .border_1()
        .border_color(theme.border_strong)
        .text_size(sp(11.0))
        .text_color(theme.text_tertiary)
        .child(text)
}

fn placeholder_line(theme: Theme, text: String) -> Div {
    div()
        .py(px(8.0))
        .text_size(sp(12.5))
        .text_color(theme.text_tertiary)
        .child(text)
}

fn error_line(theme: Theme, error: String) -> Div {
    div()
        .mt(px(10.0))
        .text_size(sp(12.0))
        .line_height(sp(17.0))
        .text_color(theme.warning)
        .child(error)
}

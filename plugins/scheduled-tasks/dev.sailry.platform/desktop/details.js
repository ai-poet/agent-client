import {div} from "gpui-kit";
import {Icon, Tag} from "gpui-component";
import {theme} from "sailry";
import {ClampedText, EmptyState, IconButton, Loading, LoadMore, PanelHeader, SegmentedTabs} from "sailry/ui";
import {effortKey} from "./editor.js";
import {clock, dayLabel, localWeekday, relative, ribbon, streak, summary, tally, upcoming, wallClock,
  weeklyOffset, when, zoneWeekday, interval} from "./schedule.js";
export const DETAIL_WIDTH = 380;

// Theme colors are #rrggbbaa; tints keep the semantic hue with a lighter alpha.
const tint = (color, alpha) => `${color.slice(0, 7)}${alpha}`;
const RECENT = 14;
export function runColor(status, colors) {
  return {completed:colors.success, failed:colors.destructive, running:colors.info, queued:colors.info,
    unknown:colors.warning}[status] ?? colors.muted_foreground;
}
export function runLabel(text, status) { return text[status === "failed" ? "runFailed" : status] ?? text.finished; }
// Native activity wins, then the pause choice, then the latest outcome and the schedule.
export function taskStatus(text, item) {
  if (item.unavailable) return {label:text.executionUnavailable, tone:"muted"};
  if (item.status === "running") return {label:text.running, tone:"info"};
  if (item.status === "queued") return {label:text.queued, tone:"info"};
  if (!item.enabled) return {label:text.paused, tone:"muted"};
  if (item.status === "failed") return {label:text.runFailed, tone:"danger"};
  if (item.next_ms !== null && item.next_ms !== undefined) return {label:text.scheduled, tone:"neutral"};
  if (item.status === "completed") return {label:text.completed, tone:"success"};
  return {label:text[item.status] ?? text.finished, tone:"neutral"};
}
export function statusTag(view, item, id) {
  const {label, tone} = taskStatus(view.text, item);
  const tag = new Tag().variant(["danger", "info", "success"].includes(tone) ? tone : "secondary").size("medium").rounded_full();
  return div().id(id).flex_none().child((tone === "muted" ? tag.outline() : tag).child(label));
}
function cell(id, label, value, font, first) {
  const colors = theme().colors;
  let box = div().id(id).flex_1().min_w_0().v_flex().gap_0p5().px_3().py_2();
  if (!first) box = box.border_l_1().border_color(colors.border);
  return box.child(div().text_xs().text_color(colors.muted_foreground).child(label))
    .child(div().text_xs().truncate().font_family(font).child(value));
}
function plan(view, item, now, font) {
  const {text} = view, colors = theme().colors, {kind, data} = item.timing;
  const next = item.enabled ? upcoming(item, now, 1)[0] : undefined;
  const nextValue = !item.enabled ? text.paused : next === undefined ? "—" : when(next, now, text);
  const hero = kind === "every" ? interval(data.interval_ms, text) : kind === "once" ? clock(data.at_ms) : wallClock(data.hour, data.minute);
  const side = kind === "every" ? text.fixedInterval : kind === "once" ? text.once : text.weeklyDays(data.weekdays.length);
  const card = div().id("task-plan").v_flex().w_full().rounded_lg().border_1().border_color(colors.border).bg(colors.muted).overflow_hidden()
    .child(div().h_flex().items_end().justify_between().gap_2().px_3().pt_3().pb_2()
      .child(div().id("task-plan-hero").text_2xl().font_bold().font_family(font).child(hero))
      .child(div().text_xs().text_color(colors.muted_foreground).whitespace_nowrap().child(side)));
  if (kind === "weekly") {
    const today = item.next_ms === null ? localWeekday(now) : zoneWeekday(now, weeklyOffset(data, item.next_ms));
    card.child(div().id("task-plan-days").h_flex().gap_1().px_3().pb_3().children([1, 2, 3, 4, 5, 6, 7].map(day => {
      const active = data.weekdays.includes(day);
      let box = div().id(`task-plan-day-${day}`).h(34).w_full().rounded_md()
        .flex().items_center().justify_center().bg(active ? tint(colors.primary, "29") : colors.background);
      if (day === today) box = box.border_1().border_color(colors.ring);
      return div().flex_1().min_w_0().v_flex().items_center().gap_1()
        .child(div().text_xs().text_color(day === today ? colors.foreground : colors.muted_foreground).child(text[`weekday${day}`]))
        .child(box.child(active ? div().size(8).rounded_full().bg(colors.primary) : div().w(8).h(2).rounded_full().bg(colors.border)));
    })));
  } else if (kind === "every") {
    const strip = ribbon(data, now);
    const band = div().id("task-plan-ribbon").relative().w_full().h(30).rounded_md().bg(colors.background).overflow_hidden()
      .child(div().absolute().left(0).top(0).bottom(0).w(`${(strip.now * 100).toFixed(2)}%`).bg(tint(colors.primary, "12")))
      .children((strip.ticks ?? []).map((at, index) => div().id(`task-plan-tick-${index}`).absolute().top(7).bottom(7).w(3).rounded_sm()
        .left(`${(at * 100).toFixed(2)}%`).bg(at <= strip.now ? tint(colors.primary, "99") : colors.primary)))
      .child(div().id("task-plan-now").absolute().top(2).bottom(2).w(2).rounded_sm().left(`${(strip.now * 100).toFixed(2)}%`).bg(colors.foreground));
    card.child(div().v_flex().gap_1p5().px_3().pb_3().child(band)
      .child(div().h_flex().justify_between().text_xs().font_family(font).text_color(colors.muted_foreground)
        .children(["00", "06", "12", "18", "24"].map(hour => div().child(hour)))));
  } else {
    const date = new Date(data.at_ms);
    card.child(div().id("task-plan-date").h_flex().items_center().gap_3().px_3().pb_3()
      .child(div().w(46).flex_none().rounded_lg().border_1().border_color(colors.border).overflow_hidden().v_flex().items_center()
        .child(div().w_full().text_center().text_xs().font_semibold().bg(colors.primary).text_color(colors.primary_foreground)
          .child(text.month(date.getMonth() + 1)))
        .child(div().py_1().text_lg().font_bold().font_family(font).child(String(date.getDate()))))
      .child(div().text_xs().text_color(colors.muted_foreground).child(`${text[`weekday${localWeekday(data.at_ms)}`]} ${clock(data.at_ms)}`)));
  }
  const cells = kind === "weekly" ? [["timezone", text.timezone, data.timezone], ["next", text.next, nextValue]]
    : kind === "every" ? [["start", text.start, when(data.anchor_ms, now, text)], ["next", text.next, nextValue]]
    : [["next", text.next, nextValue]];
  return card.child(div().h_flex().border_t_1().border_color(colors.border)
    .children(cells.map(([key, label, value], index) => cell(`task-plan-${key}`, label, value, font, index === 0))));
}
function fact(id, label, value, note, dot, icon) {
  const colors = theme().colors;
  return div().id(id).h_flex().items_center().gap_3().min_h(30).text_sm()
    .child(div().w(72).flex_none().text_color(colors.muted_foreground).child(label))
    .child(div().flex_1().min_w_0().h_flex().items_center().gap_1p5()
      .children(dot ? [div().size(8).flex_none().rounded_full().bg(dot)] : [])
      .children(icon ? [new Icon(icon).size("small")] : [])
      .child(div().min_w_0().truncate().child(value))
      .children(note ? [div().flex_none().text_xs().text_color(colors.muted_foreground).child(note)] : []));
}
export function effortLabel(effort) {
  if (effort === undefined || effort === null || effort === "default") return "";
  return effortKey(effort);
}
function facts(view, item, runs, now) {
  const {text} = view, colors = theme().colors, next = upcoming(item, now, 1)[0], last = runs[0];
  const rows = [
    fact("task-fact-next", text.nextRun, !item.enabled ? text.paused : next === undefined ? "—" : when(next, now, text),
      item.enabled && next !== undefined ? relative(next, now, text) : "", null),
    fact("task-fact-last", text.lastRun, last ? when(last.job.started_ms ?? last.job.created_ms, now, text) : text.neverRun,
      last ? runLabel(text, last.job.status) : "", last ? runColor(last.job.status, colors) : null),
    fact("task-fact-project", text.project, view.projectName(item.project), "", null, "icons/folder.svg"),
  ];
  if (item.project) {
    const tree = item.worktree ? view.catalog.worktrees.find(tree => tree.id === item.worktree)?.path ?? text.unavailable : text.mainWorktree;
    rows.push(fact("task-fact-worktree", text.worktree, tree, "", null));
  }
  if (item.config) {
    const effort = effortLabel(item.config.effort);
    rows.push(fact("task-fact-model", text.model, `${item.config.model}${effort ? ` ${effort}` : ""}`, "", null));
  }
  rows.push(fact("task-fact-execution", text.execution, (item.execution ?? "team") === "model" ? text.model
    : item.team ? view.teams.find(team => team.id === item.team.id)?.name ?? text.unavailable : text.defaultTeam, "", null));
  rows.push(fact("task-fact-status", text.status, item.enabled ? text.enabled : text.paused, "",
    item.enabled ? colors.success : colors.muted_foreground));
  return div().id("task-facts").v_flex().gap_0p5().pt_3().border_t_1().border_color(colors.border).children(rows);
}
function section(id, title, trailing) {
  const colors = theme().colors;
  return div().id(id).v_flex().gap_2().pt_3().border_t_1().border_color(colors.border)
    .child(div().h_flex().items_center().justify_between().gap_2()
      .child(div().text_xs().font_semibold().text_color(colors.muted_foreground).child(title))
      .children(trailing ? [trailing] : []));
}
export function recentText(text, runs) {
  const counts = tally(runs);
  const parts = [["completed", text.countCompleted], ["failed", text.countFailed], ["cancelled", text.countCancelled],
    ["running", text.countRunning]].filter(([key]) => counts[key]).map(([key, format]) => format(counts[key]));
  return parts.length ? parts.join(" · ") : text.noRuns;
}
function overview(view, item, runs, now, font) {
  const {text} = view, colors = theme().colors, failures = streak(runs), body = div().id("task-overview").v_flex().gap_4().px_4().pt_3().pb_6();
  if (failures) {
    const failed = runs[0], at = failed.job.started_ms ?? failed.job.created_ms;
    body.child(div().id("task-failure").h_flex().items_start().gap_2p5().p_3().rounded_lg().border_1()
      .border_color(tint(colors.destructive, "59")).bg(tint(colors.destructive, "14"))
      .child(div().h_5().flex_none().h_flex().items_center()
        .child(new Icon("icons/circle-alert.svg").size("small").text_color(colors.destructive)))
      .child(div().flex_1().min_w_0().v_flex().gap_1()
        .child(div().text_sm().line_height("1.25rem").font_semibold().child(text.lastFailed))
        .children(failed.job.error?.message ? [div().text_xs().whitespace_normal().child(failed.job.error.message)] : [])
        .child(div().text_xs().text_color(colors.muted_foreground)
          .child(failures > 1 ? `${when(at, now, text)} · ${text.failedStreak(failures)}` : when(at, now, text)))));
  }
  body.child(plan(view, item, now, font)).child(facts(view, item, runs, now));
  body.child(section("task-prompt", text.prompt, IconButton.new("task-prompt-copy", {icon:"copy", label:text.copy, size:"small"}))
    .child(div().id("task-prompt-box").rounded_lg().bg(colors.muted).px_3().py_2p5().text_sm()
      .child(ClampedText.new("task-prompt-text", {text:item.prompt, lines:5}))));
  const recent = runs.slice(0, RECENT).reverse();
  body.child(section("task-recent", text.recent(RECENT), div().text_xs().text_color(colors.muted_foreground).child(recentText(text, recent)))
    .children(recent.length ? [div().id("task-recent-strip").h_flex().gap_1().children(recent.map(run => {
      const block = div().id(`task-recent-${run.job.id}`).flex_1().min_w_0().h(22).rounded_sm().bg(runColor(run.job.status, colors));
      return run.job.status === "cancelled" ? block.opacity(0.6) : block;
    }))] : []));
  const times = upcoming(item, now);
  body.child(section("task-upcoming", text.upcoming)
    .children(!item.enabled ? [div().id("task-upcoming-paused").text_xs().text_color(colors.muted_foreground).child(text.resumeNote)]
      : times.length ? times.map((at, index) => div().id(`task-upcoming-${index}`).h_flex().items_center().justify_between().gap_3().h(26)
        .child(div().text_xs().font_family(font).child(when(at, now, text)))
        .child(div().text_xs().text_color(colors.muted_foreground).child(relative(at, now, text))))
      : [div().id("task-upcoming-empty").text_xs().text_color(colors.muted_foreground).child(text.noUpcoming)]));
  return body;
}
function entry(view, run, last, font) {
  const {text} = view, colors = theme().colors, {job} = run, failed = job.status === "failed";
  const disabled = view.busy || !!view.pending, stoppable = job.status === "queued" || (job.status === "running" && run.turn);
  const card = div().id(`task-run-card-${job.id}`).flex_1().min_w_0().v_flex().gap_1p5().px_2p5().py_2().rounded_lg().border_1()
    .border_color(failed ? tint(colors.destructive, "38") : colors.border).bg(failed ? tint(colors.destructive, "12") : colors.muted)
    .child(div().h_flex().items_center().gap_2()
      .child(div().text_sm().font_medium().text_color(failed ? colors.destructive : colors.foreground).child(runLabel(text, job.status)))
      .child(div().text_xs().font_family(font).text_color(colors.muted_foreground).child(clock(job.started_ms ?? job.created_ms, true)))
      .child(div().flex_1())
      .children(stoppable ? [IconButton.new(`task-stop-${job.id}`, {icon:"square", label:text.stop, size:"small", disabled})] : [])
      .children(run.session ? [IconButton.new(`task-open-${job.id}`, {icon:"external-link", label:text.open, size:"small"})] : []))
    .children(job.error?.message ? [div().id(`task-run-error-${job.id}`).text_xs().whitespace_normal()
      .text_color(failed ? colors.destructive : colors.muted_foreground).child(job.error.message)] : []);
  return rail(`task-run-${job.id}`, div().mt(11).size(12).flex_none().rounded_full().bg(runColor(job.status, colors)), last, card, 3);
}
function rail(id, node, last, body, gap) {
  const colors = theme().colors;
  return div().id(id).h_flex().items_stretch().gap_3().px_2()
    .child(div().w(14).flex_none().v_flex().items_center()
      .child(node).children(last ? [] : [div().flex_1().w(2).min_h(10).mt_1().rounded_sm().bg(colors.border)]))
    .child(div().flex_1().min_w_0().pb(gap * 4).child(body));
}
export function timeline(view, runs, now, font) {
  const {text} = view, colors = theme().colors, rows = [];
  let day = null;
  runs.forEach((run, index) => {
    const at = run.job.started_ms ?? run.job.created_ms, label = dayLabel(at, now, text);
    if (label !== day) {
      day = label;
      rows.push(rail(`task-day-${index}`, div().mt_1().size(8).flex_none().rounded_full().border_2().border_color(colors.border).bg(colors.background),
        false, div().text_xs().font_semibold().text_color(colors.muted_foreground).child(label), 2.5));
    }
    rows.push(entry(view, run, index === runs.length - 1 && !view.more, font));
  });
  return rows;
}
function records(view, runs, loaded, now, font) {
  const {text} = view;
  if (!loaded) return div().id("task-records").relative().w_full().h_24().child(Loading.new("task-records-loading"));
  if (!runs.length) return div().id("task-records").v_flex().flex_1().p_3()
    .child(EmptyState.new("task-records-empty", {variant:"list", icon:"calendar", label:text.noRuns}));
  return div().id("task-records").v_flex().px_2().pt_2().pb_5().children(timeline(view, runs, now, font))
    .children(view.more ? [LoadMore.new("task-records-more", {loading:view.loading, label:text.loading})] : []);
}
export function details(view, cx) {
  const {text, panel} = view, item = panel && view.items.find(item => item.id === panel.task);
  if (!item) return null;
  const colors = theme().colors, font = cx.theme().typography.mono, now = Date.now();
  const loaded = view.historyTask === item.id && view.historyLoaded;
  const runs = loaded ? view.history.filter(run => run.job.handler === item.id) : [];
  const header = PanelHeader.new("task-panel-header", {draggable:false, bordered:true})
    .child(div().h_flex().w_full().min_w_0().items_center().justify_between().gap_2()
      .child(SegmentedTabs.new("task-panel-tabs", {selected:panel.tab, items:[{id:"details", label:text.details},
        {id:"records", label:loaded ? `${text.records} ${runs.length}${view.more ? "+" : ""}` : text.records}]}))
      .child(IconButton.new("task-panel-close", {icon:"close", label:text.close})));
  const top = div().id("task-panel-summary").v_flex().gap_2p5().px_4().pt_4().pb_3().border_b_1().border_color(colors.border)
    .child(div().h_flex().items_center().gap_2().min_w_0()
      .child(div().min_w_0().truncate().text_base().font_semibold().child(item.name))
      .child(statusTag(view, item, "task-panel-status")))
    .child(div().h_flex().items_center().gap_3().min_w_0().text_xs().text_color(colors.muted_foreground)
      .child(div().flex_none().font_family(font).child(summary(item, now, text)))
      .child(div().id("task-panel-project").h_flex().items_center().gap_1p5().min_w_0()
        .child(new Icon("icons/folder.svg").size("small"))
        .child(div().min_w_0().truncate().child(view.projectName(item.project)))));
  return div().id("task-panel").v_flex().size_full().min_w_0().min_h_0()
    .child(header)
    .child(div().id("task-panel-body").flex_1().min_h_0().v_flex().overflow_y_scrollbar()
      .child(top).child(panel.tab === "records" ? records(view, runs, loaded, now, font) : overview(view, item, runs, now, font)));
}

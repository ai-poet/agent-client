import {div} from "gpui-kit";
import {Icon, Tag} from "gpui-component";
import {theme} from "sailry";
import {EmptyState, IconButton, Loading, PanelHeader, SegmentedTabs} from "sailry/ui";
import {clock, date, dayLabel, relative, when} from "./time.js";
export const DETAIL_WIDTH = 380;

// Theme colors are #rrggbbaa; tints keep the semantic hue with a lighter alpha.
const tint = (color, alpha) => `${color.slice(0, 7)}${alpha}`;
export function overdue(item, now) { return !item.completed && item.due_ms !== null && item.due_ms <= now; }
// Completion wins; an open reminder past its due time is overdue even after notification.
export function reminderStatus(text, item, now) {
  if (item.completed) return {label:text.completed, tone:"success"};
  if (overdue(item, now)) return {label:text.overdue, tone:"danger"};
  return {label:text.pending, tone:"warning"};
}
export function statusTag(status, id) {
  return div().id(id).flex_none().child(new Tag().variant(status.tone).size("medium").rounded_full().child(status.label));
}
export function deliveryColor(status, colors) {
  return {completed:colors.success, failed:colors.destructive, running:colors.info, queued:colors.info,
    unknown:colors.warning}[status] ?? colors.muted_foreground;
}
export function deliveryLabel(text, status) {
  return {completed:text.notified, failed:text.deliveryFailed, running:text.waiting, queued:text.waiting,
    unknown:text.deliveryUnknown, cancelled:text.deliveryCancelled}[status] ?? text.deliveryUnknown;
}
function due(view, item, now, font) {
  const {text} = view, colors = theme().colors, at = item.due_ms, late = overdue(item, now);
  const band = at === null ? colors.muted_foreground : item.completed ? colors.success : late ? colors.destructive : colors.primary;
  const value = at === null ? null : new Date(at);
  const tile = div().id("reminder-due-tile").w(46).flex_none().rounded_lg().border_1().border_color(colors.border).overflow_hidden()
    .bg(colors.background).v_flex().items_center()
    .child(div().id("reminder-due-month").w_full().text_center().text_xs().font_semibold().bg(band)
      .text_color(at === null ? colors.background : colors.primary_foreground).child(value ? text.month(value.getMonth() + 1) : "—"))
    .child(div().py_1().text_lg().font_bold().font_family(font).child(value ? String(value.getDate()) : "—"));
  const lines = at === null
    ? [div().id("reminder-due-time").text_sm().font_medium().child(text.noDue),
      div().id("reminder-due-relative").text_xs().text_color(colors.muted_foreground).child(text.noNotice)]
    : [div().id("reminder-due-time").text_sm().font_medium().font_family(font).child(`${date(at, text)} ${clock(at)}`),
      div().id("reminder-due-relative").text_xs().text_color(late ? colors.destructive : colors.muted_foreground)
        .child(relative(at, now, text))];
  return div().id("reminder-due").h_flex().items_center().gap_3().p_3().rounded_lg().border_1().border_color(colors.border)
    .bg(colors.muted).child(tile).child(div().flex_1().min_w_0().v_flex().gap_1().children(lines));
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
function facts(view, item, now) {
  const {text} = view, colors = theme().colors;
  const notice = item.notified_ms !== null ? [text.notified, when(item.notified_ms, now, text), colors.success]
    : item.due_ms !== null && !item.completed ? [text.waiting, "", colors.info] : [text.none, "", null];
  return div().id("reminder-facts").v_flex().gap_0p5().pt_3().border_t_1().border_color(colors.border)
    .child(fact("reminder-fact-status", text.status, item.completed ? text.completed : text.pending, "",
      item.completed ? colors.success : colors.warning))
    .child(fact("reminder-fact-notice", text.reminder, notice[0], notice[1], notice[2]))
    .child(fact("reminder-fact-project", text.project, view.projectName(item.project), "", null, "icons/folder.svg"));
}
function overview(view, item, now, font) {
  const {text} = view, colors = theme().colors;
  const note = item.message ? div().id("reminder-note").rounded_lg().bg(colors.muted).px_3().py_2p5().text_sm().whitespace_normal()
    .child(item.message) : div().id("reminder-note").text_sm().text_color(colors.muted_foreground).child(text.noNotes);
  return div().id("reminder-overview").v_flex().gap_4().px_4().pt_3().pb_6()
    .child(due(view, item, now, font)).child(facts(view, item, now))
    .child(div().pt_3().border_t_1().border_color(colors.border).child(note));
}
function rail(id, node, last, body, gap) {
  const colors = theme().colors;
  return div().id(id).h_flex().items_stretch().gap_3().px_2()
    .child(div().w(14).flex_none().v_flex().items_center()
      .child(node).children(last ? [] : [div().flex_1().w(2).min_h(10).mt_1().rounded_sm().bg(colors.border)]))
    .child(div().flex_1().min_w_0().pb(gap * 4).child(body));
}
export function deliveryTime(job) { return job.finished_ms ?? job.started_ms ?? job.created_ms; }
function entry(view, job, last, font) {
  const {text} = view, colors = theme().colors, failed = job.status === "failed";
  const card = div().id(`reminder-record-card-${job.id}`).flex_1().min_w_0().v_flex().gap_1p5().px_2p5().py_2().rounded_lg().border_1()
    .border_color(failed ? tint(colors.destructive, "38") : colors.border).bg(failed ? tint(colors.destructive, "12") : colors.muted)
    .child(div().h_flex().items_center().gap_2()
      .child(div().text_sm().font_medium().text_color(failed ? colors.destructive : colors.foreground).child(deliveryLabel(text, job.status)))
      .child(div().text_xs().font_family(font).text_color(colors.muted_foreground).child(clock(deliveryTime(job), true))))
    .children(job.error?.message ? [div().id(`reminder-record-error-${job.id}`).text_xs().whitespace_normal()
      .text_color(failed ? colors.destructive : colors.muted_foreground).child(job.error.message)] : []);
  return rail(`reminder-record-${job.id}`, div().mt(11).size(12).flex_none().rounded_full().bg(deliveryColor(job.status, colors)), last, card, 3);
}
export function timeline(view, jobs, now, font) {
  const {text} = view, colors = theme().colors, rows = [];
  let day = null;
  jobs.forEach((job, index) => {
    const label = dayLabel(deliveryTime(job), now, text);
    if (label !== day) {
      day = label;
      rows.push(rail(`reminder-day-${index}`, div().mt_1().size(8).flex_none().rounded_full().border_2().border_color(colors.border).bg(colors.background),
        false, div().text_xs().font_semibold().text_color(colors.muted_foreground).child(label), 2.5));
    }
    rows.push(entry(view, job, index === jobs.length - 1, font));
  });
  return rows;
}
function records(view, jobs, loaded, now, font) {
  const {text} = view;
  if (!loaded) return div().id("reminder-records").relative().w_full().h_24().child(Loading.new("reminder-records-loading"));
  if (!jobs.length) return div().id("reminder-records").v_flex().flex_1().p_3()
    .child(EmptyState.new("reminder-records-empty", {variant:"list", icon:"bell", label:text.noRecords}));
  return div().id("reminder-records").v_flex().px_2().pt_2().pb_5().children(timeline(view, jobs, now, font));
}
export function details(view, cx) {
  const {text, panel} = view, item = panel && view.items.find(item => item.id === panel.reminder);
  if (!item) return null;
  const colors = theme().colors, font = cx.theme().typography.mono, now = Date.now();
  const loaded = view.historyId === item.id && view.historyLoaded;
  const jobs = loaded ? view.history.filter(job => job.handler === item.id) : [];
  const header = PanelHeader.new("reminder-panel-header", {draggable:false, bordered:true})
    .child(div().h_flex().w_full().min_w_0().items_center().justify_between().gap_2()
      .child(SegmentedTabs.new("reminder-panel-tabs", {selected:panel.tab, items:[{id:"details", label:text.details},
        {id:"records", label:loaded ? `${text.records} ${jobs.length}` : text.records}]}))
      .child(IconButton.new("reminder-panel-close", {icon:"close", label:text.close})));
  const top = div().id("reminder-panel-summary").v_flex().gap_2p5().px_4().pt_4().pb_3().border_b_1().border_color(colors.border)
    .child(div().h_flex().items_center().gap_2().min_w_0()
      .child(div().min_w_0().truncate().text_base().font_semibold().child(item.title))
      .child(statusTag(reminderStatus(text, item, now), "reminder-panel-status")))
    .child(div().id("reminder-panel-project").h_flex().items_center().gap_1p5().min_w_0().text_xs().text_color(colors.muted_foreground)
      .child(new Icon("icons/folder.svg").size("small"))
      .child(div().min_w_0().truncate().child(view.projectName(item.project))));
  return div().id("reminder-panel").v_flex().size_full().min_w_0().min_h_0()
    .child(header)
    .child(div().id("reminder-panel-body").flex_1().min_h_0().v_flex().overflow_y_scrollbar()
      .child(top).child(panel.tab === "records" ? records(view, jobs, loaded, now, font) : overview(view, item, now, font)));
}

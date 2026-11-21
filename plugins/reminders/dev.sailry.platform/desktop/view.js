import { div } from "gpui-kit";
import { Button, Icon } from "gpui-component";
import { theme } from "sailry";
import { CardColumn, CardList, CardRow, CardSummary, IconButton, Modal, SelectField, SettingsPage, Toggle, Workspace } from "sailry/ui";
import { render as editor } from "./editor.js";
import { details, overdue, reminderStatus, statusTag, DETAIL_WIDTH } from "./details.js";
import { when } from "./time.js";

// Earlier due times first, so overdue reminders lead upcoming ones; untimed reminders follow.
export function order(items) {
  return [...items].sort((a, b) => (a.due_ms ?? Number.MAX_SAFE_INTEGER) - (b.due_ms ?? Number.MAX_SAFE_INTEGER)
    || a.title.localeCompare(b.title));
}
function card(view, item, now) {
  const {text, panel} = view, colors = theme().colors, disabled = view.busy || !!view.pending || !!item.unavailable;
  const open = panel?.reminder === item.id ? panel.tab : null, late = overdue(item, now);
  // CardRow has no selected state; a ring border, in place of its own, marks the reminder shown in the details panel.
  const row = CardRow.new(`reminder-card-${item.id}`, {row_id:`reminder-row-${item.id}`})
    .rounded_lg().border_1().border_color(open ? colors.ring : colors.border);
  if (item.completed) row.line_through();
  return row
    .child(CardSummary.new(`reminder-summary-${item.id}`, {title:item.title,subtitle:view.projectName(item.project),icon:"icons/folder.svg"})
      .child(Toggle.new(`reminder-complete-${item.id}`, {variant:"checkbox", label:`${text.complete}: ${item.title}`,
        checked:item.completed, disabled})))
    .child(CardColumn.new(`reminder-time-${item.id}`, {variant:"wide_metadata"})
      .child(div().h_flex().w_full().min_w_0().items_center().justify_end().gap_2()
        .child(div().id(`reminder-when-${item.id}`).min_w_0().truncate().text_xs()
          .text_color(late ? colors.destructive : colors.muted_foreground)
          .child(item.unavailable ? text.executionUnavailable : item.due_ms === null ? text.noTime : when(item.due_ms, now, text)))
        .children(late ? [statusTag(reminderStatus(text, item, now), `reminder-state-${item.id}`)] : [])))
    .child(CardColumn.new(`reminder-actions-${item.id}`, {variant:"actions"})
      .child(IconButton.new(`reminder-details-${item.id}`, {icon:"info",label:text.details,selected:open === "details"}))
      .child(IconButton.new(`reminder-records-${item.id}`, {icon:"reicon:time/clock",label:text.records,selected:open === "records"}))
      .child(IconButton.new(`reminder-edit-${item.id}`, {icon:"reicon:newicons/edit",label:text.edit,disabled}))
      .child(IconButton.new(`reminder-delete-${item.id}`, {icon:"reicon:ui/trash2",label:text.remove,disabled})));
}
function confirmation(view) {
  const {text} = view;
  return [div().id("reminder-delete-confirmation").v_flex().gap_3()
    .child(div().truncate().child(view.deleting.title))
    .child(div().text_sm().text_color(theme().colors.muted_foreground).child(text.deleteMessage)),
  div().h_flex().justify_end().gap_2()
    .child(new Button("reminder-delete-cancel").label(text.cancel).disabled(view.busy)
      .on_click((_, cx) => view.close(cx)))
    .child(new Button("reminder-delete-confirm").danger().label(view.pending ? text.retry : text.remove).disabled(view.busy)
      .on_click((_, cx) => view.perform("remove", {id:view.deleting.id,revision:view.deleting.revision}, cx)))];
}
function projectFilter(view) {
  const {text} = view;
  const projects = new Map(view.catalog.projects.map(project => [project.id, project.name]));
  for (const item of view.items) if (item.project !== null && !projects.has(item.project)) {
    projects.set(item.project, text.unavailableProject);
  }
  return SelectField.new("reminders-project-filter", {label:text.project, placeholder:text.allProjects,
    selected:view.projectFilter ?? "none", items:[{id:"all",label:text.allProjects}, {id:"none",label:text.noProject},
      ...Array.from(projects, ([id,label]) => ({id,label}))]});
}
function completedGroup(view, items, now) {
  const colors = theme().colors, expanded = view.showCompleted;
  return div().id("reminders-completed").v_flex().gap_2().w_full()
    .child(div().h_flex().child(new Button("reminders-completed-toggle").ghost().size("small")
      .child(div().h_flex().items_center().gap_1p5()
        .child(new Icon(expanded ? "icons/chevron-down.svg" : "icons/chevron-right.svg").size("small"))
        .child(div().child(view.text.completed))
        .child(div().text_color(colors.muted_foreground).child(String(items.length))))
      .on_click((_, cx) => { view.showCompleted = !view.showCompleted; cx.notify(); })))
    .children(expanded ? [CardList.new("reminders-done", {loaded:true})
      .children(items.map(item => card(view, item, now)))] : []);
}
export function render(view, cx) {
  const {text} = view, colors = theme().colors, disabled = view.busy || !!view.pending, now = Date.now();
  const items = order(view.items.filter(item => view.projectFilter === "all" || item.project === view.projectFilter));
  const open = items.filter(item => !item.completed), done = items.filter(item => item.completed);
  const body = SettingsPage.new("reminders", {title:text.pageTitle, description:text.description})
    .child(div().id("reminders-toolbar").h_flex().items_center().justify_between().gap_3().flex_wrap()
      .child(div().w_48().max_w_full().child(projectFilter(view)))
      .child(new Button("reminder-create").primary().label(text.create).disabled(disabled)
        .on_click((_, cx) => view.edit(null, cx))))
    .children(!view.editing && !view.deleting && !view.busy && view.pending ? [new Button("reminder-retry").label(text.retry)
      .disabled(view.busy).on_click((_, cx) => view.perform(null, null, cx)),
      new Button("reminder-dismiss").label(text.cancel).disabled(view.busy).on_click((_, cx) => view.close(cx))] : [])
    .children(open.length || !done.length ? [CardList.new("reminders-items", {loaded:view.loaded,empty_icon:"bell",empty_label:text.empty})
      .children(open.map(item => card(view, item, now)))] : [])
    .children(done.length ? [completedGroup(view, done, now)] : []);
  const panel = details(view, cx);
  return div().id("reminders-page").size_full().v_flex().min_h_0().min_w_0()
    .text_color(colors.foreground)
    .child(Workspace.new("reminders-workspace", {has_details:!!panel, details_label:text.details,
      default_details_width:DETAIL_WIDTH, min_details_width:320, navigation:"none"})
      .children([body, ...(panel ? [panel] : [])]))
    .child(Modal.new(`reminder-dialog-${view.dialogId}`, {open:!!(view.editing || view.deleting), dismissable:!view.busy,
      form:!!(view.editing || view.deleting), title:view.editing ? view.editing.draft.id ? text.edit : text.create
        : view.deleting ? text.deleteTitle : "", width:view.deleting ? 528 : 480})
      .children(view.editing ? editor(view) : view.deleting ? confirmation(view) : []));
}

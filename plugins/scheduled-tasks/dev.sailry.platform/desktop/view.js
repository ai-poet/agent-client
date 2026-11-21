import {div} from "gpui-kit";
import {Button} from "gpui-component";
import {theme} from "sailry";
import {CardColumn, CardList, CardRow, CardSummary, IconButton, Modal, SelectField, SettingsPage, Toggle, Workspace} from "sailry/ui";
import {render as editor} from "./editor.js";
import {details, statusTag, DETAIL_WIDTH} from "./details.js";
import {MINUTE, when} from "./schedule.js";

// Running time comes from the native job start; scheduled tasks show their next run.
function timeText(view, item, now) {
  if (item.status === "running" && Number.isFinite(item.started_ms)) {
    return view.text.runningFor(Math.max(0, Math.floor((now - item.started_ms) / MINUTE)));
  }
  return item.enabled && Number.isFinite(item.next_ms) ? when(item.next_ms, now, view.text) : "";
}
function card(view, item, now) {
  const {text, panel} = view, colors = theme().colors, disabled = view.busy || !!view.pending || !!item.unavailable;
  const open = panel?.task === item.id ? panel.tab : null, time = timeText(view, item, now);
  // CardRow has no selected state; a ring border, in place of its own, marks the task shown in the details panel.
  return CardRow.new(`task-card-${item.id}`,{row_id:`task-row-${item.id}`})
    .rounded_lg().border_1().border_color(open ? colors.ring : colors.border)
    .child(CardSummary.new(`task-summary-${item.id}`,{title:item.name,subtitle:view.projectName(item.project),icon:"icons/folder.svg"}))
    .child(CardColumn.new(`task-status-${item.id}`,{variant:"wide_metadata"})
      .child(div().h_flex().w_full().min_w_0().items_center().justify_end().gap_2()
        .children(time ? [div().id(`task-time-${item.id}`).min_w_0().truncate().text_xs().text_color(colors.muted_foreground).child(time)] : [])
        .child(statusTag(view, item, `task-state-${item.id}`))))
    .child(CardColumn.new(`task-leading-${item.id}`,{variant:"control"})
      .child(Toggle.new(`task-enabled-${item.id}`, {label:`${text.enabled}: ${item.name}`,checked:item.enabled,disabled})))
    .child(CardColumn.new(`task-actions-${item.id}`,{variant:"actions"})
      .child(IconButton.new(`task-run-${item.id}`,{icon:"reicon:video/play",label:text.run,disabled}))
      .child(IconButton.new(`task-details-${item.id}`,{icon:"info",label:text.details,selected:open === "details"}))
      .child(IconButton.new(`task-records-${item.id}`,{icon:"reicon:time/clock",label:text.records,selected:open === "records"}))
      .child(IconButton.new(`task-edit-${item.id}`,{icon:"reicon:newicons/edit",label:text.edit,disabled}))
      .child(IconButton.new(`task-delete-${item.id}`,{icon:"reicon:ui/trash2",label:text.remove,disabled})));
}
function confirmation(view) {
  const {text} = view;
  return [div().id("task-delete-confirmation").v_flex().gap_3()
    .child(div().truncate().child(view.deleting.name))
    .child(div().text_sm().text_color(theme().colors.muted_foreground).child(text.deleteMessage)),
  div().h_flex().justify_end().gap_2()
    .child(new Button("task-delete-cancel").label(text.cancel).disabled(view.busy).on_click((_,cx) => view.close(cx)))
    .child(new Button("task-delete-confirm").danger().label(view.pending ? text.retry : text.remove).disabled(view.busy)
      .on_click((_,cx) => view.perform("remove",{id:view.deleting.id,revision:view.deleting.revision},cx)))];
}
function projectFilter(view) {
  const {text} = view;
  const projects = new Map(view.catalog.projects.map(project => [project.id, project.name]));
  for (const item of view.items) if (item.project !== null && !projects.has(item.project)) projects.set(item.project, text.unavailable);
  return SelectField.new("tasks-project-filter", {label:text.project, placeholder:text.allProjects,
    selected:view.projectFilter ?? "none", items:[{id:"all",label:text.allProjects}, {id:"none",label:text.noProject},
      ...Array.from(projects, ([id,label]) => ({id,label}))]});
}
export function render(view, cx) {
  const {text} = view, colors = theme().colors, disabled = view.busy || !!view.pending, now = Date.now();
  const items = view.items.filter(item => view.projectFilter === "all" || item.project === view.projectFilter);
  const body = SettingsPage.new("tasks", {title:text.title, description:text.description})
    .child(div().id("tasks-toolbar").w_full()
      .child(div().id("tasks-controls").h_flex().items_center().justify_between().gap_3().flex_wrap()
        .child(div().w_48().max_w_full().child(projectFilter(view)))
        .child(new Button("task-create").primary().label(text.create).disabled(disabled).on_click((_,cx) => view.edit(null,cx)))))
    .children(!view.editing && !view.deleting && !view.busy && view.pending ? [new Button("task-retry").label(text.retry).disabled(view.busy)
      .on_click((_,cx) => view.perform(null,null,cx)),new Button("task-dismiss").label(text.cancel).disabled(view.busy).on_click((_,cx) => view.close(cx))] : [])
    .child(CardList.new("tasks-items",{loaded:view.loaded,empty_icon:"calendar",empty_label:text.empty})
      .children(items.map(item => card(view,item,now))));
  const panel = details(view, cx);
  return div().id("scheduled-tasks-page").size_full().v_flex().min_h_0().min_w_0()
    .text_color(colors.foreground)
    .child(Workspace.new("tasks-workspace",{has_details:!!panel,details_label:text.details,
      default_details_width:DETAIL_WIDTH,min_details_width:320,navigation:"none"})
      .children([body,...(panel ? [panel] : [])]))
    .child(Modal.new(`task-dialog-${view.dialogId}`,{open:!!(view.editing || view.deleting),dismissable:!view.busy,
      form:!!(view.editing || view.deleting),title:view.editing ? view.editing.draft.id ? text.edit : text.create
        : view.deleting ? text.deleteTitle : "",width:view.deleting ? 528 : 560})
      .children(view.editing ? editor(view) : view.deleting ? confirmation(view) : []));
}

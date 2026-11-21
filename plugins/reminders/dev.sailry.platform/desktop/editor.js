import { div } from "gpui-kit";
import { Button, Checkbox, Field, VForm } from "gpui-component";
import { SelectField } from "sailry/ui";
import {sameNode, snapshot} from "../host/records.js";
import { TextField, createText, readText, releaseText,
  DateTimeField, createDateTime, readDateTime, releaseDateTime } from "sailry/forms";

export function editor(item, text) {
  const draft = item ? {...item} : {revision:"0", project:null, title:"", message:"", due_ms:null, completed:false};
  return {draft, original:item ? snapshot(item) : null, timed:draft.due_ms !== null,
    title:createText(draft.title, {label:text.title, placeholder:text.titlePlaceholder}),
    message:createText(draft.message, {label:text.message, placeholder:text.messagePlaceholder, multiline:true}),
    time:createDateTime(draft.due_ms ?? Date.now() + 300000, text.time)};
}
export function release(editor) {
  releaseText(editor.title); releaseText(editor.message); releaseDateTime(editor.time);
}
export function value(editor) {
  const title = readText(editor.title).trim();
  if (!title) throw new Error("required");
  let due = null;
  if (editor.timed) {
    try { due = readDateTime(editor.time); } catch (_) { throw new Error("invalidTime"); }
    if (due !== editor.draft.due_ms && due <= Date.now()) throw new Error("invalidTime");
  }
  return {...editor.draft, title, message:readText(editor.message), due_ms:due,
    ...(editor.original ? {expected_value:editor.original} : {})};
}
function projects(view, disabled) {
  const {text, editing} = view;
  const items = [{id:"none",label:text.noProject}, ...view.catalog.projects
    .filter(project => sameNode(project.node, editing.draft.execution_node)).map(({id,name}) => ({id,label:name}))];
  if (editing.draft.project !== null && !items.some(item => item.id === editing.draft.project)) {
    items.push({id:editing.draft.project,label:text.unavailableProject});
  }
  return SelectField.new("reminder-project", {label:text.project, placeholder:text.projectPlaceholder,
    selected:editing.draft.project ?? "none", items, disabled});
}
function hosts(view, disabled) {
  const {text, editing} = view;
  const items = view.hosts.map(source => ({id:JSON.stringify(source.node),
    label:source.fault ? `${source.name} (${text.executionUnavailable})` : source.name}));
  const selected = JSON.stringify(editing.draft.execution_node ?? null);
  if (!items.some(item => item.id === selected)) items.push({id:selected,label:text.executionUnavailable});
  return SelectField.new("reminder-host", {label:text.executionHost, selected, items,
    disabled:disabled || !!editing.draft.id});
}
export function render(view) {
  const {text, editing} = view, disabled = view.busy || !!view.pending;
  const fields = new VForm().size('large').gap_y(8).w_full().gap_3()
    .child(new Field().label(text.title).child(TextField.new(editing.title, {disabled})))
    .child(new Field().label(text.message).child(TextField.new(editing.message, {disabled})))
    .child(new Field().label(text.executionHost).child(hosts(view, disabled)))
    .child(new Field().label(text.project).child(projects(view, disabled)))
    .child(new Field().child(new Checkbox("reminder-timed").label(text.timed).checked(editing.timed).disabled(disabled)
      .on_change((checked, cx) => { editing.timed = checked; cx.notify(); })))
    .children(editing.timed ? [new Field().label(text.time).child(DateTimeField.new(editing.time, {disabled}))] : []);
  return [div().id("reminder-editor").w_full()
    .child(div().id("reminder-fields").w_full().child(fields)),
    div().h_flex().flex_shrink(0).justify_end().gap_2()
      .child(new Button("reminder-cancel").label(text.cancel).disabled(view.busy)
        .on_click((_, cx) => view.close(cx)))
      .child(new Button("reminder-save").primary().label(view.pending ? text.retry : text.save).disabled(view.busy)
        .on_click((_, cx) => view.save(cx)))];
}

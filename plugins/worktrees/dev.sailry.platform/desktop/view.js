// Approved forms composed with native Kit fields, buttons, checkbox and modal.
import {div} from 'gpui-kit';
import {Button,Checkbox,Field,Form} from 'gpui-component';
import {TextField} from 'sailry/forms';
import {Modal,Picker} from 'sailry/ui';

function button(id,label,action,disabled = false,primary = false) {
  let control = new Button(id).label(label).disabled(disabled).on_click(action);
  if (primary) control = control.primary();
  return div().id(id).child(control);
}
function footer(view,owner,submit,cancel) {
  return div().h_flex().w_full().mt_3().justify_end().gap_2()
    .children(owner.uncertain && owner.kind === 'create' ? [button('live-worktree-recover',view.text('workspace_project'),(_,cx) => view.overview(cx))] : [])
    .child(button(cancel,view.text('settings_cancel'),(_,cx) => view.close(cx)))
    .child(button(submit,view.text(owner.request && owner.kind !== 'create' ? 'worktree_retry' : owner.kind === 'remove' ? 'worktree_remove' : 'worktree_submit'),(_,cx) => view.submit(cx),owner.pending,true));
}
function create(view,owner) {
  const readonly = owner.pending || owner.uncertain;
  const fields = [['revision','worktree_base','live-worktree-base'],['branch','worktree_branch','live-worktree-branch'],['path','worktree_path','live-worktree-path']];
  const form = new Form().size('large').gap_y(8).children(fields.map(([name,label,id]) => {
    const input = div().id(id).child(TextField.new(owner.fields[name],{readonly}));
    return new Field().label(view.text(label)).required(true).child(input);
  }));
  const content = div().v_flex().max_h(Math.max(120,window.viewport_size().height-200)).overflow_y_scroll().child(form);
  return div().id('live-worktree-form').v_flex().gap_4().child(content);
}
function managed(view,owner) {
  const readonly = owner.pending || !!owner.request;
  const name = new Field().label(view.text('worktree_branch')).required(true)
    .child(div().id(owner.pending || !owner.status ? 'location-branch-loading' : 'location-branch-name')
      .child(TextField.new(owner.fields.branch,{readonly})));
  // A checkout started from another branch cannot carry this checkout's changes.
  const source = owner.base
    ? new Field().label(view.text('location_source')).child(div().text_sm().child(owner.base.from))
    : new Field().child(div().id('location-copy-changes').child(new Checkbox('location-copy-changes').size('small')
      .label(view.text('location_copy_changes')).checked(owner.include_changes).disabled(readonly)
      .on_change((value,cx) => {if (!readonly) {owner.include_changes = value; cx.notify();}})));
  return div().id('location-create-form').v_flex().child(new Form().size('large').gap_y(8).children([name,source]));
}
function removal(view,owner) {
  return div().id('worktree-remove-form').v_flex().gap_4()
    .child(div().text_sm().child(view.text('worktree_remove_named').replace('%{name}',owner.entry.path)));
}
/** Picker ids change with each list so its search and selection reset. */
export function pickerId(owner) { return `worktrees-picker-${owner.token}-${owner.kind}`; }
// Worktree lists use the shared command picker, like other searchable resource lists.
function management(view,owner) {
  const choices = owner.choices ?? [];
  return Picker.new(pickerId(owner),{mode:'command',open:true,loading:!!owner.loading && !choices.length,
    title:owner.kind === 'actions' ? owner.title ?? view.text('worktree_manage') : view.text('worktree_manage'),
    empty:view.text('git_no_matches'),items:choices.map(choice => ({id:choice.id,label:choice.label,
      detail:choice.detail ?? '',icon:choice.icon,group:choice.group ?? '',checked:!!choice.checked,
      disabled:!!(owner.pending || choice.disabled || !choice.action),value:choice.id}))});
}
export function render(view) {
  const owner = view.model.dialog;
  if (owner && ['list','actions'].includes(owner.kind)) return div().id('worktrees-controller').child(management(view,owner));
  const title = !owner ? '' : owner.kind === 'create' ? view.text('worktree_create') : owner.kind === 'managed'
    ? view.text(owner.fork ? 'location_fork' : 'location_create') : view.text('worktree_remove');
  const modal = Modal.new(owner?.token ?? 'worktree-dialog',{open:!!owner,form:true,title,
    width:owner?.kind === 'create' ? 488 : 448}).children(owner ? [
      owner.kind === 'create' ? create(view,owner) : owner.kind === 'managed' ? managed(view,owner) : removal(view,owner),
      footer(view,owner,owner.kind === 'create' ? 'live-worktree-submit' : owner.kind === 'managed' ? 'location-create-submit' : 'worktree-remove-confirm',
        owner.kind === 'create' ? 'live-worktree-cancel' : owner.kind === 'managed' ? 'location-cancel' : 'worktree-remove-cancel')
    ] : []);
  return div().id('worktrees-controller').child(modal);
}

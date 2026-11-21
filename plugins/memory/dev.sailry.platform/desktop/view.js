// Existing memory settings rows, catalog toolbar and modal fields use Kit controls.
import {div} from 'gpui-kit';
import {Button,Switch,Tab,TabBar,DropdownMenu,Field,Tag} from 'gpui-component';
import {theme} from 'sailry';
import {TextField} from 'sailry/forms';
import {EmptyState,Modal,SettingsGroup,SelectField,SegmentedTabs,Loading,IconButton} from 'sailry/ui';

function row(view,key,control,description) {
  return div().id(`settings-row-${key}`).h_flex().items_center().w_full().py_3().gap_4().text_sm().flex_wrap()
    .child(div().flex_1().min_w(160).v_flex().gap_1().child(view.text[key])
      .children(description ? [div().text_color(theme().colors.muted_foreground).child(view.text[description])] : []))
    .child(div().h_flex().flex_shrink(0).min_w_0().max_w_full().justify_end().w_64().ml_auto().child(control));
}
function configuration(view) {
  const {text,draft,configuration:state} = view;
  if (!draft) return SettingsGroup.new('memory_configuration',{title:text.memory_configuration})
    .child(div().relative().w_full().h_24().child(Loading.new('memory-configuration-loading')));
  const disabled = !view.connected || state.pending || !!state.request || !!state.error;
  const budget = SelectField.new('memory-budget',{label:text.memory_context_budget,placeholder:text.memory_context_budget,
    selected:String(draft.context_bytes/1024),disabled,items:[2,4,8,16,32,64].map(amount=>({id:String(amount),label:`${amount} KiB`}))});
  const age = SelectField.new('memory-review-age',{label:text.memory_review_age,placeholder:text.memory_review_age,
    selected:String(draft.review_after_days),disabled,items:[30,90,180,365].map(days=>({id:String(days),label:String(days)}))});
  return SettingsGroup.new('memory_configuration',{title:text.memory_configuration})
    .child(row(view,'memory_enabled',new Switch('memory-enabled').checked(draft.enabled).disabled(disabled)
      .on_change((value,cx) => view.configure('enabled',value,cx))))
    .child(row(view,'memory_auto_write',new Switch('memory-auto-write').checked(draft.auto_write).disabled(disabled || !draft.enabled)
      .on_change((value,cx) => view.configure('auto_write',value,cx)),'memory_auto_write_description'))
    .child(row(view,'memory_context_budget',budget,'memory_budget_description'))
    .child(row(view,'memory_review_age',age))
    .children(state.error ? [div().v_flex().gap_2().py_2()
      .child(div().h_flex().gap_2()
        .children(state.error !== 'memory_settings_conflict' ? [new Button('memory-settings-retry').label(text.memory_retry).disabled(state.pending || !view.connected).on_click((_,cx)=>view.saveConfiguration(cx))] : [])
        .children(!state.request ? [new Button('memory-settings-reload').label(text.memory_settings_reload).disabled(state.pending).on_click((_,cx)=>view.reloadConfiguration(cx))] : []))] : []);
}
function toolbar(view) {
  const {text} = view;
  const scope = SelectField.new('memory-scope',{label:text.memory_scope,placeholder:text.memory_all,selected:view.scope,
    items:[{id:'all',label:text.memory_all},{id:'global',label:text.memory_global},...view.projects.map(project=>({id:project.id,label:project.name}))]});
  const tabs = ['active','archived','review'];
  return div().h_flex().items_center().w_full().flex_wrap().gap_3()
    .child(new TabBar('memory-views').variant('segmented').selected_index(tabs.indexOf(view.view))
      .children(tabs.map(key => new Tab().label(text[`memory_${key}`])))
      .on_change((index,cx)=>{view.view=tabs[index];view.refresh(cx);}))
    .child(div().h_flex().items_center().ml_auto().gap_2()
      .child(div().id('memory-search').w_40().child(TextField.new(view.query)))
      .child(div().w_48().child(scope))
      .child(new Button('memory-create').primary().label(text.memory_create).disabled(view.busy()).on_click((_,cx)=>view.edit(null,cx))));
}
function catalog(view) {
  const {text} = view;
  const rows = view.results.map(entry => {
    const review = view.reviews.find(candidate => candidate.summary.id === entry.id);
    return div().id(`memory-${entry.id}`).h_flex().items_center().w_full().gap_3().py_3().text_sm()
      .child(div().flex_1().min_w_0().v_flex().gap_1().child(div().truncate().child(entry.title))
        .children(review ? [div().text_color(theme().colors.muted_foreground).child(text[review.duplicate_of ? 'memory_possible_duplicate' : 'memory_needs_review'])] : [])
        .child(div().h_flex().items_center().gap_2()
          .child(div().text_color(theme().colors.muted_foreground).child(view.kindLabel(entry.kind)))
          .child(new Tag().variant('secondary').size('small').child(view.projectLabel(entry.project)))))
      .child(IconButton.new(`memory-edit-${entry.id}`,{icon:'reicon:newicons/edit',label:text.settings_edit,disabled:view.busy()}))
      .child(IconButton.new(`memory-delete-${entry.id}`,{icon:'reicon:ui/trash2',label:text.settings_delete,disabled:view.busy()}));
  });
  return SettingsGroup.new('settings_memory',{heading:false})
    .children(rows)
    .children(!view.saved ? [view.error && !view.loading
      ? new Button('memory-search-retry').label(text.memory_retry).disabled(!view.connected).on_click((_,cx)=>view.refresh(cx))
      : div().relative().w_full().h_24().child(Loading.new('memory-loading'))]
      : !rows.length ? [EmptyState.new('memory-empty',{variant:'list',icon:'inbox',label:text.memory_empty})] : []);
}
function editor(view) {
  const {editing:owner,text} = view, summary = owner.entry.summary;
  const disabled = owner.loading || owner.pending || !!owner.request;
  const kinds = ['user','feedback','project','reference'];
  let merge = new DropdownMenu('memory-merge-source',text.memory_merge_source);
  const candidates = view.entries.filter(entry => !entry.archived && entry.project === summary.project && entry.id !== summary.id);
  for (const entry of candidates) merge = merge.item(entry.title,cx=>view.merge(entry.id,cx));
  return div().id('memory-editor').v_flex().max_h(window.viewport_size().height*0.8-48).gap_3()
    .child(div().v_flex().min_h_0().overflow_y_scroll().gap_3()
      .child(new Field().label(text.memory_title).child(TextField.new(owner.title,{disabled})))
      .child(new Field().label(text.memory_kind).child(SegmentedTabs.new(`memory-kind-${view.dialogId}`,{selected:summary.kind,disabled,fill:true,
        items:kinds.map(id=>({id,label:view.kindLabel(id)}))})))
      .child(new Field().label(text.memory_body).child(TextField.new(owner.body,{disabled})))
      .children(summary.revision ? [new Switch('memory-archive').label(text.memory_archived).checked(summary.archived).disabled(disabled || !!owner.sources.length)
        .on_change((value,cx)=>{summary.archived=value;cx.notify();})] : [])
      .children(summary.revision && !summary.archived && candidates.length ? [disabled || owner.sources.length ? new Button('memory-merge-source').label(text.memory_merge_source).disabled(true) : merge] : [])
      .children(owner.sources.length ? [div().text_sm().child(text.memory_merge_description)] : []));
}
function editorFooter(view) {
  const owner = view.editing, text = view.text;
  return div().h_flex().justify_end().gap_2()
    .child(new Button('memory-cancel').label(text.settings_cancel).on_click((_,cx)=>view.close(cx)))
    .children(owner.loading && owner.error ? [new Button('memory-reload').label(text.memory_retry).on_click((_,cx)=>view.loadEditor(owner,cx))] : [])
    .child(new Button('memory-save').primary().label(text[owner.request ? 'memory_retry' : owner.sources.length ? 'memory_merge' : 'settings_save'])
      .disabled(owner.pending || owner.loading || !view.connected).on_click((_,cx)=>view.save(cx)));
}
function removal(view) {
  const {removal:owner} = view;
  return div().id('memory-removal').text_sm().child(owner.summary.title);
}
function removalFooter(view) {
  const {text,removal:owner} = view;
  return div().h_flex().justify_end().gap_2()
    .child(new Button('memory-delete-cancel').label(text.settings_cancel).on_click((_,cx)=>view.close(cx)))
    .child(new Button('memory-delete-confirm').danger().label(text[owner.request ? 'memory_retry' : 'settings_delete'])
      .disabled(owner.pending || !view.connected).on_click((_,cx)=>view.confirmRemoval(cx)));
}
export function render(view) {
  const owner = view.editing ?? view.removal;
  const title = view.editing ? view.text[view.editing.entry.summary.revision ? 'memory_editor' : 'memory_create']
    : view.removal ? view.text.memory_delete : '';
  return div().id('memory-settings').v_flex().gap_4()
    .child(configuration(view)).child(toolbar(view))
    .child(catalog(view))
    .child(Modal.new(`memory-dialog-${view.dialogId}`,{open:!!owner,form:true,title,width:view.editing ? 640 : 448})
      .children(view.editing ? [editor(view),editorFooter(view)] : view.removal ? [removal(view),removalFooter(view)] : []));
}

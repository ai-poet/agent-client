import {div} from 'gpui-kit';
import {Button,Switch,Field,Form} from 'gpui-component';
import {TextField} from 'sailry/forms';
import {SecretField} from 'sailry/credentials';
import {SettingsGroup,SettingsRow,SelectField,Menu,Modal,ModelPopup} from 'sailry/ui';
import * as catalog from './catalog.js';

function button(id,label,action,disabled=false) {
  return div().id(id).child(new Button(id).label(label).disabled(disabled).on_click(action));
}
function toggle(id,value,locked,action) {
  return div().id(id).child(new Switch(id).checked(!!value).disabled(locked).on_change(action));
}
function editor(view) {
  const owner=view.editor,text=view.text,locked=view.loading||view.pending||!!view.request;
  let modal=Modal.new(owner?.id??'search-dialog',{open:!!owner,form:true,width:540,
    title:owner?text[`search_driver_${owner.driver}`]:'',dismissable:!locked});
  if(!owner)return modal;
  const form=new Form().size('large').gap_y(8);
  if(owner.driver==='model') {
    form.child(new Field().label(text.search_model_name).child(ModelPopup.new('search-model',{
      label:text.search_model_name,placeholder:text.search_model_required,disabled:locked,catalog:{models:catalog.popup(view.models)},
      selected:owner.model ? `${owner.model.provider}/${owner.model.model}` : null})));
  } else {
    if(owner.driver==='searxng')form.child(new Field().label(text.search_endpoint)
      .child(div().id('search-editor-endpoint').child(TextField.new(owner.endpoint,{readonly:locked}))));
    form.child(new Field().label(text.search_api_key).child(div().id('search-editor-key').child(SecretField.new(owner.key,{disabled:locked}))));
    if(owner.driver==='searxng')for(const key of ['username','password'])form.child(new Field().label(text[`search_${key}`])
      .child(div().id(`search-editor-${key}`).child(SecretField.new(owner[key],{disabled:locked}))));
  }
  return modal.child(div().id('search-editor').child(form))
    .child(div().h_flex().justify_end().gap_2()
      .children(view.saved?.revision!==owner.settings.revision
        ? [button('search-editor-reload',text.search_reload,(_,cx)=>view.reloadEditor(cx),locked)] : [])
      .child(button('search-editor-cancel',text.settings_cancel,(_,cx)=>view.close(cx),locked))
      .child(button('search-editor-save',text[view.request?'search_retry':'settings_save'],(_,cx)=>view.saveEditor(cx),view.loading||view.pending)));
}
export function render(view) {
  const text=view.text,settings=view.saved??{providers:{}},locked=view.busy(),drivers=view.available();
  const selected=settings.driver?.kind==='driver'?settings.driver.driver:settings.driver?.kind??'none';
  const providers=drivers.map(driver=>{
    const config=settings.providers[driver]??{},capability=settings.capabilities?.[driver];
    // The switch already shows whether a service is enabled; describe only what it lacks and its quota.
    const missing=catalog.needsKey(capability)&&!config.key_configured?'search_key_required'
      :driver==='searxng'&&!config.endpoint?'search_endpoint_required'
      :driver==='model'&&!catalog.contains(view.models,config.model)?'search_model_required':null;
    const detail=[missing,capability?.quota?`search_quota_${capability.quota}`:null].filter(Boolean).map(key=>text[key]).join(' · ');
    // The application's settings row: brand mark, name, what is missing; drag the handle to reorder.
    return SettingsRow.new(`${catalog.row}${driver}`,{label:text[`search_driver_${driver}`],description:detail,icon:catalog.icon(driver)})
      .child(toggle(`search-enabled-${driver}`,config.enabled,locked,(value,cx)=>view.enabled(driver,value,cx)))
      .child(Menu.new(`search-more-${driver}`,{label:text.search_more,icon:'ellipsis',disabled:locked,items:[
        {id:'edit',label:text.settings_edit,icon:'settings-2'},{id:'test',label:text.search_test,icon:'play'}]}));
  });
  return div().id('search-settings').v_flex().gap_6()
    .children(!view.saved&&!view.loading?[button('search-reload',text.search_reload,(_,cx)=>view.load(cx))]:[])
    .children(view.request?.writing&&!view.editor?[button('search-retry',text.search_retry,(_,cx)=>view.retry(cx),view.pending)]:[])
    .child(SettingsGroup.new('search_settings',{title:text.search_settings})
      .child(SettingsRow.new('search-driver-row',{label:text.search_default_driver,wide:true}).child(SelectField.new('search-default-driver',{
        label:text.search_default_driver,disabled:locked,selected,items:['auto',...drivers,'none'].map(id=>({id,label:text[`search_driver_${id}`]}))})))
      .child(SettingsRow.new('search-fallback',{label:text.search_fallback}).child(toggle('search-fallback-toggle',settings.fallback,locked||selected==='model',
        (value,cx)=>view.toggle('fallback',value,cx)))))
    .child(SettingsGroup.new('search_providers',{title:text.search_providers,sortable:true,sort_label:text.search_reorder,sort_disabled:locked}).children(providers))
    .child(editor(view));
}

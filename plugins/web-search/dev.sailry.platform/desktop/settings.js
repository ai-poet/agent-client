// Search settings and editing workflows belong to the package, not Desktop.
import {View} from 'gpui-kit';
import {context} from 'sailry';
import {prepareRequest,completeRequest,forgetRequest,nextChange,faultCode,listModels} from 'sailry/sdk';
import {createText,readText,releaseText} from 'sailry/forms';
import {createSecret,releaseSecret,describeSecret} from 'sailry/credentials';
import {prepareSettings} from 'sailry/search';
import {nextControlEvent,nextMenuEvent,modal_closed,toast} from 'sailry/ui';
import {messages} from './locales.js';
import {render} from './view.js';
import * as catalog from './catalog.js';

const clone = value => JSON.parse(JSON.stringify(value));
export const drivers = catalog.drivers;
export default class Settings extends View {
  init(_props,cx) {
    this.text=messages(JSON.parse(context()).locale); this.saved=null;this.models=[];
    this.pending=false; this.loading=false; this.refresh=false; this.connected=true;
    this.request=null; this.editor=null; this.serial=0; this.load(cx);
    cx.spawn(async cx=>{let cursor;try {while(true) {
      const update=await nextChange(cursor),first=cursor===undefined;cursor=update.cursor;
      this.connected=update.connected;if(update.connected&&!first)this.load(cx);cx.notify();
    }} catch(_) { /* The captured scope ends with the view. */ }});
    cx.spawn(async cx=>{try {while(true)this.control(await nextControlEvent(),cx);} catch(_) {}});
    cx.spawn(async cx=>{try {while(true)this.menu(await nextMenuEvent(),cx);} catch(_) {}});
    cx.spawn(async cx=>{try {while(true) {
      const id=await modal_closed();if(id===this.editor?.id)this.close(cx);
    }} catch(_) {}});
  }
  busy() {return !this.saved||!this.connected||this.loading||this.pending||!!this.request;}
  fail(error,writing=false,testing=false) {
    const code=typeof error==='string'?error:error?.code,details=error?.details;
    const key=details?.kind==='search_unsupported'?'search_not_supported'
      :code==='expired'?'search_sign_in'
      :code==='revision_conflict'?'search_settings_conflict':code==='invalid_request'&&!testing?'search_settings_invalid'
      :code==='not_configured'?'search_not_configured':testing&&(code==='permission_denied'||details?.kind==='model_request'&&details.reason==='unauthorized')?'search_test_auth_failed'
      :writing&&['unavailable','outcome_unknown'].includes(code)?'search_save_unknown':'search_request_failed';
    toast({id:'search-operation',message:this.text[key],kind:'error'});
  }
  load(cx) {
    if(!this.connected)return;
    if(this.loading||this.pending||this.request){this.refresh=true;return;}
    this.loading=true;
    cx.notify();
    cx.spawn(async cx=>{let request;try {
      request=prepareRequest({kind:'read_search_settings'});
      const [result,models]=await Promise.all([completeRequest(request),listModels().catch(()=>null)]);
      if(result.Err)this.fail(result.Err);else {this.saved=result.Ok.data;if(models)this.models=catalog.models(models);}
    } catch(error){this.fail(faultCode(String(error.message)));}
    finally {if(request)forgetRequest(request);this.loading=false;
      const refresh=this.refresh;this.refresh=false;if(refresh)this.load(cx);cx.notify();}});
  }
  save(settings,cx) {
    if(this.busy())return;
    this.request={id:prepareRequest({kind:'save_search_settings',data:{settings,credentials:{}}}),writing:true};
    this.retry(cx);
  }
  toggle(key,value,cx) {
    if(this.busy())return;const settings=clone(this.saved);settings[key]=value;this.save(settings,cx);
  }
  enabled(driver,value,cx) {
    if(this.busy())return;const settings=clone(this.saved);settings.providers[driver].enabled=value;this.save(settings,cx);
  }
  available() {return catalog.available(this.saved);}
  // rows are the service rows' identifiers as the host dropped them.
  reorder(rows,cx) {
    if(this.busy())return;
    const order=catalog.reordered(this.saved,rows);
    if(!order)return;
    const settings=clone(this.saved);settings.order=order;
    // Show the dropped order while it is saved; a failed save reloads the stored one.
    this.saved.order=order;this.save(settings,cx);
  }
  test(driver,cx) {
    if(this.busy())return;
    this.request={id:prepareRequest({kind:'test_search_driver',data:{driver}}),testing:true};this.retry(cx);
  }
  retry(cx) {
    if(!this.request||this.pending||!this.connected)return;
    const request=this.request;this.pending=true;cx.notify();
    cx.spawn(async cx=>{try {
      const result=await completeRequest(request.id);
      if(result.Err){this.fail(result.Err,request.writing,request.testing);if(result.Err.code==='outcome_unknown'&&request.writing)return;}
      else {
        if(result.Ok.kind==='search_settings')this.saved=result.Ok.data;
        toast({id:'search-operation',message:this.text[request.testing?'search_test_passed':'search_saved'],kind:'info'});
        if(request.editor&&this.editor===request.editor)this.close(cx,true);
      }
      forgetRequest(request.id);this.request=null;
    } catch(_){this.fail('outcome_unknown',request.writing,request.testing);
      if(request.testing){forgetRequest(request.id);this.request=null;}}
    finally {this.pending=false;if(!this.request)this.load(cx);cx.notify();}});
  }
  edit(driver,cx) {
    if(this.busy()||this.editor)return;
    const config=this.saved.providers[driver],text=this.text;
    if(!config)return;
    if(driver==='model') {
      this.editor={id:`search-editor-${++this.serial}`,driver,settings:clone(this.saved),model:clone(config.model ?? null)};
      cx.notify();return;
    }
    this.editor={id:`search-editor-${++this.serial}`,driver,settings:clone(this.saved),
      endpoint:createText(config.endpoint??'',{label:text.search_endpoint}),
      key:createSecret({label:text.search_api_key,placeholder:text[config.key_configured?'search_configured':'search_api_key']}),
      username:createSecret({label:text.search_username,placeholder:text[config.basic_configured?'search_configured':'search_username']}),
      password:createSecret({label:text.search_password,placeholder:text[config.basic_configured?'search_configured':'search_password']})};
    cx.notify();
  }
  saveEditor(cx) {
    if(this.loading)return;
    if(this.request){this.retry(cx);return;}
    if(this.pending||!this.connected||!this.editor)return;
    const editor=this.editor,settings=clone(editor.settings);
    if(editor.driver==='model') {
      if(!catalog.contains(this.models,editor.model)) {
        toast({id:'search-operation',message:this.text.search_model_unavailable,kind:'error'});return;
      }
      settings.providers.model.model=clone(editor.model);
      try {
        this.request={id:prepareRequest({kind:'save_search_settings',data:{settings,credentials:{}}}),writing:true,editor};
        this.retry(cx);
      } catch(_){toast({id:'search-operation',message:this.text.search_settings_invalid,kind:'error'});}
      return;
    }
    if(editor.driver==='searxng')settings.providers.searxng.endpoint=readText(editor.endpoint).trim()||null;
    // Secret contents stay native; an edited field left empty removes the stored secret.
    const key=describeSecret(editor.key),username=describeSecret(editor.username),password=describeSecret(editor.password);
    if(username.filled!==password.filled){toast({id:'search-operation',message:this.text.search_basic_invalid,kind:'error'});return;}
    try {
      const id=prepareSettings({settings,driver:editor.driver,
        key:key.filled?{action:'set',secret:editor.key}:key.version>0?{action:'clear'}:{action:'keep'},
        basic:username.filled?{action:'set',username:editor.username,password:editor.password}
          :username.version>0||password.version>0?{action:'clear'}:{action:'keep'}});
      this.request={id,writing:true,editor};this.retry(cx);
    } catch(_){toast({id:'search-operation',message:this.text.search_settings_invalid,kind:'error'});}
  }
  reloadEditor(cx) {
    if(this.loading||this.pending||this.request||!this.editor||!this.saved)return;
    this.editor.settings=clone(this.saved);cx.notify();
  }
  close(cx,confirmed=false) {
    if(!confirmed&&(this.pending||this.request))return;
    if(this.editor?.endpoint)releaseText(this.editor.endpoint);
    if(this.editor)[this.editor.key,this.editor.username,this.editor.password].filter(Boolean).forEach(releaseSecret);
    this.editor=null;cx.notify();
  }
  control(event,cx) {
    const editor=this.editor;
    if(editor?.driver==='model'&&!this.loading&&!this.pending&&!this.request) {
      if(event.id==='search-model')editor.model=catalog.chosen(this.models,event.value?.model) ?? editor.model;
      cx.notify();
    }
    if(event.id==='search-default-driver'&&!this.busy()) {
      const settings=clone(this.saved);
      settings.driver=event.value==='none'?null:event.value==='auto'?{kind:'auto'}:{kind:'driver',driver:event.value};
      if (settings.driver?.kind==='driver') {
        const config=settings.providers[settings.driver.driver];if(!config)return;config.enabled=true;
      }
      this.save(settings,cx);
    }
    if(event.id==='search_providers')this.reorder(event.value?.order,cx);
  }
  menu(event,cx) {
    const driver=this.available().find(driver=>event.menu===`search-more-${driver}`);
    if(driver&&event.id==='edit')this.edit(driver,cx);
    if(driver&&event.id==='test')this.test(driver,cx);
  }
  render(){return render(this);}
}

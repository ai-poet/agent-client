import {View} from 'gpui-kit';
import {context,header_action,focusTerminal,releaseTerminal} from 'sailry';
import {nextChange,readProjectCatalog,prepareRequest,completeRequest,forgetRequest,faultCode} from 'sailry/sdk';
import {listSsh,newSshId,prepareSsh,listSshTerminals,prepareSshTerminal} from 'sailry/connections';
import {createText,readText,releaseText,setText,focusText,nextTextEvent} from 'sailry/forms';
import {createSecret,releaseSecret,describeSecret,chooseSecretFile} from 'sailry/credentials';
import {nextTreeEvent,nextContextMenuEvent,nextNavigationTabEvent,nextPickerEvent,nextControlEvent,modal_closed,writeClipboard,toast} from 'sailry/ui';
import {uploadDropped,selectUpload,selectDownload,nextTransfers,transferAction,previewFile} from 'sailry/ssh-transfers';
import {draft,prepare} from './editor.js';
import {Request,failure} from './requests.js';
import {Mutations} from './mutations.js';
import {Files,join,parent,name} from './files.js';
import {messages,format} from './locales.js';
import {render} from './view.js';
export const command=(kind,data)=>({kind,data});
export default class Ssh extends View {
  // Exited terminals: toasted once, then closed once.
  ended=new Set();dismissed=new Set();
  init(_props,cx) {
    this.text=messages(JSON.parse(context()).locale);this.profiles=[];this.projects=[];this.worktrees=[];this.connected=true;this.loaded=false;
    this.selected=null;this.terminals=[];this.terminal=null;this.browsers=new Map();this.pending=null;this.outcome=null;this.error=null;this.target=null;this.resume=false;this.refreshRevision=0;this.refreshing=null;this.refreshAgain=false;
    this.terminalSelection=0;this.terminalClose=null;this.closingTerminals=new Set();this.home=false;
    this.editor=null;this.picker=false;this.pickerProfile=null;this.pickerRevision=0;this.confirmation=null;this.fileEdit=null;this.commandOpen=false;this.transfer=null;this.dialog=0;this.jobs=[];this.transfersObserved=false;this.installing=null;
    this.command=createText('',{multiline:true,rows:[2,6],placeholder:this.text.ssh_command_hint});this.output=[createText('',{multiline:true,rows:[2,12]}),createText('',{multiline:true,rows:[2,12]})];
    this.watch(nextChange,(change,cx)=>{this.connected=change.connected;if(change.connected)this.run(cx=>this.refresh(cx),cx);},cx,true);
    this.watch(nextTransfers,(value,cx)=>this.transfers(value,cx),cx,true);
    this.watch(nextTreeEvent,(event,cx)=>this.run(cx=>this.tree(event,cx),cx),cx);
    this.watch(nextContextMenuEvent,(event,cx)=>this.run(cx=>event.id?.startsWith('connection:')?this.connectionMenu(event.id,cx):this.fileAction(event.id,cx),cx),cx);
    this.watch(nextNavigationTabEvent,(event,cx)=>{if(event.bar!=='ssh-tabs')return;if(event.kind==='close')this.run(cx=>this.closeTerminal(event.id,cx),cx);else this.selectTerminal(event.id,cx,true);},cx);
    this.watch(header_action,(id,cx)=>{if(id==='ssh-connections-menu')this.connections(cx);else if(id==='ssh-home')this.showHome(cx);},cx);
    this.watch(nextPickerEvent,(event,cx)=>this.pick(event,cx),cx);
    this.watch(nextControlEvent,(event,cx)=>this.control(event,cx),cx);
    this.watch(nextTextEvent,(event,cx)=>this.textEvent(event,cx),cx);
    this.watch(modal_closed,(id,cx)=>{if(id===`ssh-dialog-${this.dialog}`)this.close(cx);},cx);
  }
  watch(read,handle,cx,cursor=false){cx.spawn(async cx=>{let seen='';try{while(true){const value=await read(...(cursor?[seen]:[]));if(cursor)seen=value.cursor;handle(value,cx);cx.notify();}}catch(_){}});}
  run(action,cx){cx.spawn(async cx=>{try{await action(cx);}catch(error){this.report({message:error.message});}finally{cx.notify();}});}
  report(error,key=failure(error)){this.error=error;const label=this.text[key];toast({id:'ssh-error',kind:'error',message:error.message&&error.message!==label?`${label}\n${error.message}`:label});}
  transfers(value,cx){const previous=this.jobs;this.jobs=value.transfers;if(!this.transfersObserved){this.transfersObserved=true;return;}for(const job of this.jobs){const old=previous.find(entry=>entry.id===job.id);
    if(old?.stage!==job.stage||JSON.stringify(old.error)!==JSON.stringify(job.error)){
      if(job.stage==='cancelled')toast({id:`ssh-transfer-${job.id}`,kind:'info',message:this.text.ssh_cancelled});
      else if(job.error||job.stage==='uncertain')toast({id:`ssh-transfer-${job.id}`,kind:'error',message:this.text[job.error?.message]??this.text[job.stage==='uncertain'?'ssh_files_uncertain':'ssh_files_action_failed']});
      if(job.stage==='done'){toast({id:`ssh-transfer-${job.id}`,kind:'info',message:this.text.ssh_transfer_done});const browser=this.browsers.get(job.profile);if(browser)this.run(cx=>browser.load(browser.path,cx),cx);}
    }
  }}
  locked(){return !this.connected||!!this.pending;}
  connectionLocked(profile){const pending=this.pending;return !this.connected||!!pending&&!(pending.target?.id===profile.id&&pending.request.command?.kind==='open_ssh_terminal'&&pending.request.unknown&&!pending.request.running);}
  profile(){return this.profiles.find(profile=>profile.id===this.selected);}
  activeProfile(){const active=this.terminals.find(info=>info.id===this.terminal);return this.profiles.find(profile=>profile.id===active?.ssh);}
  showHome(cx){if(this.locked())return;this.home=true;++this.terminalSelection;this.terminalClose=null;this.picker=false;cx.notify();}
  connectionMenu(id,cx){if(this.locked()||!id?.startsWith('connection:'))return;const target=JSON.parse(id.slice(11)),profile=this.profiles.find(profile=>profile.id===target.id&&profile.revision===target.revision);if(!profile)return;if(target.action==='edit')this.edit(profile,cx);else if(target.action==='remove')this.remove(profile,cx);}
  control(event,cx){
    if(event.id==='ssh-close-current'){const id=this.terminal;if(id&&!this.home&&!this.locked())this.run(cx=>this.closeTerminal(id,cx),cx);return;}
    if(event.id==='ssh-new'){if(!this.locked())this.edit(null,cx);return;}
    if(event.id===`ssh-worktree-${this.dialog}`){const transfer=this.transfer;if(transfer&&!this.locked()&&this.worktrees.some(tree=>tree.project&&tree.id===event.value)){transfer.worktree=event.value;cx.notify();}return;}
    if(event.id?.startsWith('ssh-open-')){const profile=this.profiles.find(profile=>event.id===`ssh-open-${profile.id}`);if(profile&&!this.connectionLocked(profile))this.run(cx=>this.open(profile,cx),cx);return;}
    if(event.id==='ssh-files-refresh'){const files=this.files();if(files&&!files.pending)this.run(cx=>files.load(files.path,cx),cx);return;}
    if(event.id==='ssh-files-pathbar'){const files=this.files(),segment=files?.segments()[event.value];if(files&&!files.pending&&Number.isInteger(event.value)&&segment)this.run(cx=>files.load(segment.path,cx),cx);return;}
    const editor=this.editor;if(!editor||this.locked()||describeSecret(editor.key).picking||event.id!==`ssh-authentication-${this.dialog}`)return;
    if(['password','private_key','key_path'].includes(event.value)){editor.authentication=event.value;cx.notify();}}
  editPath(cx){const files=this.files();if(files&&!files.editing&&!files.pending){files.editing=true;setText(files.input,files.path);focusText(files.input);cx.notify();}}
  // Escape and leaving the field discard the typed path; only Enter loads it.
  cancelPath(cx){const files=this.files();if(!files?.editing||files.pending)return false;files.editing=false;setText(files.input,files.path);cx.notify();return true;}
  textEvent(event,cx){const files=this.files();if(!files?.editing||event.id!==files.input)return;
    if(event.kind==='blur'){this.cancelPath(cx);return;}
    if(event.kind==='enter'&&!files.pending){const path=readText(files.input);files.editing=false;this.run(cx=>files.load(path,cx),cx);}}
  pathKey(event,cx){if(event.key==='escape'&&this.cancelPath(cx))cx.stop_propagation();}
  connections(cx){if(this.locked())return;this.picker=true;this.pickerProfile=null;this.pickerRevision++;cx.notify();}
  pick(event,cx){if(!this.picker||event.picker!==`ssh-picker-${this.pickerRevision}`)return;
    if(event.kind==='close'){this.picker=false;cx.notify();return;}if(this.locked()||event.kind!=='select')return;
    const choice=event.value;if(choice?.kind==='profile'){if(!this.profiles.some(profile=>profile.id===choice.id))return;this.pickerProfile=choice.id;this.pickerRevision++;cx.notify();return;}
    if(choice?.kind==='back'){this.pickerProfile=null;this.pickerRevision++;cx.notify();return;}
    if(choice?.kind==='add'){this.picker=false;this.edit(null,cx);return;}
    const profile=this.profiles.find(profile=>profile.id===this.pickerProfile);if(!profile||choice?.kind!=='action')return;
    this.selected=profile.id;this.picker=false;
    if(choice.action==='open')this.run(cx=>this.open(profile,cx),cx);
    if(choice.action==='edit')this.edit(profile,cx);
    if(choice.action==='remove')this.remove(profile,cx);
    if(choice.action==='check'){this.resume=false;this.run(cx=>this.execute(command('check_ssh',{profile:profile.id,expected_revision:profile.revision}),cx,null,null,profile),cx);}
    if(choice.action==='run'){this.outcome=null;this.error=null;this.commandOpen=true;this.dialog++;}
    if(choice.action==='transfer')this.transferDialog(cx);
    cx.notify();
  }
  files(){const info=this.terminals.find(info=>info.id===this.terminal);return info&&this.profiles.some(profile=>profile.id===info.ssh)?this.browsers.get(info.ssh):null;}
  refresh(cx){++this.refreshRevision;this.refreshAgain=true;
    if(!this.refreshing)this.refreshing=(async()=>{try{while(this.refreshAgain){this.refreshAgain=false;await this.readInventory(this.refreshRevision,cx);}}finally{this.refreshing=null;}})();
    return this.refreshing;
  }
  async readInventory(revision,cx){const known=[...this.terminals],terminalIndex=known.findIndex(info=>info.id===this.terminal);
    const inventory=(async()=>{const profiles=await listSsh();
      const selected=this.selected??profiles[0]?.id??null,ids=new Set(known.map(info=>info.ssh));if(selected)ids.add(selected);
      const lists=await Promise.all([...ids].filter(id=>profiles.some(profile=>profile.id===id)).map(id=>listSshTerminals(id)));
      const terminals=new Map(lists.flat().map(info=>[info.id,info]));
      const missing=await Promise.all(known.filter(previous=>!terminals.has(previous.id)).map(previous=>this.readTerminal(previous)));
      for(const info of missing)if(info&&info.status.kind!=='closed')terminals.set(info.id,info);
      return {profiles,selected,terminals};
    })();
    const [{profiles,selected,terminals},catalog]=await Promise.all([inventory,readProjectCatalog()]);
    if(revision!==this.refreshRevision)return;this.projects=catalog.projects;this.worktrees=catalog.worktrees;
    if(this.transfer?.worktree&&!this.worktrees.some(tree=>tree.project&&tree.id===this.transfer.worktree))this.transfer.worktree=null;
    this.profiles=profiles;this.selected=selected;this.terminals=[...terminals.values()];this.loaded=true;for(const id of this.closingTerminals)if(!terminals.has(id))this.closingTerminals.delete(id);if(!this.terminals.some(info=>info.id===this.terminal))this.terminal=this.terminals[Math.min(Math.max(terminalIndex,0),this.terminals.length-1)]?.id??null;if(this.terminal)this.selectTerminal(this.terminal,cx);this.focusSuccessor(cx);this.endExited(cx);
  }
  // Any exit closes the tab once; exit 0 is silent.
  endExited(cx){for(const info of this.terminals){const kind=info.status.kind;if((kind!=='exited'&&kind!=='failed')||this.ended.has(info.id))continue;this.ended.add(info.id);
    if(kind==='failed'||info.status.data?.code!==0)toast({id:`ssh-exit-${info.id}`,kind:'error',message:kind==='exited'?format(this.text.ssh_terminal_exited,{code:info.status.data?.code}):this.text.ssh_terminal_ended});}
    this.closeEnded(cx);}
  // Closing resets command outcomes; never discard a pending host-key decision.
  closeEnded(cx){if(this.locked()||this.outcome?.kind==='host_key_required')return;const info=this.terminals.find(info=>this.ended.has(info.id)&&!this.dismissed.has(info.id)&&!this.closingTerminals.has(info.id));if(!info)return;this.dismissed.add(info.id);this.run(cx=>this.closeTerminal(info.id,cx),cx);}
  async readTerminal(previous){const id=prepareRequest(command('inspect_terminal',{terminal:previous.id}));try{const result=await completeRequest(id);if(result.Ok?.kind==='terminal_snapshot')return result.Ok.data.info;if(result.Err?.code==='not_found')return null;return previous;}catch(error){return faultCode(error.message??String(error))==='not_found'?null:previous;}finally{forgetRequest(id);}}
  async execute(value,cx,after=null,id=null,target=this.profile(),prepared=null){
    if(this.locked())return null;const request=prepared??new Request(value,id);this.pending={request,after,target};this.outcome=this.commandOpen&&this.outcome?.kind==='completed'?this.outcome:null;this.error=null;this.target=target?{id:target.id,revision:target.revision}:null;
    await request.start(cx);if(request.unknown)this.report(request.uncertainty??{code:'outcome_unknown'});return this.finish(cx);
  }
  async finish(cx){const pending=this.pending;if(!pending?.request.done)return null;this.pending=null;const{request,after,target}=pending;request.release();if(request.settled)await request.settled(cx);if(request.error){this.report(request.error);return null;}const output=request.output;
    if(output.kind==='ssh_outcome'){this.outcome=output.data;
      if(output.data.kind==='connected')toast({id:'ssh-success',kind:'info',message:this.text.ssh_connected});
      if(output.data.kind==='transferred')toast({id:'ssh-success',kind:'info',message:format(this.text.ssh_transferred,{size:output.data.bytes})});
      if(output.data.kind==='terminal'){const info=output.data;await this.refresh(cx);this.selectTerminal(info.id,cx,true);this.picker=false;}
      if(output.data.kind==='host_key_required'){this.target={id:target.id,revision:target.revision};this.dialog++;}
      if(output.data.kind==='completed'){setText(this.output[0],output.data.stdout);setText(this.output[1],output.data.stderr);toast({id:'ssh-command-result',kind:output.data.exit_code===0?'info':'error',message:format(this.text.command_exited,{code:output.data.exit_code})});}
    }
    if(['ssh_profile','ssh_profiles','terminal'].includes(output.kind))await this.refresh(cx);
    if(after)await after(output,cx);cx.notify();return output;
  }
  async inspect(cx){if(this.pending){const request=this.pending.request;await request.check(cx);if(request.unknown)this.report(request.uncertainty??{code:'outcome_unknown'});await this.finish(cx);}}
  async open(profile,cx){if(this.connectionLocked(profile))return;
    // Reopening an unconfirmed connection checks its original request first.
    // It never starts a second remote shell while that result is unknown.
    if(this.pending){await this.inspect(cx);return;}
    this.selected=profile.id;this.resume=true;
    const terminals=await listSshTerminals(profile.id),existing=terminals.find(info=>info.status.kind==='running');
    if(existing){this.terminals=[...new Map([...this.terminals,...terminals].map(info=>[info.id,info])).values()];this.selectTerminal(existing.id,cx,true);this.picker=false;return;}
    this.home=true;this.picker=false;++this.terminalSelection;this.terminalClose=null;
    await this.execute(command('open_ssh_terminal',{profile:profile.id,expected_revision:profile.revision}),cx,null,prepareSshTerminal(profile.id,profile.revision),profile);
  }
  selectTerminal(id,cx,focus=false){const info=this.terminals.find(info=>info.id===id);if(!info)return;if(focus)this.home=false;this.terminal=id;if(!this.commandOpen)this.selected=info.ssh;const profile=this.profiles.find(profile=>profile.id===info.ssh);if(profile){let files=this.browsers.get(profile.id);if(!files||JSON.stringify(files.profile)!==JSON.stringify(profile)){files?.close();files=new Files(profile,(error,key)=>this.report(error,key),this.text);this.browsers.set(profile.id,files);this.run(cx=>files.load('.',cx),cx);}}else{this.browsers.get(info.ssh)?.close();this.browsers.delete(info.ssh);}if(focus){++this.terminalSelection;this.terminalClose=null;focusTerminal(id);}cx.notify();}
  // A newer observation can supersede the command's refresh before it prunes the closed tab.
  focusSuccessor(cx){const close=this.terminalClose;if(!close)return;if(close.selection!==this.terminalSelection){this.terminalClose=null;return;}if(this.terminals.some(info=>info.id===close.id))return;this.terminalClose=null;if(this.terminal)this.selectTerminal(this.terminal,cx,true);}
  async closeTerminal(id,cx){
    if(this.locked())return;const active=!this.home&&this.terminal===id,selection=this.terminalSelection;
    const request=new Request(command('close_ssh_terminal',{terminal:id}));
    request.settled=()=>{if(request.error)this.closingTerminals.delete(id);};
    this.closingTerminals.add(id);
    try{releaseTerminal(id);cx.notify();const output=await this.execute(request.command,cx,null,null,this.profile(),request);
      if(output&&active&&selection===this.terminalSelection){this.terminalClose={id,selection};this.focusSuccessor(cx);}return output;
    }catch(error){if(!request.unknown&&!request.output){this.closingTerminals.delete(id);if(!request.done&&!this.pending)forgetRequest(request.id);}throw error;
    }finally{cx.notify();}
  }
  trust(cx){const target=this.target,key=this.outcome?.key;if(!target||!key||this.locked())return;this.run(cx=>this.execute(command('trust_ssh',{profile:target.id,expected_revision:target.revision,key}),cx,async()=>{this.outcome=null;await this.refresh(cx);const profile=this.profiles.find(profile=>profile.id===target.id);if(this.resume&&profile)await this.open(profile,cx);}),cx);}
  edit(profile,cx){if(this.locked())return;this.close(cx);const editor=draft(profile,newSshId());editor.inputs=editor.fields.map((value,index)=>createText(value,{label:this.text[['settings_name','ssh_host','ssh_port','ssh_username'][index]],placeholder:this.text[['form_name_hint','form_host_hint','form_port_hint','form_username_hint'][index]]}));
    editor.password=createSecret({label:this.text.ssh_password,placeholder:this.text.form_password_hint});editor.key=createSecret({label:this.text.ssh_private_key,placeholder:this.text.form_private_key_hint,multiline:true});editor.passphrase=createSecret({label:this.text.ssh_passphrase,placeholder:this.text.form_passphrase_hint});this.editor=editor;this.dialog++;cx.notify();}
  save(cx){if(!this.editor||this.locked()||describeSecret(this.editor.key).picking)return;const editor=this.editor;this.run(async cx=>{editor.fields=editor.inputs.map(readText);const key=describeSecret(editor.key),password=describeSecret(editor.password),passphrase=describeSecret(editor.passphrase);const prepared=prepare(editor,{password:password.filled,key:key.filled,file:key.file});
    const credential=prepared.replace?prepared.source==='password'?{kind:'password',secret:editor.password}:{kind:'private_key',secret:editor.key,passphrase:passphrase.filled?editor.passphrase:null}:null;
    await this.execute(null,cx,async()=>{await this.refresh(cx);if(this.editor===editor)this.close(cx);},prepareSsh({profile:prepared.profile,credential}));},cx);}
  chooseKey(cx){const editor=this.editor;if(!editor||this.locked()||describeSecret(editor.key).picking)return;this.run(async cx=>{if(this.editor!==editor||this.locked()||describeSecret(editor.key).picking)return;const picking=chooseSecretFile(editor.key,this.text.ssh_key_choose);cx.notify();await picking;if(this.editor===editor)cx.notify();},cx);}
  close(cx){if(this.editor){this.editor.inputs.forEach(releaseText);[this.editor.password,this.editor.key,this.editor.passphrase].forEach(releaseSecret);}if(this.fileEdit?.input)releaseText(this.fileEdit.input);if(this.transfer)this.transfer.inputs.forEach(releaseText);this.editor=null;this.fileEdit=null;this.transfer=null;this.confirmation=null;this.picker=false;this.commandOpen=false;if(this.outcome?.kind==='host_key_required'){this.outcome=null;this.resume=false;}cx.notify();}
  remove(profile,cx){if(this.locked())return;this.selected=profile.id;this.picker=false;this.confirmation={label:'settings_delete',text:format(this.text.ssh_remove_confirm,{name:profile.name}),run:cx=>this.execute(command('remove_ssh',{profile:profile.id,expected_revision:profile.revision}),cx,null,null,profile)};this.dialog++;cx.notify();}
  confirm(cx){const dialog=this.confirmation;if(!dialog||this.locked())return;this.confirmation=null;this.run(dialog.run,cx);}
  runCommand(cx){const profile=this.profile(),text=readText(this.command);if(!profile||!text.trim()||this.locked())return;this.resume=false;this.run(cx=>this.execute(command('run_ssh',{profile:profile.id,expected_revision:profile.revision,command:text,timeout_ms:300000}),cx,null,null,profile),cx);}
  transferDialog(cx){if(this.locked())return;this.picker=false;this.transfer={profile:this.profile(),worktree:this.worktrees.find(tree=>tree.project)?.id??null,inputs:[createText('',{label:this.text.ssh_project_path,placeholder:this.text.form_source_path_hint}),createText('',{label:this.text.ssh_remote_path,placeholder:this.text.form_destination_path_hint})]};this.error=null;this.outcome=null;this.dialog++;cx.notify();}
  transferFile(direction,cx){const value=this.transfer;if(!value?.worktree||this.locked())return;this.resume=false;const data={profile:value.profile.id,expected_revision:value.profile.revision,transfer:{worktree:value.worktree,path:readText(value.inputs[0]),remote_path:readText(value.inputs[1]),direction},timeout_ms:300000};
    const run=cx=>this.execute(command('transfer_ssh',data),cx,null,null,value.profile);if(direction==='upload'){this.confirmation={label:'ssh_upload',text:format(this.text.ssh_upload_confirm,{path:data.transfer.remote_path}),run};cx.notify();}else this.run(run,cx);}
  async tree(event,cx){const files=this.files();if(!files||event.tree!=='ssh-files')return;
    const placeholder=files.placeholder(event.id);if(placeholder){if(['open','select'].includes(event.kind)&&['retry','more'].includes(placeholder.kind))await files.loadBranch(placeholder.path,cx,placeholder.kind==='more');return;}
    if(event.kind==='expand'){await files.expand(event.id,cx);return;}
    if(event.kind==='collapse'){files.expanded.delete(event.id);cx.notify();return;}
    // A file opens in the application's preview; a folder's row expands it.
    if(event.kind==='open'){const entry=files.entry(event.id);if(entry&&entry.kind!=='directory')previewFile(files.profile,event.id);return;}
    if(event.kind==='external_drop'&&event.files){if(this.locked())return;const directory=files.entry(event.id)?.kind==='directory'?event.id:event.id?parent(event.id):files.path;for(const id of uploadDropped(files.profile,directory,event.files))transferAction(id,'start');return;}
    if(event.kind==='select'){files.select(event.id,event);cx.notify();return;}
    if(event.kind==='context'){if(!files.selected.includes(event.id))files.selected=[event.id];files.current=event.id;return;}
    if(event.kind==='command'){if(event.action==='select_all'){files.selected=files.visible();return;}return this.fileAction(this.fileId(event.action,event.id),cx);}
    if(event.kind==='menu')return this.fileAction(event.action,cx);
    if(event.kind==='open'){const entry=files.entry(event.id);if(entry?.kind==='directory')await files.expand(event.id,cx);else if(entry)await this.download([event.id],cx);}
    if(event.kind==='drop'&&event.source){const destination=files.entry(event.id)?.kind==='directory'?event.id:event.id?parent(event.id):files.path,paths=files.selected.includes(event.source)?[...files.selected]:[event.source];if(paths.every(path=>path!==destination&&parent(path)!==destination&&!destination.startsWith(`${path}/`)))await this.mutate(this.fileScope(files),paths.map(path=>({path,action:{operation:'move',destination:join(destination,name(path))}})),cx);}
  }
  fileScope(files=this.files()){return files?{files,profile:files.profile,revision:files.revision}:null;}
  fileOwner(scope){return !!scope&&this.files()===scope.files&&!scope.files.closed&&this.profiles.some(profile=>profile.id===scope.profile.id&&profile.revision===scope.profile.revision);}
  fileCurrent(scope){return this.fileOwner(scope)&&scope.files.revision===scope.revision;}
  fileId(action,path){const files=this.files();return JSON.stringify({action,path,profile:files.profile.id,revision:files.profile.revision,generation:files.revision});}
  fileMenu(path){const files=this.files(),locked=this.locked()||files.pending;return ['new_file','new_directory','upload','download','copy','cut','paste','rename','delete','copy_path','refresh'].map(action=>({id:this.fileId(action,path),label:this.text[{new_file:'files_new_file',new_directory:'files_new_directory',upload:'files_upload',download:'files_download',copy:'files_copy',cut:'files_cut',paste:'files_paste',rename:'files_rename',delete:'settings_delete',copy_path:'files_copy_path',refresh:'refresh'}[action]],enabled:!locked&&(action!=='paste'||!!files.clipboard)&&(action!=='rename'||files.selected.length<=1)}));}
  async fileAction(id,cx){const files=this.files();if(!files||this.locked())return;const value=JSON.parse(id);if(value.profile!==files.profile.id||value.revision!==files.profile.revision||value.generation!==files.revision)return;
    const scope=this.fileScope(files),path=value.path,paths=files.selected.includes(path)?[...files.selected]:path?[path]:[],directory=files.entry(path)?.kind==='directory'?path:path?parent(path):files.path,action=value.action;
    if(action==='refresh')return files.load(files.path,cx);if(action==='copy_path')return writeClipboard(paths.join('\n'));
    if(['copy','cut'].includes(action)){files.clipboard={paths:[...paths],cut:action==='cut'};return;}
    if(action==='paste'&&files.clipboard){const clipboard=files.clipboard,completed=await this.mutate(scope,clipboard.paths.map(path=>({path,action:{operation:clipboard.cut?'move':'copy',destination:join(directory,name(path))}})),cx);if(clipboard.cut&&completed)files.clipboard=null;return;}
    if(action==='upload'){const ids=await selectUpload(scope.profile,directory,this.text.files_upload);if(this.fileOwner(scope))for(const id of ids)transferAction(id,'start');return;}
    if(action==='download')return this.download(paths,cx,scope);
    if(['rename','new_file','new_directory'].includes(action)){this.fileEdit={scope,action,path:paths[0],directory,input:createText(action==='rename'?name(paths[0]):'',{label:this.text.files_entry_name,placeholder:this.text.form_filename_hint})};this.dialog++;cx.notify();return;}
    if(action==='delete'){this.confirmation={label:'settings_delete',text:this.text.ssh_files_delete_confirm+'\n'+paths.join('\n'),run:cx=>this.mutate(scope,paths.map(path=>({path,action:{operation:'remove'}})),cx)};this.dialog++;cx.notify();}
  }
  async mutate(scope,actions,cx){
    if(this.locked()||!this.fileCurrent(scope))return false;const{files,profile}=scope;
    const batch=new Mutations(actions.map(action=>command('modify_ssh_file',{profile:profile.id,expected_revision:profile.revision,...action})),()=>this.fileCurrent(scope));
    files.mutation=batch;let current=false;
    batch.settled=async cx=>{current=this.fileCurrent(scope);if(current)await files.load(files.path,cx);};
    const output=await this.execute(null,cx,null,null,profile,batch);
    return current&&output?.data?.kind==='files_changed';
  }
  submitFile(cx){const value=this.fileEdit;if(!value||this.locked()||!this.fileCurrent(value.scope))return;const text=readText(value.input);if(!text||/[\/\\\0]/.test(text)||['.','..'].includes(text))return;
    const action=value.action==='rename'?{path:value.path,action:{operation:'move',destination:join(parent(value.path),text)}}:{path:join(value.directory,text),action:{operation:'create',directory:value.action==='new_directory'}};this.close(cx);this.run(cx=>this.mutate(value.scope,[action],cx),cx);}
  async download(paths,cx,scope=this.fileScope()){if(!this.fileCurrent(scope))return;const entries=paths.map(path=>({path,directory:scope.files.entry(path)?.kind==='directory'}));const ids=await selectDownload(scope.profile,entries,this.text.files_download);if(this.fileOwner(scope))for(const id of ids){try{transferAction(id,'start');}catch(_){/* Existing destinations await explicit replace. */}}cx.notify();}
  transferAction(job,action,cx){transferAction(job.id,action);cx.notify();}
  render(){return render(this);}
}

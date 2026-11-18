import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFile} from 'node:fs/promises';

async function fixture() {
  const context=vm.createContext({});
  const opened=[],notices=[],focused=[],released=[],focusedTexts=[],dropped=[],copied=[],previewed=[];
  const profile={id:'saved-host',revision:3,name:'Saved host'};
  const terminal={id:'existing-terminal',ssh:profile.id,status:{kind:'running'}};
  const host={profiles:[profile],terminals:[terminal],catalog:{projects:[],worktrees:[]},catalogReads:0,inspect:async()=>({Ok:{kind:'terminal_snapshot',data:{info:terminal}}}),browse:async()=>({kind:'directory',path:'.',entries:[],next:null}),upload:async()=>[],download:async()=>[]};
  host.listProfiles=async()=>host.profiles;
  host.listTerminals=async id=>host.terminals.filter(value=>value.ssh===id);
  host.readCatalog=async()=>host.catalog;
  const requests=new Map(),reads=[],forgotten=[],started=[],texts=[];
  const exports={
    'gpui-kit':{View:class{}},
    sailry:{context:()=>JSON.stringify({locale:'en'}),header_action(){},focusTerminal:id=>focused.push(id),releaseTerminal:id=>released.push(id)},
    'sailry/sdk':{nextChange(){},readProjectCatalog(){host.catalogReads++;return host.readCatalog();},prepareRequest(value){const id=`request-${reads.length}`;reads.push(value);requests.set(id,value);return id;},completeRequest:id=>host.inspect(requests.get(id)),forgetRequest:id=>forgotten.push(id),faultCode(message){try{return JSON.parse(message).code;}catch(_){return 'failed';}}},
    'sailry/connections':{listSsh:()=>host.listProfiles(),newSshId(){},prepareSsh(){},listSshTerminals:id=>host.listTerminals(id),prepareSshTerminal(){},browseSshDirectory:async input=>{opened.push(input);return host.browse(input);},requestOutcome:id=>host.outcome(id)},
    'sailry/forms':{createText:value=>value,readText:value=>value,releaseText:value=>texts.push(['release',value]),setText:(field,value)=>texts.push(['set',field,value]),focusText:id=>focusedTexts.push(id),nextTextEvent(){}},
    'sailry/credentials':{createSecret(){},releaseSecret(){},describeSecret(){},chooseSecretFile(){}},
    'sailry/ui':{nextTreeEvent(){},nextContextMenuEvent(){},nextNavigationTabEvent(){},nextPickerEvent(){},nextControlEvent(){},modal_closed(){},writeClipboard:value=>copied.push(value),toast:value=>notices.push(plain(value))},
    'sailry/ssh-transfers':{uploadDropped:(...args)=>{dropped.push(args);return ['dropped-job'];},selectUpload:(...args)=>host.upload(...args),selectDownload:(...args)=>host.download(...args),nextTransfers(){},transferAction:(...args)=>started.push(args),previewFile:(...args)=>previewed.push(args)},
    './editor.js':{draft(){},prepare(){}},
    './locales.js':{messages:()=>({}),format(){}},
    './view.js':{render(){}},
  };
  const cache=new Map();
  const resolve=async name=>{
    if(cache.has(name))return cache.get(name);
    const local=['./files.js','./requests.js','./mutations.js'].includes(name);
    const module=local
      ?new vm.SourceTextModule(await readFile(new URL(`../dev.sailry.platform/desktop/${name.slice(2)}`,import.meta.url),'utf8'),{context})
      :new vm.SyntheticModule(Object.keys(exports[name]),function(){for(const [key,value] of Object.entries(exports[name]))this.setExport(key,value);},{context});
    cache.set(name,module);if(local)await module.link(resolve);return module;
  };
  await resolve('./mutations.js');
  const module=new vm.SourceTextModule(await readFile(new URL('../dev.sailry.platform/desktop/main.js',import.meta.url),'utf8'),{context});
  await module.link(resolve);await module.evaluate();
  const view=new module.namespace.default();const pending=[];
  view.run=(action,cx)=>pending.push(Promise.resolve().then(()=>action(cx)));
  Object.assign(view,{profiles:[],terminals:[],terminal:null,terminalSelection:0,terminalClose:null,closingTerminals:new Set(),selected:null,browsers:new Map(),pending:null,connected:true,loaded:false,refreshRevision:0,text:{ssh_files_delete_confirm:'Delete files',files_upload:'Upload',files_download:'Download'},dialog:0});
  return {view,profile,terminal,host,opened,reads,forgotten,started,texts,notices,focused,released,focusedTexts,dropped,copied,previewed,cx:{notify(){},stop_propagation(){}},settle:()=>Promise.all(pending)};
}

async function browsing() {const setup=await fixture();await setup.view.refresh(setup.cx);await setup.settle();return setup;}
const plain=value=>JSON.parse(JSON.stringify(value));
function deferred(){let resolve,reject;const promise=new Promise((accept,fail)=>{resolve=accept;reject=fail;});return {promise,resolve,reject};}

test('initial inventory stays unconfirmed until all reads finish',async()=>{
  const setup=await fixture(),profiles=deferred(),catalog=deferred();let changes,terminals=0;
  setup.host.listProfiles=()=>profiles.promise;setup.host.readCatalog=()=>catalog.promise;
  setup.host.listTerminals=async()=>{terminals++;return [setup.terminal];};
  setup.view.watch=(_read,handle,_cx,cursor)=>{if(cursor&&!changes)changes=handle;};setup.view.init({},setup.cx);
  changes({connected:true,entry:'0'},setup.cx);
  assert.equal(setup.view.loaded,false);assert.equal(setup.view.terminals.length,0);
  await Promise.resolve();assert.equal(setup.host.catalogReads,1,'project choices start before profiles settle');
  profiles.resolve([setup.profile]);
  for(let step=0;step<20&&!terminals;step++)await Promise.resolve();
  assert.equal(terminals,1,'terminals do not wait for independent project choices');
  assert.equal(setup.host.catalogReads,1);assert.equal(setup.view.loaded,false);
  assert.equal(setup.view.terminals.length,0);
  catalog.resolve(setup.host.catalog);await setup.settle();
  assert.equal(setup.view.loaded,true);assert.equal(setup.view.terminal,setup.terminal.id);
  assert.equal(setup.view.profiles[0],setup.profile);
});

test('subscription retains initial, scope, and connection changes',async()=>{
  const {view,cx,host,profile,settle}=await fixture(),first=deferred(),latest=deferred();let changes,reads=0;
  host.listProfiles=()=>++reads===1?first.promise:latest.promise;
  view.watch=(_read,handle,_cx,cursor)=>{if(cursor&&!changes)changes=handle;};
  view.init({},cx);await settle();assert.equal(reads,0);
  changes({connected:false,entry:'0'},cx);await settle();assert.equal(reads,0);
  changes({connected:true,entry:'0'},cx);await Promise.resolve();assert.equal(reads,1);
  view.selected=profile.id;changes({connected:true,entry:'1'},cx);await Promise.resolve();
  first.resolve([]);
  for(let step=0;step<30&&reads<2;step++)await Promise.resolve();
  assert.equal(reads,2);assert.equal(view.loaded,false,'superseded inventory is not published');
  latest.resolve([profile]);await settle();assert.equal(view.loaded,true);assert.equal(view.selected,profile.id);
  changes({connected:true,entry:'1'},cx);await settle();assert.equal(reads,3,'the first real host update remains observable');
  changes({connected:false,entry:'1'},cx);await settle();assert.equal(reads,3);
  changes({connected:true,entry:'1'},cx);await settle();assert.equal(reads,4,'reconnection refreshes without a new entry');
});

test('initial failures stay unconfirmed and later reads recover',async()=>{
  const {view,cx,host}=await fixture();
  host.listProfiles=async()=>{throw Error('inventory unavailable');};
  await assert.rejects(view.refresh(cx),/inventory unavailable/);
  assert.equal(view.loaded,false);assert.equal(view.profiles.length,0);
  host.listProfiles=async()=>[];await view.refresh(cx);
  assert.equal(view.loaded,true);assert.equal(view.terminals.length,0);
});

test('refresh failures retain confirmed terminals and open drafts',async()=>{
  const {view,cx,host,terminal}=await fixture();await view.refresh(cx);
  const profiles=view.profiles,terminals=view.terminals,editor={inputs:['Retained draft']};
  view.editor=editor;host.listProfiles=async()=>{throw Error('inventory unavailable');};
  await assert.rejects(view.refresh(cx),/inventory unavailable/);
  assert.equal(view.loaded,true);assert.equal(view.terminal,terminal.id);
  assert.equal(view.profiles,profiles);assert.equal(view.terminals,terminals);assert.equal(view.editor,editor);
});

test('coalesces overlapping refreshes and awaits the latest inventory',async()=>{
  const {view,cx,host}=await fixture(),first=deferred(),latest=deferred();let reads=0;
  host.listProfiles=()=>++reads===1?first.promise:latest.promise;
  const initial=view.refresh(cx),queued=Array.from({length:8},()=>view.refresh(cx));
  assert.equal(reads,1,'entry and subscription changes do not overlap reads');
  first.resolve([]);
  for(let step=0;step<20&&reads<2;step++)await Promise.resolve();
  assert.equal(reads,2,'one follow-up absorbs all queued changes');
  latest.resolve([]);await Promise.all([initial,...queued]);
  assert.equal(reads,2);assert.equal(view.refreshing,null);
  assert.deepEqual(Array.from(view.profiles),[]);
});

test('retained entry refreshes newly registered project choices',async()=>{
  const {view,cx,host,settle}=await fixture();let changes;
  view.watch=(_read,handle,_cx,cursor)=>{if(cursor&&!changes)changes=handle;};
  view.init({},cx);changes({connected:true,entry:'1'},cx);await settle();assert.equal(host.catalogReads,1);
  host.catalog={projects:[{id:'new-project',name:'New'}],worktrees:[{id:'new-tree',project:'new-project',path:'/new'}]};
  changes({connected:true,entry:'2'},cx);await settle();view.transferDialog(cx);
  assert.equal(host.catalogReads,2);assert.deepEqual(plain(view.projects),host.catalog.projects);
  assert.deepEqual(plain(view.worktrees),host.catalog.worktrees);assert.equal(view.transfer.worktree,'new-tree');
});

test('catalog changes remove unavailable transfer selections without losing paths',async()=>{
  const {view,cx,host}=await fixture();
  host.catalog={projects:[{id:'project',name:'Original'}],worktrees:[{id:'tree',project:'project',path:'/original'}]};
  await view.refresh(cx);view.transferDialog(cx);const transfer=view.transfer;
  transfer.inputs=['source.txt','/remote/destination.txt'];
  host.catalog={projects:[{id:'project',name:'Renamed'}],worktrees:[{id:'other-tree',project:'project',path:'/other'}]};
  await view.refresh(cx);
  assert.equal(view.transfer,transfer);assert.equal(transfer.worktree,null);
  assert.deepEqual(transfer.inputs,['source.txt','/remote/destination.txt']);
  assert.deepEqual(plain(view.projects),host.catalog.projects);assert.deepEqual(plain(view.worktrees),host.catalog.worktrees);
  view.control({id:`ssh-worktree-${view.dialog}`,value:'other-tree'},cx);assert.equal(transfer.worktree,'other-tree');
});

test('refresh failures remain visible and allow a later read',async()=>{
  const {view,cx,host}=await fixture(),first=deferred();let reads=0;
  host.listProfiles=()=>++reads===1?first.promise:Promise.resolve([]);
  const initial=view.refresh(cx),queued=view.refresh(cx);
  const results=Promise.allSettled([initial,queued]);
  first.reject(new Error('inventory unavailable'));
  for(const result of await results){assert.equal(result.status,'rejected');assert.match(result.reason.message,/inventory unavailable/);}
  assert.equal(view.refreshing,null);
  await view.refresh(cx);assert.equal(reads,2);
});

test('restores every profile terminal list concurrently',async()=>{
  const {view,cx,host,terminal,settle}=await fixture(),held=[];let reads=0;
  host.profiles=Array.from({length:6},(_,index)=>({id:`host-${index}`,revision:1,name:`Host ${index}`}));
  host.terminals=host.profiles.map(profile=>({...terminal,id:`terminal-${profile.id}`,ssh:profile.id}));
  view.terminals=[...host.terminals];
  host.listTerminals=id=>{reads++;const wait=deferred();held.push({id,...wait});return wait.promise;};
  const restoring=view.refresh(cx);
  for(let step=0;step<20&&reads<6;step++)await Promise.resolve();
  assert.equal(reads,6,'all independent profiles start before any terminal list settles');
  assert.equal(view.loaded,false);
  for(const wait of held)wait.resolve(host.terminals.filter(info=>info.ssh===wait.id));
  await restoring;await settle();
  assert.equal(view.terminals.length,6);
});

test('inspects missing terminal snapshots concurrently',async()=>{
  const {view,cx,host,terminal}=await fixture(),held=[];
  view.terminals=Array.from({length:6},(_,index)=>({...terminal,id:`terminal-${index}`}));
  host.listTerminals=async()=>[];
  host.inspect=command=>{const wait=deferred();held.push({id:command.data.terminal,...wait});return wait.promise;};
  const restoring=view.refresh(cx);
  for(let step=0;step<30&&held.length<6;step++)await Promise.resolve();
  assert.equal(held.length,6,'missing snapshots start without waiting for one another');
  for(const wait of held)wait.resolve({Ok:{kind:'terminal_snapshot',data:{info:view.terminals.find(info=>info.id===wait.id)}}});
  await restoring;assert.equal(view.terminals.length,6);
});

test('transfer outcomes use one toast and cancellation remains neutral',async()=>{
  const {view,cx,notices}=await fixture();view.jobs=[];
  Object.assign(view.text,{ssh_cancelled:'Cancelled',ssh_transfer_done:'Transferred',ssh_files_uncertain:'Result unconfirmed',ssh_files_action_failed:'File action failed'});
  view.transfers({transfers:[{id:'prior',stage:'done'}]},cx);assert.equal(notices.length,0);
  view.transfers({transfers:[{id:'prior',stage:'done'},{id:'job',stage:'transferring'}]},cx);
  for(const [stage,kind,message]of[['uncertain','error','Result unconfirmed'],['done','info','Transferred'],['cancelled','info','Cancelled']]){
    const value={transfers:[{id:'job',stage}]};view.transfers(value,cx);view.transfers(value,cx);
    assert.deepEqual(notices.at(-1),{id:'ssh-transfer-job',kind,message});
  }
  assert.equal(notices.length,3);assert.equal(view.jobs[0].stage,'cancelled');
});

test('restores existing terminal and its file browser without reopening a terminal',async()=>{
  const setup=await fixture();await setup.view.refresh(setup.cx);await setup.settle();
  assert.equal(setup.view.terminal,setup.terminal.id);
  assert.equal(setup.view.selected,setup.profile.id);
  assert.equal(setup.view.files().profile,setup.profile);
  assert.deepEqual(setup.opened.map(value=>[value.profile,value.expected_revision,value.path]),[[setup.profile.id,3,'.']]);
  await setup.view.refresh(setup.cx);await setup.settle();assert.equal(setup.opened.length,1);
});

test('dragging selected files moves the captured selection and preserves clipboard',async()=>{
  const {view,cx}=await fixture();const calls=[];
  const clipboard={paths:['/remote/copied'],cut:false};
  const files={placeholder:()=>null,path:'/remote',selected:['/remote/a','/remote/b'],clipboard,entry:path=>path==='/remote/target'?{kind:'directory'}:null};
  view.files=()=>files;view.mutate=async (_scope,actions)=>calls.push(actions);
  await view.tree({tree:'ssh-files',kind:'drop',source:'/remote/a',id:'/remote/target'},cx);
  assert.deepEqual(JSON.parse(JSON.stringify(calls)),[[
    {path:'/remote/a',action:{operation:'move',destination:'/remote/target/a'}},
    {path:'/remote/b',action:{operation:'move',destination:'/remote/target/b'}},
  ]]);
  assert.equal(files.clipboard,clipboard);
  files.selected=['/remote/a','/remote/target'];
  await view.tree({tree:'ssh-files',kind:'drop',source:'/remote/a',id:'/remote/target'},cx);
  assert.equal(calls.length,1,'a selection containing the destination is refused as a unit');
});

test('native additive and range modifiers retain the selected remote paths',async()=>{
  const setup=await fixture();await setup.view.refresh(setup.cx);await setup.settle();
  const files=setup.view.files();files.path='/remote';files.entries=['a','b','c'].map(name=>({name,kind:'file'}));
  files.select('/remote/a',{});files.select('/remote/c',{additive:true});
  assert.deepEqual(Array.from(files.selected),['/remote/a','/remote/c']);
  files.select('/remote/b',{shift:true});assert.deepEqual(Array.from(files.selected),['/remote/b','/remote/c']);
  files.select('/remote/c',{additive:true});assert.deepEqual(Array.from(files.selected),['/remote/b']);
});

test('delete confirmation refuses a changed browser or captured revision',async()=>{
  for(const change of ['browser','directory','profile','closed']){
    const {view,cx,profile,settle}=await browsing(),files=view.files(),commands=[];
    files.selected=['/remote/a'];view.execute=async value=>commands.push(value);
    await view.fileAction(view.fileId('delete','/remote/a'),cx);
    if(change==='browser')view.files=()=>({...files});
    if(change==='directory')files.revision++;
    if(change==='profile')view.profiles=[{...profile,revision:profile.revision+1}];
    if(change==='closed')files.close();
    view.confirm(cx);await settle();
    assert.equal(commands.length,0,change);
  }
});

test('delete confirmation retains original profile and selected paths',async()=>{
  const {view,cx,host,profile,reads,settle}=await browsing(),files=view.files();
  files.selected=['/remote/a','/remote/b'];
  host.inspect=async()=>({Ok:{kind:'ssh_outcome',data:{kind:'files_changed'}}});
  await view.fileAction(view.fileId('delete','/remote/a'),cx);
  files.selected.push('/remote/later');view.confirm(cx);
  await settle();
  assert.deepEqual(plain(reads),['/remote/a','/remote/b'].map(path=>({kind:'modify_ssh_file',data:{profile:profile.id,expected_revision:profile.revision,path,action:{operation:'remove'}}})));
  assert.equal(files.mutation.requests.length,2);assert.equal(files.mutation.done,true);
});

test('independent mutations retain captured scope after admission',async()=>{
  for(const change of ['browser','directory','profile']){
    const {view,cx,host,profile,opened,reads}=await browsing(),files=view.files(),held=[];
    host.inspect=()=>{const wait=deferred();held.push(wait);return wait.promise;};
    const result=view.mutate(view.fileScope(),['/remote/a','/remote/b'].map(path=>({path,action:{operation:'remove'}})),cx);
    assert.equal(held.length,2,'all independent writes start before either settles');
    if(change==='browser')view.files=()=>({...files});
    if(change==='directory')files.revision++;
    if(change==='profile')view.profiles=[{...profile,revision:profile.revision+1}];
    for(const wait of held)wait.resolve({Ok:{kind:'ssh_outcome',data:{kind:'files_changed'}}});
    assert.equal(await result,false,change);
    assert.equal(reads.length,2,change);assert.equal(files.mutation.done,true);
    assert.equal(opened.length,1,'a stale browser is not reloaded');
    for(const value of reads){assert.equal(value.data.profile,profile.id);assert.equal(value.data.expected_revision,profile.revision);}
  }
});

test('partial results remain inspectable without replay',async()=>{
  const {view,cx,host,reads,forgotten,opened}=await browsing(),files=view.files(),held=[];
  host.inspect=()=>{const wait=deferred();held.push(wait);return wait.promise;};
  const loading=view.mutate(view.fileScope(),['/remote/a','/remote/b','/remote/c'].map(path=>({path,action:{operation:'remove'}})),cx);
  assert.equal(held.length,3);
  held[0].resolve({Ok:{kind:'ssh_outcome',data:{kind:'files_changed'}}});
  held[1].resolve({Err:{code:'permission_denied',message:'Remote path is read-only'}});
  held[2].reject(Error('transport lost after admission'));
  assert.equal(await loading,false);const batch=files.mutation;
  assert.equal(view.pending.request,batch);assert.equal(batch.requests[0].output.data.kind,'files_changed');
  assert.equal(batch.requests[1].error.code,'permission_denied');assert.equal(batch.requests[2].unknown,true);
  const inspections=[];host.outcome=async id=>{inspections.push(id);return {kind:'completed',data:{Ok:{kind:'ssh_outcome',data:{kind:'files_changed'}}}};};
  await view.inspect(cx);
  assert.deepEqual(inspections,[batch.requests[2].id]);assert.equal(reads.length,3,'inspection does not repeat writes');
  assert.equal(view.pending,null);assert.equal(view.error.code,'permission_denied');
  assert.equal(files.mutation,batch);assert.equal(batch.requests[0].output.data.kind,'files_changed');
  assert.equal(batch.requests[2].output.data.kind,'files_changed');assert.equal(forgotten.length,3);
  assert.equal(opened.length,2,'confirmed partial changes refresh the current browser');
});

test('queued file edits refuse a browser switch before dispatch',async()=>{
  const {view,cx,settle}=await browsing(),files=view.files(),commands=[];
  await view.fileAction(view.fileId('new_file',''),cx);view.fileEdit.input='created.txt';
  view.execute=async value=>commands.push(value);view.submitFile(cx);
  view.files=()=>({...files});await settle();assert.equal(commands.length,0);
});

test('native transfer pickers do not start jobs after scope changes',async()=>{
  for(const action of ['upload','download']){
    for(const change of ['browser','profile','closed']){
    const {view,cx,host,profile,started}=await browsing(),wait=deferred(),files=view.files();
    host[action]=async target=>{assert.equal(target,profile);return wait.promise;};
    const pending=view.fileAction(view.fileId(action,'/remote/a'),cx);
    if(change==='browser')view.files=()=>({...files});
    if(change==='profile')view.profiles=[{...profile,revision:profile.revision+1}];
    if(change==='closed')files.close();
    wait.resolve(['prepared-job']);await pending;
    assert.equal(started.length,0,`${action}: ${change}`);
    }
  }
});

test('native transfer pickers retain captured targets across listing refreshes',async()=>{
  for(const action of ['upload','download']){
    const {view,cx,host,profile,started}=await browsing(),wait=deferred(),files=view.files(),chosen=[];
    files.path='/remote';files.entries=[{name:'a',kind:'file'}];
    host[action]=async(target,paths)=>{chosen.push({target,paths});return wait.promise;};
    const pending=view.fileAction(view.fileId(action,'/remote/a'),cx);
    await files.load('/elsewhere',cx);
    wait.resolve(['prepared-job']);await pending;
    assert.deepEqual(plain(chosen),[{target:profile,paths:action==='upload'?'/remote':[{path:'/remote/a',directory:false}]}]);
    assert.deepEqual(started,[['prepared-job','start']],action);
  }
});

test('retains a captured running terminal after its profile is removed',async()=>{
  const {view,cx,host,terminal,reads,forgotten}=await browsing(),files=view.files();
  host.profiles=[];host.terminals=[];await view.refresh(cx);
  assert.equal(view.terminal,terminal.id);assert.equal(view.terminals[0],terminal);
  assert.equal(view.files(),null);assert.equal(files.closed,true);
  assert.deepEqual(plain(reads),[{kind:'inspect_terminal',data:{terminal:terminal.id}}]);
  assert.equal(forgotten.length,1);
});

test('exited captured terminals close once without discovering unowned terminals',async()=>{
  for(const [status,toasted] of [[{kind:'exited',data:{code:0}},false],[{kind:'exited',data:{code:7}},true],[{kind:'failed',data:{message:'signal'}},true]]){
    const {view,cx,host,terminal,reads,released,notices,settle}=await browsing();
    let current={...terminal,owner:null,status};
    host.terminals=[];host.inspect=async value=>{
      if(value.kind!=='close_ssh_terminal')return {Ok:{kind:'terminal_snapshot',data:{info:current}}};
      current={...current,status:{kind:'closed'}};return {Ok:{kind:'terminal',data:current}};
    };
    await view.refresh(cx);await settle();await view.refresh(cx);await settle();
    assert.deepEqual(reads.map(value=>value.kind),['inspect_terminal','close_ssh_terminal','inspect_terminal']);
    assert.equal(view.terminal,null);
    assert.deepEqual(released,[terminal.id]);
    assert.deepEqual(notices.map(notice=>notice.id),toasted?[`ssh-exit-${terminal.id}`]:[]);
  }
  const other=await fixture();other.host.terminals=[];
  await other.view.refresh(other.cx);assert.equal(other.view.terminals.length,0);assert.equal(other.reads.length,0);
});

test('keeps captured terminals on transient read failure until closed or not found',async()=>{
  for(const [result,retained] of [[{Err:{code:'unavailable'}},true],[new Error('disconnected'),true],[new Error(JSON.stringify({code:'unavailable',message:'disconnected'})),true],[{Ok:{kind:'terminal_snapshot',data:{info:{id:'existing-terminal',ssh:'saved-host',status:{kind:'closed'}}}}},false],[{Err:{code:'not_found'}},false],[new Error(JSON.stringify({code:'not_found',message:'terminal closed'})),false]]){
    const {view,cx,host,terminal,forgotten}=await browsing();host.terminals=[];
    host.inspect=async()=>{if(result instanceof Error)throw result;return result;};
    await view.refresh(cx);
    assert.equal(view.terminal,retained?terminal.id:null);
    assert.equal(forgotten.length,1);
  }
});

test('opening a running connection preserves captured exited tabs',async()=>{
  const {view,cx,host,terminal,profile}=await browsing(),exited={...terminal,status:{kind:'exited',code:0}},running={...terminal,id:'second-terminal'};
  view.terminals=[exited];host.terminals=[running];await view.open(profile,cx);
  assert.deepEqual(Array.from(view.terminals,info=>info.id),[exited.id,running.id]);
});

test('failed openings return home and allow reconnection',async()=>{
  const {view,cx,host,profile,terminal,reads,notices,focused}=await browsing();
  const other={...profile,id:'other-host',name:'Other host'};
  host.profiles=[profile,other];view.profiles=[profile,other];
  let attempts=0;
  host.inspect=async value=>{
    assert.equal(value.kind,'open_ssh_terminal');
    if(++attempts===1)return {Err:{code:'unavailable',message:'SSH connection could not be established'}};
    const opened={...terminal,id:'new-terminal',ssh:other.id};host.terminals.push(opened);
    return {Ok:{kind:'ssh_outcome',data:{kind:'terminal',...opened}}};
  };
  await view.open(other,cx);
  assert.equal(view.home,true);assert.equal(view.pending,null);assert.equal(view.locked(),false);
  assert.equal(view.terminals[0].id,terminal.id,'a failed open keeps existing terminal tabs');
  assert.equal(notices.length,1);assert.equal(attempts,1);
  await view.open(other,cx);
  assert.equal(view.home,false);assert.equal(view.terminal,'new-terminal');assert.equal(view.pending,null);
  assert.deepEqual(focused,['new-terminal']);
  assert.equal(reads.filter(value=>value.kind==='open_ssh_terminal').length,2);
});

test('unconfirmed openings inspect the original request before reconnecting',async()=>{
  const {view,cx,host,profile,reads,notices,settle}=await browsing();
  host.terminals=[];view.terminals=[];view.terminal=null;
  host.inspect=async()=>({Err:{code:'outcome_unknown',message:'SSH opening could not be confirmed'}});
  await view.open(profile,cx);
  const pending=view.pending,id=pending.request.id;
  assert.equal(view.home,true);assert.equal(view.connectionLocked(profile),false);
  assert.equal(view.connectionLocked({...profile,id:'another-host'}),true);
  assert.equal(view.locked(),true);
  const inspected=[];
  host.outcome=async value=>{inspected.push(value);return {kind:'admitted'};};
  view.control({id:`ssh-open-${profile.id}`},cx);await settle();
  assert.deepEqual(inspected,[id]);assert.equal(view.pending,pending);
  assert.equal(reads.filter(value=>value.kind==='open_ssh_terminal').length,1);
  assert.equal(notices.length,2,'each explicit check reports its still-unconfirmed outcome once');
  host.outcome=async value=>{inspected.push(value);return {kind:'completed',data:{Err:{code:'unavailable',message:'SSH connection could not be established'}}};};
  await view.open(profile,cx);
  assert.deepEqual(inspected,[id,id]);assert.equal(view.pending,null);assert.equal(view.locked(),false);
  assert.equal(reads.filter(value=>value.kind==='open_ssh_terminal').length,1,'checking never sends another opening');
});

test('closing a browser suppresses its late directory result',async()=>{
  const {view,cx,host,texts}=await browsing(),files=view.files(),wait=deferred();
  texts.length=0;host.browse=async()=>wait.promise;const pending=files.load('/late',cx);files.close();
  wait.resolve({kind:'directory',path:'/late',entries:[],next:null});await pending;
  assert.deepEqual(texts,[['release',files.input]]);assert.equal(files.path,'.');
});

test('active terminal closes focus each captured successor without replay',async()=>{
  const {view,cx,host,terminal,focused}=await browsing();
  const second={...terminal,id:'second-terminal'},third={...terminal,id:'third-terminal'};
  host.terminals=[terminal,second,third];await view.refresh(cx);
  assert.equal(focused.length,0,'background observations never take focus');
  view.selectTerminal(second.id,cx,true);assert.deepEqual(focused,[second.id]);
  const closed=[];
  view.execute=async value=>{
    closed.push(value.data.terminal);host.terminals=host.terminals.filter(info=>info.id!==value.data.terminal);
    view.terminals=view.terminals.map(info=>info.id===value.data.terminal?{...info,status:{kind:'closed'}}:info);
    host.inspect=async value=>({Ok:{kind:'terminal_snapshot',data:{info:view.terminals.find(info=>info.id===value.data.terminal)}}});
    await view.refresh(cx);return {kind:'terminal'};
  };
  await view.closeTerminal(second.id,cx);await view.closeTerminal(third.id,cx);await view.closeTerminal(terminal.id,cx);
  assert.deepEqual(focused,[second.id,third.id,terminal.id]);
  assert.deepEqual(closed,[second.id,third.id,terminal.id]);assert.equal(view.terminal,null);
});

test('background, rejected and superseded terminal closes do not steal focus',async()=>{
  const {view,cx,host,terminal,focused}=await browsing(),second={...terminal,id:'second-terminal'};
  host.terminals=[terminal,second];await view.refresh(cx);
  view.execute=async()=>null;await view.closeTerminal(terminal.id,cx);assert.equal(focused.length,0);
  const pending=deferred();view.execute=()=>pending.promise;
  const closing=view.closeTerminal(terminal.id,cx);view.selectTerminal(second.id,cx,true);
  pending.resolve({kind:'terminal'});await closing;assert.deepEqual(focused,[second.id]);
  view.execute=async()=>({kind:'terminal'});await view.closeTerminal(terminal.id,cx);
  assert.deepEqual(focused,[second.id]);
  view.pending={};view.control({id:'ssh-close-current'},cx);assert.deepEqual(focused,[second.id]);
});

test('accepted closes focus after the newest overlapping terminal observation',async()=>{
  for(const superseded of [false,true]){
  const {view,cx,host,terminal,profile,focused}=await browsing();
  const second={...terminal,id:'second-terminal'},third={...terminal,id:'third-terminal'};
  host.terminals=[terminal,second,third];await view.refresh(cx);
  view.selectTerminal(second.id,cx,true);focused.length=0;
  host.terminals=[terminal,third];
  host.inspect=async()=>({Ok:{kind:'terminal_snapshot',data:{info:{...second,status:{kind:'closed'}}}}});
  const older=deferred(),newer=deferred();let reads=0;
  Object.defineProperty(host,'profiles',{get:()=>++reads===1?older.promise:newer.promise});
  // Use the real completion reducer; only durable admission is isolated here.
  view.execute=async()=>{
    view.pending={request:{done:true,release(){},output:{kind:'terminal'}}};
    return view.finish(cx);
  };
  const closing=view.closeTerminal(second.id,cx);
  const observation=view.refresh(cx);
  older.resolve([profile]);
  for(let step=0;step<20&&reads<2;step++)await Promise.resolve();
  assert.equal(reads,2,'the latest observation follows the older read without overlapping it');
  assert.deepEqual(focused,[],'the closed content is not focused before its successor is known');
  if(superseded)view.selectTerminal(terminal.id,cx,true);
  newer.resolve([profile]);await Promise.all([closing,observation]);
  assert.equal(view.terminal,superseded?terminal.id:third.id);
  assert.deepEqual(focused,[superseded?terminal.id:third.id]);
  assert.equal(view.terminalClose,null);
  assert.equal(reads,2,'only the two original reads run; the mutation is not replayed');
  }
});

test('the close shortcut captures its terminal before dispatch',async()=>{
  const {view,cx,host,terminal,settle}=await browsing();
  const second={...terminal,id:'second-terminal'},closed=[];
  host.terminals=[terminal,second];await view.refresh(cx);
  view.closeTerminal=async id=>closed.push(id);
  view.control({id:'ssh-close-current'},cx);
  view.selectTerminal(second.id,cx,true);
  await settle();
  assert.deepEqual(closed,[terminal.id]);
  assert.equal(view.terminal,second.id);
});

test('terminal release precedes close and lasts through settlement',async()=>{
  const {view,cx,host,terminal,released,reads}=await browsing(),reply=deferred();
  host.inspect=value=>{
    if(value.kind==='close_ssh_terminal'){assert.deepEqual(released,[terminal.id]);return reply.promise;}
    return Promise.resolve({Ok:{kind:'terminal_snapshot',data:{info:{...terminal,status:{kind:'closed'}}}}});
  };
  const closing=view.closeTerminal(terminal.id,cx);
  assert.deepEqual(released,[terminal.id]);assert.equal(view.closingTerminals.has(terminal.id),true);
  assert.equal(view.pending.request.command.kind,'close_ssh_terminal');
  host.terminals=[];reply.resolve({Ok:{kind:'terminal',data:{...terminal,status:{kind:'closed'}}}});
  await closing;
  assert.equal(view.terminal,null);assert.equal(view.closingTerminals.size,0);
  assert.equal(reads.filter(value=>value.kind==='close_ssh_terminal').length,1);
});

test('confirmed close failure restores the captured terminal without replay',async()=>{
  const {view,cx,host,terminal,released,reads}=await browsing();
  host.inspect=async()=>({Err:{code:'unavailable',message:'Close failed'}});
  await view.closeTerminal(terminal.id,cx);
  assert.deepEqual(released,[terminal.id]);assert.equal(view.closingTerminals.size,0);
  assert.equal(view.terminal,terminal.id);assert.equal(view.terminals[0],terminal);
  assert.equal(view.error.code,'unavailable');assert.equal(view.pending,null);
  assert.equal(reads.filter(value=>value.kind==='close_ssh_terminal').length,1);
});

test('uncertain closes retain release until the original outcome settles',async()=>{
  for(const failed of [false,true]){
    const {view,cx,host,terminal,released,reads}=await browsing();
    host.inspect=async()=>({Err:{code:'outcome_unknown',message:'Close outcome unavailable'}});
    await view.closeTerminal(terminal.id,cx);
    const id=view.pending.request.id;
    assert.equal(view.pending.request.unknown,true);assert.equal(view.closingTerminals.has(terminal.id),true);
    host.outcome=async observed=>{assert.equal(observed,id);return {kind:'completed',data:failed
      ?{Err:{code:'unavailable',message:'Close failed'}}
      :{Ok:{kind:'terminal',data:{...terminal,status:{kind:'closed'}}}}};};
    if(!failed){host.terminals=[];host.inspect=async()=>({Err:{code:'not_found'}});}
    await view.inspect(cx);
    assert.equal(view.pending,null);assert.equal(view.closingTerminals.size,0);
    assert.equal(view.terminal,failed?terminal.id:null);assert.deepEqual(released,[terminal.id]);
    assert.equal(reads.filter(value=>value.kind==='close_ssh_terminal').length,1);
  }
});

test('overlapping settled closes retain each released terminal',async()=>{
  const {view,cx,host,profile,terminal,released,reads}=await browsing(),profiles=deferred();
  const second={...terminal,id:'second-terminal'};host.terminals=[terminal,second];await view.refresh(cx);
  let requested=false;host.listProfiles=()=>{requested=true;return profiles.promise;};
  host.inspect=async value=>{
    const info={...terminal,id:value.data.terminal,status:{kind:'closed'}};
    if(value.kind==='close_ssh_terminal'){host.terminals=host.terminals.filter(item=>item.id!==info.id);return {Ok:{kind:'terminal',data:info}};}
    return {Ok:{kind:'terminal_snapshot',data:{info}}};
  };
  const first=view.closeTerminal(terminal.id,cx);
  for(let step=0;step<20&&!requested;step++)await Promise.resolve();
  assert.equal(requested,true);assert.equal(view.pending,null);
  const next=view.closeTerminal(second.id,cx);
  assert.equal(view.closingTerminals.has(terminal.id),true);assert.equal(view.closingTerminals.has(second.id),true);
  assert.deepEqual(released,[terminal.id,second.id]);
  profiles.resolve([profile]);await Promise.all([first,next]);
  assert.equal(view.closingTerminals.size,0);assert.equal(view.terminal,null);
  assert.equal(reads.filter(value=>value.kind==='close_ssh_terminal').length,2);
});

test('path editing commits Enter cancels Escape and navigates breadcrumbs',async()=>{
  const {view,cx,host,opened,focusedTexts,settle}=await browsing(),files=view.files();
  host.browse=async({path})=>({kind:'directory',path,entries:[],next:null});
  files.path='/remote/start';view.editPath(cx);
  assert.equal(files.editing,true);assert.deepEqual(focusedTexts,[files.input]);
  files.input='/manual/path';view.textEvent({id:files.input,kind:'enter'},cx);await settle();
  assert.equal(files.path,'/manual/path');assert.equal(files.editing,false);assert.equal(opened.at(-1).path,'/manual/path');
  view.editPath(cx);const count=opened.length;let stopped=false;
  view.pathKey({key:'escape'},{...cx,stop_propagation(){stopped=true;}});
  assert.equal(files.editing,false);assert.equal(stopped,true);assert.equal(opened.length,count);
  stopped=false;view.pathKey({key:'escape'},{...cx,stop_propagation(){stopped=true;}});assert.equal(stopped,false,'Escape outside editing passes on');
  view.control({id:'ssh-files-pathbar',value:1},cx);await settle();assert.equal(opened.at(-1).path,'/manual');
  view.control({id:'ssh-files-pathbar',value:0},cx);await settle();assert.equal(opened.at(-1).path,'/');
  const before=opened.length;view.control({id:'ssh-files-pathbar',value:12},cx);await settle();assert.equal(opened.length,before);
  assert.equal(files.editing,false,'segment clicks do not open the editor');
});

test('blank header clicks edit the path and blur cancels it',async()=>{
  const {view,cx,host,opened,texts,focusedTexts,settle}=await browsing(),files=view.files();
  host.browse=async({path})=>({kind:'directory',path,entries:[],next:null});
  files.path='/remote/start';view.editPath(cx);
  assert.equal(files.editing,true);assert.deepEqual(focusedTexts,[files.input]);
  assert.deepEqual(texts.at(-1),['set',files.input,'/remote/start']);
  const count=opened.length;
  view.textEvent({id:'other-field',kind:'blur'},cx);assert.equal(files.editing,true,'another field blurring keeps editing');
  view.textEvent({id:files.input,kind:'blur'},cx);await settle();
  assert.equal(files.editing,false);assert.equal(opened.length,count,'blur loads nothing');
  assert.deepEqual(texts.at(-1),['set',files.input,'/remote/start']);
  view.textEvent({id:files.input,kind:'enter'},cx);await settle();assert.equal(opened.length,count,'Enter after cancelling loads nothing');
  files.pending=true;view.editPath(cx);assert.equal(files.editing,false,'a pending load keeps the breadcrumb');
});

test('tree expansion pagination and retries preserve the workspace root',async()=>{
  const {view,cx,host,opened}=await browsing(),files=view.files();
  host.browse=async({path,after})=>({kind:'directory',path,entries:path==='/'?[{name:'folder',kind:'directory'}]:[{name:after?'second':'first',kind:'file'}],next:path==='/folder'&&!after?'page-2':null});
  await files.load('/',cx);await view.tree({tree:'ssh-files',kind:'expand',id:'/folder'},cx);
  assert.equal(files.path,'/');assert.equal(files.expanded.has('/folder'),true);
  const marker=files.items(()=>[],{}).at(0).children.at(-1).id;
  await view.tree({tree:'ssh-files',kind:'open',id:marker},cx);
  assert.equal(opened.at(-1).after,'page-2');assert.deepEqual(plain(files.directories.get('/folder').entries.map(entry=>entry.name)),['first','second']);
  await view.tree({tree:'ssh-files',kind:'collapse',id:'/folder'},cx);assert.equal(files.expanded.has('/folder'),false);
  const count=opened.length;await view.tree({tree:'ssh-files',kind:'expand',id:'/folder'},cx);assert.equal(opened.length,count);
  host.browse=async()=>{throw Error('failed listing');};await files.loadBranch('/folder',cx);
  const retry=files.items(()=>[],{}).at(0).children.find(item=>item.id.includes('"kind":"retry"')).id;
  host.browse=async({path})=>({kind:'directory',path,entries:[],next:null});await view.tree({tree:'ssh-files',kind:'open',id:retry},cx);
  assert.equal(files.directories.get('/folder').error,null);assert.equal(files.path,'/');
});

test('opening a file previews it and a folder does not',async()=>{
  const {view,cx,host,previewed}=await browsing(),files=view.files();
  host.browse=async({path})=>({kind:'directory',path,entries:path==='/'?[{name:'folder',kind:'directory'},{name:'notes.md',kind:'file'}]:[{name:'inner.go',kind:'file'}],next:null});
  await files.load('/',cx);await view.tree({tree:'ssh-files',kind:'expand',id:'/folder'},cx);
  await view.tree({tree:'ssh-files',kind:'open',id:'/notes.md'},cx);
  await view.tree({tree:'ssh-files',kind:'open',id:'/folder/inner.go'},cx);
  await view.tree({tree:'ssh-files',kind:'open',id:'/folder'},cx);
  assert.deepEqual(plain(previewed.map(([profile,path])=>[profile.id,path])),[[files.profile.id,'/notes.md'],[files.profile.id,'/folder/inner.go']]);
});

test('native file and blank menus retain scoped file actions',async()=>{
  const {view,cx,copied}=await browsing(),files=view.files();
  files.path='/remote';files.entries=[{name:'folder',kind:'directory'},{name:'a',kind:'file'}];
  for(const path of ['', '/remote/a']){
    const actions=view.fileMenu(path).map(item=>JSON.parse(item.id));
    assert.deepEqual(plain(actions.map(item=>item.action)),['new_file','new_directory','upload','download','copy','cut','paste','rename','delete','copy_path','refresh']);
    for(const item of actions){assert.equal(item.path,path);assert.equal(item.profile,files.profile.id);assert.equal(item.generation,files.revision);}
  }
  await view.tree({tree:'ssh-files',kind:'context',id:'/remote/a'},cx);
  assert.deepEqual(Array.from(files.selected),['/remote/a']);assert.equal(files.current,'/remote/a');
  await view.tree({tree:'ssh-files',kind:'menu',action:view.fileId('copy_path','/remote/a')},cx);
  assert.deepEqual(copied,['/remote/a']);
  await view.tree({tree:'ssh-files',kind:'menu',action:view.fileId('copy','/remote/a')},cx);
  assert.deepEqual(plain(files.clipboard),{paths:['/remote/a'],cut:false});
});

test('rename and new entries target their nested directory',async()=>{
  for(const [action,path,operation]of[
    ['rename','/remote/folder/original',{path:'/remote/folder/original',action:{operation:'move',destination:'/remote/folder/changed'}}],
    ['new_file','/remote/folder/original',{path:'/remote/folder/changed',action:{operation:'create',directory:false}}],
    ['new_directory','/remote/folder',{path:'/remote/folder/changed',action:{operation:'create',directory:true}}],
  ]){const {view,cx,settle}=await browsing(),files=view.files(),changes=[];
    files.path='/remote';files.entries=[{name:'folder',kind:'directory'}];files.directories.set('/remote/folder',{entries:[{name:'original',kind:'file'}],loaded:true});
    view.mutate=async(scope,actions)=>{assert.equal(scope.files,files);changes.push(...actions);};
    await view.fileAction(view.fileId(action,path),cx);assert.equal(view.fileEdit.directory,'/remote/folder');
    view.fileEdit.input='changed';view.submitFile(cx);await settle();assert.deepEqual(plain(changes),[operation]);
  }
});

test('external drops and transfers retain captured nested targets',async()=>{
  const {view,cx,host,dropped,started}=await browsing(),files=view.files(),downloads=[];
  files.path='/remote';files.entries=[{name:'folder',kind:'directory'}];files.directories.set('/remote/folder',{entries:[{name:'file',kind:'file'}],loaded:true});
  await view.tree({tree:'ssh-files',kind:'external_drop',id:'/remote/folder/file',files:['os-file']},cx);
  assert.deepEqual(plain(dropped),[[files.profile,'/remote/folder',['os-file']]]);assert.deepEqual(started,[['dropped-job','start']]);
  host.download=async(profile,entries)=>{downloads.push({profile,entries});return ['download-job'];};
  files.selected=['/remote/folder','/remote/folder/file'];await view.fileAction(view.fileId('download','/remote/folder'),cx);
  assert.deepEqual(plain(downloads),[{profile:files.profile,entries:[{path:'/remote/folder',directory:true},{path:'/remote/folder/file',directory:false}]}]);
  assert.deepEqual(started.at(-1),['download-job','start']);
  view.transferAction({id:'replace-job'},'replace',cx);assert.deepEqual(started.at(-1),['replace-job','replace']);
  view.connected=false;await view.tree({tree:'ssh-files',kind:'external_drop',id:'/remote/folder',files:['later']},cx);assert.equal(dropped.length,1);
});

test('upload confirmation retains the target until explicit submission',async()=>{
  const {view,cx,profile,settle}=await browsing(),commands=[];
  view.transfer={profile,worktree:'project-tree',inputs:['local.txt','/remote/existing.txt']};
  view.execute=async value=>commands.push(value);view.transferFile('upload',cx);
  assert.equal(commands.length,0);assert.equal(view.confirmation.label,'ssh_upload');
  view.confirm(cx);await settle();
  assert.deepEqual(plain(commands),[{kind:'transfer_ssh',data:{profile:profile.id,expected_revision:profile.revision,
    transfer:{worktree:'project-tree',path:'local.txt',remote_path:'/remote/existing.txt',direction:'upload'},timeout_ms:300000}}]);
});

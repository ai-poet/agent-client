import test from 'node:test';
import assert from 'node:assert/strict';
import {Controller} from '../dev.sailry.platform/desktop/controller.js';
import {listing} from '../dev.sailry.platform/desktop/policy.js';

function fixture(surface = 'project') {
  let location = {cursor:'1',surface,node:'node-a',project:'project-a',worktree:'tree-a',session:surface === 'composer' ? 'session-a' : null,can_move:true,can_retarget:false,connected:true,projects:[],main:true,project_name:'Project',branch:'main',git:true};
  const state = {prepared:[],requests:new Set(),completed:[],forgotten:[],selected:[],released:[],toasts:[],resolution:'head-a',reads:0,result:null,drafts:false};
  const api = {
    readLocation:() => location,
    selectLocation:async (cursor,value) => {if (value.request) assert.ok(!state.forgotten.includes(value.request));state.selected.push({cursor,value});return true;},
    readWorktreeCatalog:async () => ({projects:[{id:'project-a',name:'Project'}],worktrees:[{id:'tree-a',path:'/main',main:true}]}),
    listWorktrees:async () => ({kind:'repository',entries:[{path:'/main',branch:'main',head:'head-a',main:true,locked:false,available:true},{path:'/native',branch:'topic',head:'head-b',main:false,locked:false,available:true}],truncated:false,omitted_paths:'0'}),
    inspectGit:async () => ({head:'head-a',branch:'main',index_revision:'index-a'}),
    resolveGitRevision:async () => {state.reads++;return {commit:state.resolution};},
    prepareWorktreeChange:value => {if(state.requests.size>=16)throw Error('plugin request draft capacity exhausted');const id = `request-${state.prepared.length}`;state.prepared.push({id,value});state.requests.add(id);return id;},
    prepareGitChange:value => api.prepareWorktreeChange(value),
    completeRequest:async id => {state.completed.push(id);if (state.result instanceof Error) throw state.result;if (typeof state.result === 'function') return state.result(id);return state.result ?? {Ok:{kind:'worktree',data:{id:'tree-new'}}};},
    forgetRequest:id => {state.forgotten.push(id);state.requests.delete(id);},
    worktreeHasDrafts:async (_id,request) => {if (request) assert.ok(!state.forgotten.includes(request));return state.drafts;},
    releaseWorktree:async (id,request) => {assert.ok(!state.forgotten.includes(request));state.released.push({id,request});},
    toast:value => state.toasts.push(value),newId:() => '12345678-abcd',
  };
  const model = new Controller(api,key => key,() => {});
  return {model,state,api,change:patch => {location = {...location,...patch};model.accept(location);}};
}
const inputs = {revision:'release',branch:'topic/new',path:'/checkouts/new'};

const deferred = () => {let resolve,reject;const promise=new Promise((done,fail)=>{resolve=done;reject=fail;});return {promise,resolve,reject};};
const settle = async () => {for(let index=0;index<8;index++)await Promise.resolve();};

test('native discovery starts before the catalog returns',async()=>{
  const {model,api}=fixture();const catalog=deferred(),native=deferred();let reads=0;
  api.readWorktreeCatalog=()=>catalog.promise;
  api.listWorktrees=()=>{reads++;return native.promise;};
  const task=model.open();assert.equal(reads,1);
  native.resolve({kind:'repository',entries:[{path:'/main',main:true,available:true}],truncated:false,omitted_paths:'0'});
  catalog.resolve({projects:[{id:'project-a',name:'Project'}],worktrees:[{id:'tree-a',path:'/main',main:true}]});
  await task;assert.equal(model.dialog.loading,false);
  assert.equal(model.dialog.choices.length,1);
});

test('registered choices appear before native discovery completes',async()=>{
  const {model,api,state}=fixture();const native=deferred();let reads=0;
  api.listWorktrees=()=>{reads++;return native.promise;};
  const first=model.open();await settle();const owner=model.dialog;
  assert.equal(owner.loading,true);assert.equal(owner.choices.length,1);
  assert.equal(owner.choices[0].checked,true);
  assert.deepEqual(owner.choices[0].action.actions.map(action=>action.kind),['open']);
  const second=model.open();assert.equal(model.dialog,owner);assert.equal(reads,1);
  await model.choose('entry-0');assert.equal(owner.kind,'actions');
  native.resolve({kind:'repository',entries:[],truncated:false,omitted_paths:'0'});
  await Promise.all([first,second]);
  assert.equal(owner.kind,'actions');assert.equal(owner.choices[0].action.kind,'open');
  await model.choose('action-0');assert.equal(state.selected[0].value.worktree,'tree-a');
});

test('native discovery cannot reopen a closed or changed picker',async()=>{
  for (const changed of [false,true]) {
    const {model,api,change}=fixture();const native=deferred();api.listWorktrees=()=>native.promise;
    const task=model.open();await settle();
    if(changed)change({cursor:'2',worktree:'tree-b'});else model.close();
    native.resolve({kind:'repository',entries:[{path:'/late',available:true}],truncated:false,omitted_paths:'0'});
    await task;assert.equal(model.dialog,null);
  }
});

test('manual create pins resolution and ID for unchanged retries', async () => {
  const {model,state} = fixture();
  model.create({branch:'main',head:'head-a'});
  state.result = {Err:{code:'invalid_request'}};
  await model.submit(inputs);
  state.resolution = 'head-moved';
  await model.submit(inputs);
  assert.equal(state.reads,1);
  assert.deepEqual(state.completed,['request-0','request-0']);
  assert.equal(state.prepared[0].value.commit,'head-a');
  state.result = null;
  await model.submit({...inputs,branch:'other'});
  assert.equal(state.reads,2);
  assert.equal(state.prepared[1].value.commit,'head-moved');
  assert.deepEqual(state.selected,[{cursor:'1',value:{worktree:'tree-new',fork:false,request:'request-1'}}]);
  assert.equal(model.dialog,null);
});

test('unconfirmed create retains original immutable request', async () => {
  const {model,state} = fixture();
  model.create({branch:'main',head:'head-a'});
  state.result = Error('receipt unavailable');
  await model.submit(inputs);
  assert.equal(model.dialog.uncertain,true);
  await model.submit({...inputs,branch:'must-not-run'});
  assert.equal(state.prepared.length,1);
  assert.equal(state.completed.length,1);
  assert.equal(state.requests.size,1);assert.deepEqual(state.forgotten,[]);
  state.result = null;
  await model.submit(inputs);
  assert.deepEqual(state.completed,['request-0','request-0']);
  assert.equal(state.requests.size,0);
});

test('closing failed forms restores request capacity',async()=>{
  const {model,state}=fixture();state.result={Err:{code:'conflict'}};
  for(let index=0;index<17;index++) {
    model.create({branch:'main'});await model.submit(inputs);model.close();
    assert.equal(state.requests.size,0);
  }
  assert.equal(state.prepared.length,17);assert.equal(state.forgotten.length,17);
});

test('closing uncertain forms releases only their abandoned handle',async()=>{
  const {model,state}=fixture();model.create({branch:'main'});state.result=Error('disconnected');
  await model.submit(inputs);assert.equal(model.dialog.uncertain,true);
  model.close();assert.equal(state.requests.size,0);assert.deepEqual(state.forgotten,['request-0']);
  assert.deepEqual(state.selected,[]);assert.deepEqual(state.released,[]);
});

test('replacing failed forms releases their abandoned requests',async()=>{
  const {model,state}=fixture();model.create({branch:'main'});state.result={Err:{code:'conflict'}};
  await model.submit(inputs);model.create({branch:'different'});
  assert.equal(state.requests.size,0);assert.deepEqual(state.forgotten,['request-0']);
});

test('cancelled forms release late faults without changing replacements',async()=>{
  for(const receipt of [{Err:{code:'conflict'}},{Err:{code:'outcome_unknown'}},new Error('disconnected')]) {
    const {model,state}=fixture();let resolve,reject;
    state.result=()=>new Promise((accept,fail)=>{resolve=accept;reject=fail;});
    model.create({branch:'main'});const task=model.submit(inputs);await Promise.resolve();
    model.close();const replacement=model.create({branch:'different'});
    assert.equal(state.requests.size,1);assert.deepEqual(state.forgotten,[]);
    if(receipt instanceof Error)reject(receipt);else resolve(receipt);
    await task;
    assert.equal(model.dialog,replacement);assert.equal(state.requests.size,0);
    assert.deepEqual(state.forgotten,['request-0']);assert.deepEqual(state.selected,[]);
  }
});

test('closing while revision resolves cannot create a checkout', async () => {
  const {model,state,api} = fixture();let resolve;
  api.resolveGitRevision = () => new Promise(done => {resolve = done;});
  model.create({branch:'main'});const task = model.submit(inputs);
  model.close();resolve({commit:'head-a'});await task;
  assert.equal(state.prepared.length,0);
  assert.equal(model.dialog,null);
});

test('late admitted completion preserves a replacement form and selection', async () => {
  const {model,state} = fixture();let finish;
  state.result = () => new Promise(done => {finish = done;});
  model.create({branch:'main'});const task = model.submit(inputs);
  await Promise.resolve();model.close();const replacement = model.create({branch:'other'});
  finish({Ok:{data:{id:'tree-new'}}});await task;
  assert.equal(model.dialog,replacement);
  assert.deepEqual(state.selected,[]);
  assert.deepEqual(state.forgotten,['request-0']);
});

test('cancelled removal releases the worktree before abandoning its request',async()=>{
  const {model,state}=fixture();await model.open();await model.choose('entry-1');await model.choose('action-1');
  let finish;state.result=()=>new Promise(done=>{finish=done;});
  const task=model.submit();await Promise.resolve();
  model.close();const replacement=model.create({branch:'other'});
  assert.deepEqual(state.forgotten,['request-0']);assert.equal(state.requests.size,1);
  finish({Ok:{kind:'worktree_removed',data:{id:'tree-new'}}});await task;
  assert.equal(model.dialog,replacement);assert.equal(state.requests.size,0);
  assert.deepEqual(state.released,[{id:'tree-new',request:'request-1'}]);
  assert.deepEqual(state.forgotten,['request-0','request-1']);
});

test('managed checkout keeps source revisions and deferred conversation handoff', async () => {
  const {model,state} = fixture('composer');
  await model.open('fork');
  assert.equal(model.dialog.include_changes,true);
  assert.equal(model.dialog.values.branch,'sailry/task-12345678');
  await model.submit({branch:'  task/branch  '});
  assert.deepEqual(state.prepared[0].value,{kind:'managed',project:'project-a',branch:'task/branch',expected_head:'head-a',expected_index:'index-a',include_changes:true});
  assert.deepEqual(state.selected,[{cursor:'1',value:{worktree:'tree-new',fork:true,request:'request-0'}}]);
});

test('changed conversation cannot be moved after checkout succeeds', async () => {
  const {model,state,change} = fixture('composer');let finish;
  await model.open('create');state.result = () => new Promise(done => {finish = done;});
  const task = model.submit({branch:'task/branch'});change({cursor:'2'});
  finish({Ok:{data:{id:'tree-new'}}});await task;
  assert.equal(model.dialog.error,'location_created_changed');
  assert.deepEqual(state.selected,[]);
  assert.equal(state.prepared.length,1);
});

test('unregistered removal registers without selecting, then checks drafts again', async () => {
  const {model,state} = fixture();await model.open();await model.choose('entry-1');await model.choose('action-1');
  assert.equal(state.prepared[0].value.kind,'register');
  assert.equal(model.dialog.kind,'remove');assert.deepEqual(state.selected,[]);
  state.drafts = true;await model.submit();assert.equal(state.prepared.length,1);
  assert.equal(model.dialog.error,'worktree_remove_unsaved');
  state.drafts = false;state.result = {Ok:{kind:'worktree_removed',data:{id:'tree-new'}}};await model.submit();
  assert.deepEqual(state.prepared[1].value,{kind:'remove',worktree:'tree-new',expected_head:'head-b',expected_branch:'topic'});
  assert.deepEqual(state.released,[{id:'tree-new',request:'request-1'}]);
});

test('removal conflicts retain original observed revision and ID', async () => {
  const {model,state} = fixture();await model.open();await model.choose('entry-1');await model.choose('action-1');
  state.result = {Err:{code:'revision_conflict'}};await model.submit();await model.submit();
  assert.equal(model.dialog.error,'worktree_remove_conflict');
  assert.deepEqual(state.completed,['request-0','request-1','request-1']);
  assert.deepEqual(state.released,[]);
});

test('frozen picker rows cannot be substituted by a refreshed array', async () => {
  const {model,state} = fixture();await model.open();const owner = model.dialog;
  const captured = {token:owner.token,choice:owner.choices[1]};
  owner.choices = [];
  await model.choose('entry-1',captured);await model.choose('action-0');
  assert.equal(state.prepared[0].value.path,'/native');
});

test('menu project choices use the live cursor',async()=>{
  const {model,state,change}=fixture('composer');
  change({project:null,worktree:null,session:null,can_retarget:true,projects:[{id:'project-a',name:'Project'}]});
  await model.select('unassigned');
  change({cursor:'2'});
  await model.select('project:project-a');
  await model.select('add_project');
  assert.deepEqual(state.selected,[
    {cursor:'2',value:{project:'project-a'}},
    {cursor:'2',value:{add_project:true}},
  ]);
  assert.equal(state.toasts.length,0);
});

test('unavailable menu choices report context changes',async()=>{
  for (const id of ['project:other','worktree:tree-a','create','manage']) {
    const {model,state,change}=fixture('composer');
    change({session:null,can_retarget:true,projects:[{id:'project-a',name:'Project'}]});
    await model.select(id);
    assert.deepEqual(state.selected,[]);
    assert.equal(state.toasts.at(-1).message,'worktree_context_changed');
  }
  const {model,state}=fixture('composer');
  await model.select('project:project-a');
  assert.deepEqual(state.selected,[]);
  assert.equal(state.toasts.at(-1).message,'worktree_context_changed');
});

test('menus cache discovery and management refreshes explicitly',async()=>{
  const {model,api,change}=fixture('composer');
  const catalog=api.readWorktreeCatalog,native=api.listWorktrees;
  let catalogs=0,discoveries=0;
  api.readWorktreeCatalog=()=>{catalogs++;return catalog();};
  api.listWorktrees=()=>{discoveries++;return native();};
  const first=model.refresh();
  assert.equal(model.refresh(),first);
  const opening=model.open();
  await Promise.all([first,opening]);
  model.choices();model.choices();await model.refresh();
  assert.equal(catalogs,1);assert.equal(discoveries,1);
  await model.open();
  assert.equal(catalogs,2);assert.equal(discoveries,2);
  change({cursor:'2',branch:'topic'});
  await model.refresh();
  assert.equal(catalogs,2);assert.equal(discoveries,2);
});

test('late catalogs cannot replace a changed location',async()=>{
  const {model,api,change}=fixture();
  const catalog=deferred();api.readWorktreeCatalog=()=>catalog.promise;
  const reading=model.refresh();
  change({cursor:'2',project:null,worktree:null,session:null,can_retarget:true,projects:[]});
  await model.refresh();
  catalog.resolve({projects:[],worktrees:[{id:'old',path:'/old',main:true}]});
  await reading;
  assert.equal(model.catalog,null);
  assert.deepEqual(model.choices().map(choice=>choice.id),['unassigned','add_project']);
});

test('inventory refresh preserves form ownership and handoff',async()=>{
  const {model,api,state}=fixture('composer');
  await model.refresh();
  await model.createManaged(false);
  const owner=model.dialog,cursor=model.location.cursor;
  const catalog=api.readWorktreeCatalog;
  api.readWorktreeCatalog=async()=>({...await catalog(),worktrees:[
    {id:'tree-a',path:'/main',main:true},{id:'added',path:'/added',main:false},
  ]});
  const native=api.listWorktrees;
  api.listWorktrees=async()=>{
    const found=await native();
    return {...found,entries:[...found.entries,{path:'/added',branch:'topic/added',head:'head-added',main:false,available:true}]};
  };
  await model.invalidate();
  assert.equal(model.dialog,owner);
  assert.equal(model.location.cursor,cursor);
  assert.ok(model.catalog.worktrees.some(tree=>tree.id==='added'));
  await model.submit({branch:'task/new'});
  assert.deepEqual(state.selected,[{cursor,value:{worktree:'tree-new',fork:false,request:'request-0'}}]);
});

test('inventory changes discard reads captured before invalidation',async()=>{
  const {model,api}=fixture();
  const first=deferred(),second=deferred();
  const pending=[first,second];
  api.readWorktreeCatalog=()=>pending.shift().promise;
  const initial=model.refresh(),refresh=model.invalidate();
  second.resolve({projects:[],worktrees:[{id:'new',path:'/new',main:true}]});
  await refresh;
  first.resolve({projects:[],worktrees:[{id:'old',path:'/old',main:true}]});
  await initial;
  assert.deepEqual(model.catalog.worktrees.map(tree=>tree.id),['new']);
  assert.equal(model.location.cursor,'1');
});

test('registration refreshes catalog before a removal form',async()=>{
  const {model,api,state}=fixture();
  const trees=[{id:'tree-a',path:'/main',main:true}];
  api.readWorktreeCatalog=async()=>({projects:[],worktrees:[...trees]});
  state.result=()=>{
    trees.push({id:'registered',path:'/native',main:false});
    return {Ok:{data:{id:'registered'}}};
  };
  await model.open();
  await model.choose('entry-1');await model.choose('action-1');
  await model.refresh();
  assert.equal(model.dialog.kind,'remove');
  assert.equal(model.dialog.bound.cursor,'1');
  assert.ok(model.catalog.worktrees.some(tree=>tree.id==='registered'));
  assert.deepEqual(state.selected,[]);
});

test('removal refreshes an unchanged location menu',async()=>{
  const {model,api,state}=fixture();
  let trees=[{id:'tree-a',path:'/main',main:true},{id:'registered',path:'/native',main:false}];
  api.readWorktreeCatalog=async()=>({projects:[],worktrees:[...trees]});
  state.result=()=>{
    trees=trees.filter(tree=>tree.id!=='registered');
    return {Ok:{kind:'worktree_removed',data:{id:'registered'}}};
  };
  await model.open();
  await model.choose('entry-1');await model.choose('action-1');
  await model.submit();await model.refresh();
  assert.equal(model.location.cursor,'1');
  assert.equal(model.catalog?.worktrees.some(tree=>tree.id==='registered') ?? false,false);
  assert.deepEqual(state.selected,[]);
});

test('completed mutations cannot invalidate another project',async()=>{
  const {model,api,change}=fixture();
  const owner=model.owner('create');
  change({cursor:'2',project:'project-b',worktree:'tree-b'});
  await model.refresh();
  const accepted=model.catalog;
  api.readWorktreeCatalog=async()=>{throw Error('Unexpected read');};
  await model.invalidate(owner);
  assert.equal(model.catalog,accepted);
});

test('failed discovery reports once and allows manual refresh',async()=>{
  const {model,api,state}=fixture();
  const native=api.listWorktrees;
  api.listWorktrees=async()=>{throw Error('Disconnected');};
  await model.open();
  assert.equal(state.toasts.length,1);
  assert.equal(state.toasts[0].message,'worktree_load_failed');
  api.listWorktrees=native;
  await model.open();
  assert.ok(model.dialog.choices.some(choice=>choice.id==='entry-1'));
});

test('offline refreshes skip reads and background failures stay quiet',async()=>{
  const {model,api,state,change}=fixture();
  const catalog=api.readWorktreeCatalog;
  let catalogs=0,discoveries=0;
  api.readWorktreeCatalog=async()=>{catalogs++;throw Error('Disconnected');};
  api.listWorktrees=async()=>{discoveries++;return {entries:[]};};
  change({connected:false});
  await model.refresh();
  assert.equal(catalogs,0);assert.equal(discoveries,0);
  change({connected:true});
  await model.refresh();
  assert.equal(catalogs,1);assert.equal(discoveries,1);
  assert.deepEqual(state.toasts,[]);
  assert.equal(model.catalog,null);
  api.readWorktreeCatalog=catalog;
  await model.refresh();
  assert.equal(model.catalog.worktrees[0].id,'tree-a');
  assert.deepEqual(state.toasts,[]);
});

test('offline explicit management reports one load failure',async()=>{
  const {model,state,change}=fixture();
  change({connected:false});
  await model.open();
  assert.equal(state.toasts.length,1);
  assert.equal(state.toasts[0].message,'worktree_load_failed');
  assert.equal(model.dialog.loading,false);
});

test('only resource identity changes invalidate successful discoveries',async()=>{
  for(const patch of [{cursor:'2'},{branch:'changed'},{session:'other'},{connected:false}]) {
    const {model,change}=fixture();
    await model.refresh();
    const read=model.catalogRead;
    change(patch);
    assert.equal(model.catalogRead,read);
    assert.ok(model.catalog);
  }
  for(const patch of [{node:'other'},{project:'other'},{worktree:'other'}]) {
    const {model,change}=fixture();
    await model.refresh();
    change(patch);
    assert.equal(model.catalogRead,null);
    assert.equal(model.catalog,null);
  }
});

test('pending discovery survives a metadata update',async()=>{
  const {model,api,change}=fixture(),catalog=deferred();
  api.readWorktreeCatalog=()=>catalog.promise;
  const read=model.refresh();
  change({cursor:'2',branch:'topic'});
  catalog.resolve({projects:[],worktrees:[{id:'tree-a',path:'/main',main:true}]});
  await read;
  assert.equal(model.catalog.worktrees[0].id,'tree-a');
  assert.equal(model.entries[0].branch,'main');
});

test('management back restores frozen worktree choices',async()=>{
  const {model}=fixture();
  await model.open();
  const choices=model.dialog.choices;
  await model.choose('entry-1');
  assert.equal(model.dialog.kind,'actions');
  model.back();
  assert.equal(model.dialog.kind,'list');
  assert.equal(model.dialog.choices,choices);
});

test('stale and locked native entries preserve action policy', () => {
  const catalog = {projects:[{id:'project-a',name:'Project'}],worktrees:[]};
  const location = {project:'project-a',surface:'project'};
  const base = {main:false,path:'/missing',head:'abcdef012345',branch:null,available:false,locked:false};
  const found = {kind:'repository',entries:[base,{...base,path:'/locked',locked:true}],truncated:true,omitted_paths:'1'};
  const choices = listing(catalog,found,location,key => key);
  assert.equal(choices[0].label,'abcdef01');assert.equal(choices[0].action.kind,'prune');
  assert.equal(choices[1].disabled,true);assert.equal(choices.at(-1).id,'partial');
  assert.equal(choices.at(-1).disabled,true);
});

test('forks from another branch start at its commit without source changes',async()=>{
  const {model,state}=fixture('composer');
  const commit='b'.repeat(40);
  await model.open('fork',{base:{from:'dev',commit}});
  assert.equal(model.dialog.kind,'managed');
  assert.deepEqual(model.dialog.base,{from:'dev',commit});
  assert.equal(model.dialog.include_changes,false);
  await model.submit({branch:'sailry/task-12345678'});
  assert.deepEqual(state.prepared[0].value,{kind:'managed',project:'project-a',branch:'sailry/task-12345678',
    expected_head:'',expected_index:'',include_changes:false,base:commit});
  assert.deepEqual(state.selected.at(-1).value,{worktree:'tree-new',fork:true,request:'request-0'});
  await model.open('fork',{base:{from:'dev',commit:'short'}});
  assert.equal(model.dialog.base,null);
  assert.equal(model.dialog.include_changes,true);
});

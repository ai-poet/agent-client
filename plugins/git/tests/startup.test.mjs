import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import {Repository} from '../dev.sailry.platform/desktop/repository.js';
import {Menus} from '../dev.sailry.platform/desktop/menus.js';
import {Surfaces} from '../dev.sailry.platform/desktop/surfaces.js';
import * as policy from '../dev.sailry.platform/desktop/session-branch.js';

const source = (await readFile(new URL('../dev.sailry.platform/desktop/main.js', import.meta.url), 'utf8'))
  .replace(/^import[\s\S]*?;\n/gm, '').replace('export default class', 'class');
const contextSource = (await readFile(new URL('../dev.sailry.platform/desktop/context.js', import.meta.url), 'utf8'))
  .replace(/^import[\s\S]*?;\n/gm, '').replace('export default class', 'class');
const deferred = () => { let resolve,reject; const promise=new Promise((done,fail)=>{resolve=done;reject=fail;}); return {promise,resolve,reject}; };
const settle = async () => { for (let index=0;index<12;index++) await Promise.resolve(); };

function fixture(discovery, options = {}) {
  const changes = [deferred(),deferred(),deferred()]; let reads=0, queries=0, cursor=0;
  const targets = [deferred(),deferred(),deferred()]; let target = 0;
  const shortcuts=[],scopes=[];
  const globals = {View:class {}, Surfaces, DiffState:()=>({}), EditorState:text=>({text}), context:()=>'{"locale":"en"}',
    messages:()=>({}), workspaceMode:()=>options.mode ?? 'main', createText:()=> 'message', Repository,
    Actions:class {}, Menus:class {}, readResourceTarget:()=>options.target ?? null,
    nextResourceTarget:()=>targets[target++].promise,focusEntry:()=>{},
    nextWindowActivation:()=>new Promise(()=>{}),registerShortcuts:bindings=>{shortcuts.push(bindings);return 'FixtureGitKeys';},
    canInvokeContribution:()=>{queries++;return discovery.promise;},
    sdk:{nextChange:(_cursor,scope)=>{scopes.push(scope);return changes[cursor++].promise;},...options.api}};
  for (const name of ['nextMenuEvent','nextContextMenuEvent','nextControlEvent','nextTreeEvent',
    'nextNavigationTabEvent','nextPickerEvent','modal_closed','nextTextEvent']) globals[name]=()=>{};
  const Git=vm.runInNewContext(`${source}\nGit`,globals);
  const owner=new Git(); owner.watch=()=>{};
  if (!options.observe) owner.invalidate=async()=>{reads++;};
  const tasks=[];let notifications=0;
  const live={notify:()=>{notifications++;},spawn:task=>{const pending=task(live);tasks.push(pending);return pending;}};
  const cx={notify:()=>{},sleep:async()=>{},spawn:live.spawn,focus_handle:()=>({focus(){}})};
  owner.init({},cx);
  return {owner,changes,targets,tasks,cx,live,shortcuts,scopes,notifications:()=>notifications,reads:()=>reads,queries:()=>queries};
}

test('empty resource targets keep document selection unchanged',async()=>{
  for (const mode of ['embedded','main']) for (const value of [null,{}, {path:null}]) {
    let diffs=0;
    const state=fixture(deferred(),{mode,target:{cursor:'0',value},api:{
      inspectGit:async()=>({kind:'directory',entries:[]}),
      readGitDiff:async path=>{diffs++;return {path,text:'diff'};},
    }});
    await settle();
    assert.equal(state.owner.repo.selected,null);
    assert.equal(state.owner.repo.tabs.length,0);
    assert.equal(state.owner.repo.documents.size,0);
    assert.equal(diffs,0);

    state.targets[0].resolve({cursor:'1',value:{path:'first.txt'}});await settle();
    assert.equal(state.owner.repo.selected,'first.txt');assert.equal(diffs,1);
    state.owner.nodes.set('second',{row:{entry:{path:'second.txt'}},scope:'all'});
    state.owner.tree({tree:'git-tree',kind:'select',id:'second'},state.live);await settle();
    assert.equal(state.owner.repo.selected,'second.txt');assert.equal(diffs,2);

    state.targets[1].resolve({cursor:'2',value:{path:null}});await settle();
    assert.equal(state.owner.repo.selected,'second.txt');
    assert.deepEqual(Array.from(state.owner.repo.tabs),['first.txt','second.txt']);
    state.owner.closeDocument('first.txt');assert.equal(state.owner.repo.selected,'second.txt');
    state.owner.closeDocument('second.txt');assert.equal(state.owner.repo.selected,null);
    assert.equal(state.owner.repo.documents.size,0);
    state.owner.repo.status={kind:'ready',entries:[{path:'first.txt'}]};
    state.owner.control('git-open-changes',state.live);await settle();
    assert.equal(state.owner.repo.selected,'changes:');
    assert.equal(state.owner.repo.documents.get('changes:').files.length,1);
    state.owner.release();
  }
});

test('plain directories do not issue diff requests',async()=>{
  for (const value of [null, {path:null}]) {
    const requests=[],reported=[];
    const state=fixture(deferred(),{mode:'embedded',target:{cursor:'0',value},observe:true,api:{
      inspectGit:async()=>{
        requests.push('status');
        return {kind:'directory',head:null,branch:null,index_revision:null,entries:[],truncated:false,omitted_paths:0};
      },
      listGitBranches:async()=>{requests.push('branches');assert.fail('a plain directory has no branches');},
      readGitDiff:async()=>{requests.push('diff');assert.fail('no diff was selected');},
    }});
    state.owner.report=key=>reported.push(key);
    state.changes[0].resolve({cursor:'1',connected:true});await settle();
    assert.deepEqual(requests,['status']);
    assert.equal(state.owner.repo.status.kind,'directory');
    assert.equal(state.owner.repo.selected,null);
    assert.equal(state.owner.repo.error,null);
    assert.deepEqual(reported,[]);
    state.owner.release();
  }
});

test('renderer owns scoped shortcuts including native text input submission',()=>{
  const state=fixture(deferred());
  assert.equal(state.shortcuts.length,1);
  const bindings=state.shortcuts[0];
  assert.equal(bindings.length,8);
  assert.deepEqual(Array.from(bindings.filter(binding=>binding.context==='Input'),binding=>[binding.keystroke,binding.action]),[
    ['secondary-enter','git-commit-action'],['secondary-shift-enter','git-amend-action'],
  ]);
  assert.equal(state.owner.shortcutContext,'FixtureGitKeys');
  state.owner.release();
});

function contributions() {
  const change=deferred(),event=deferred(),reads=[],branches=[],choices=[],published=[],resources=[];let changes=0,events=0;
  const never=()=>new Promise(()=>{});
  const globals={View:class {},Surfaces,DiffState:()=>({}),EditorState:text=>({text}),context:()=>'{"locale":"en"}',
    messages:()=>({}),workspaceMode:()=> 'main',createText:()=> 'message',Repository,Menus,
    Actions:class {
      choose(title,items){const picker={title,items};choices.push(picker);this.picker=picker;return picker;}
      update(picker,items){if(this.picker===picker){picker.items=items;picker.loading=false;}}
    },registerShortcuts:()=> 'FixtureGitKeys',
    readScope:()=>({worktree:'captured'}),readLocation:()=>({surface:'project',worktree:'captured',branch:'main',main:true,git:true,git_status:'repository',can_move:true}),
    nextLocationChange:never,nextWindowActivation:never,readResourceTarget:()=>null,canInvokeContribution:async()=>true,
    nextContributionEvent:()=>events++===0 ? event.promise : never(),publishContributions:states=>published.push(states),toast:()=>{},
    openResource:async(kind,value)=>resources.push({kind,value}),
    sdk:{faultCode:()=> 'unavailable',nextChange:()=>changes++===0 ? change.promise : never(),
      inspectGit:()=>{const pending=deferred();reads.push(pending);return pending.promise;},
      listGitBranches:()=>{const pending=deferred();branches.push(pending);return pending.promise;}}};
  for (const name of ['nextMenuEvent','nextContextMenuEvent','nextControlEvent','nextTreeEvent',
    'nextNavigationTabEvent','nextPickerEvent','modal_closed','nextTextEvent']) globals[name]=never;
  const Context=vm.runInNewContext(`${source}\n${contextSource}\nContext`,{...policy,...globals});
  const owner=new Context();owner.watch=()=>{};
  const live={notify:()=>{},sleep:async()=>{},spawn:task=>task(live),focus_handle:()=>({focus(){}})};owner.init({},live);
  return {owner,change,event,reads,branches,choices,published,resources};
}

function contextFixture(options={}) {
  const published=[];
  const location={cursor:'0',surface:'composer',node:'captured-node',worktree:'captured',branch:'main',
    main:true,git:true,git_status:'repository',connected:true,can_move:true,...options.location};
  const repo={connected:true,status:{kind:'ready',entries:[{path:'notes.txt',diff:{additions:1,deletions:0}}],truncated:false,omitted_paths:0},
    refresh:options.refresh ?? (async()=>true),invalidate(){return this.refresh();}};
  const globals={
    Git:class {
      init(){this.text={git_detached:'Detached HEAD'};this.repo=repo;this.notify=()=>{};}
      async invalidate(){await this.repo.invalidate();}
      refreshWorktrees(){}
    },
    readScope:()=>({worktree:'captured'}),readLocation:()=>location,
    publishContributions:states=>published.push(states),
  };
  const Context=vm.runInNewContext(`${contextSource}\nContext`,{...policy,...globals}),owner=new Context();
  owner.init({}, {spawn:()=>{}});
  return {owner,repo,published};
}

test('branch picker opens before fresh repository observation',async()=>{
  for (const eventFirst of [false,true]) {
    const state=contributions();
    const invoke=()=>state.event.resolve({id:'manage',handler:'manage',sequence:1,value:null});
    const invalidate=()=>state.change.resolve({cursor:'created-worktree',connected:true});
    (eventFirst ? invoke : invalidate)();await settle();
    (eventFirst ? invalidate : invoke)();await settle();
    assert.equal(state.choices.length,1);
    const picker=state.choices[0];assert.equal(picker.loading,true);assert.equal(picker.items.length,0);
    assert.equal(state.reads.length,1,'only one observation runs at a time');
    await state.owner.openPicker('manage');
    assert.equal(state.choices.length,1);assert.equal(state.reads.length,1);
    state.reads[0].resolve({branch:eventFirst ? 'stale' : 'latest',kind:'ready',entries:[]});await settle();
    state.branches[0].resolve({entries:[{name:eventFirst ? 'stale' : 'managed',commit:'abc1234'}],sync:{tracking:{}}});await settle();
    if(eventFirst) {
      assert.equal(state.reads.length,2,'a changed snapshot requires one fresh observation');
      assert.equal(picker.items.length,0,'stale branches are not displayed');
      state.reads[1].resolve({branch:'latest',kind:'ready',entries:[]});await settle();
      state.branches[1].resolve({entries:[{name:'managed',commit:'abc1234'}],sync:{tracking:{}}});await settle();
    }
    assert.equal(state.choices.length,1);assert.equal(state.owner.actions.picker,picker);
    assert.equal(picker.loading,false);assert.equal(state.owner.repo.status.branch,'latest');
    assert(picker.items.some(item=>item.label==='managed'));
    assert(!picker.items.some(item=>item.label==='stale'));
    assert(state.published.some(states=>states.some(item=>item.id==='manage' && item.reply_to===1)));
    state.owner.release();
  }
});

test('closed or released pickers reject late results',async()=>{
  for (const change of ['closed','released','scope']) {
    const state=contributions();state.event.resolve({id:'manage',handler:'manage',sequence:1,value:null});await settle();
    assert.equal(state.choices.length,1);const picker=state.choices[0];
    if (change==='closed') state.owner.actions.picker=null;
    else if (change==='released') state.owner.release();
    else state.owner.location={...state.owner.location,worktree:'other'};
    state.reads[0].resolve({branch:'latest',kind:'ready',entries:[]});await settle();
    if(change==='released') assert.equal(state.branches.length,0,'released controllers do not start branch reads');
    else { state.branches[0].resolve({entries:[{name:'late',commit:'abc1234'}],sync:{tracking:{}}});await settle(); }
    assert.equal(state.choices.length,1);assert.equal(picker.items.length,0);
    assert.equal(state.owner.actions.picker,null);
    state.owner.release();
  }
});

test('failed observation closes its loading picker',async()=>{
  const state=contributions();state.event.resolve({id:'manage',handler:'manage',sequence:1,value:null});await settle();
  assert.equal(state.choices[0].loading,true);
  state.reads[0].reject(Error('repository unavailable'));await settle();
  assert.equal(state.owner.actions.picker,null);assert.equal(state.choices.length,1);state.owner.release();
});

test('composer controllers leave status reads to Node',async()=>{
  let reads=0;
  const state=contextFixture({refresh:async()=>{reads++;return true;}});
  await Promise.all([state.owner.invalidate(),state.owner.invalidate()]);
  assert.equal(reads,0);
  assert.ok(state.published.every(entries=>entries.every(entry=>entry.id!=='changes')));
});

test('other surfaces retain repository refreshes',async()=>{
  let reads=0;
  const state=contextFixture({location:{surface:'project'},refresh:async()=>{reads++;return true;}});
  await state.owner.invalidate();
  assert.equal(reads,1);
  assert.deepEqual(Array.from(state.published.at(-1),entry=>entry.id),['branch','manage']);
});

test('forwarded status events open the Git resource',async()=>{
  const state=contributions();
  state.event.resolve({id:'changes',handler:'changes',sequence:1,kind:'invoke',value:null});
  await settle();
  assert.deepEqual(JSON.parse(JSON.stringify(state.resources)),[{kind:'git',value:{path:null}}]);
  assert.ok(state.published.every(entries=>entries.every(entry=>entry.id!=='changes')));
  state.owner.release();
});

test('retained controllers suspend queries until their view returns',async()=>{
  const discovery=deferred(),state=fixture(discovery);
  state.changes[0].resolve({cursor:'hidden',connected:true,visible:false});await settle();
  assert.equal(state.reads(),0);assert.equal(state.queries(),0);
  state.changes[1].resolve({cursor:'shown',connected:true,visible:true});await settle();
  assert.equal(state.reads(),1);assert.equal(state.queries(),1);
  assert.equal(state.scopes.every(scope=>scope==='repository'),true);
  await settle();assert.equal(state.reads(),1);
  state.owner.release();
});

test('scope changes and disconnect disable repository controls',async()=>{
  for (const moved of [{node:'other-node'},{worktree:'other-worktree'}]) {
    let reads=0;const state=contextFixture({refresh:async()=>{reads++;return true;}});
    state.owner.location={...state.owner.location,...moved};
    await state.owner.invalidate();
    assert.equal(reads,0);
    state.owner.publish();
    assert.deepEqual(Array.from(state.published.at(-1),entry=>entry.visible),[false,false]);
  }
  const state=contextFixture();
  state.repo.connected=false;state.owner.publish();
  assert.deepEqual(Array.from(state.published.at(-1),entry=>entry.visible),[false,false]);
  const branch=state.published.at(-1).find(entry=>entry.id==='branch');
  assert.equal(branch.enabled,false);assert.equal(branch.loading,false);
});

test('repository observation continues while optional worktree discovery is pending',async()=>{
  const discovery=deferred();const state=fixture(discovery);
  state.changes[0].resolve({cursor:'1',connected:true});await settle();
  assert.equal(state.reads(),1);assert.equal(state.queries(),1);
  state.changes[1].resolve({cursor:'2',connected:true});await settle();
  assert.equal(state.reads(),2);assert.equal(state.queries(),1);
  discovery.reject(Error('unavailable contribution'));await settle();
  assert.equal(state.owner.worktrees,false);assert.equal(state.owner.worktreesPending,false);
  state.changes[2].reject(Error('closed'));await settle();
});

test('released controllers ignore optional capability discovery results',async()=>{
  const discovery=deferred();const state=fixture(discovery);
  state.changes[0].resolve({cursor:'1',connected:true});await settle();
  state.owner.release();discovery.resolve(true);await settle();
  assert.equal(state.owner.worktrees,false);
  state.changes[1].reject(Error('closed'));await settle();
});

test('composer location initializes without duplicate repository reads',async()=>{
  const contextSource = (await readFile(new URL('../dev.sailry.platform/desktop/context.js', import.meta.url), 'utf8'))
    .replace(/^import[\s\S]*?;\n/gm, '').replace('export default class', 'class');
  let reads = 0;
  const location = {surface:'composer',worktree:'captured-checkout',branch:'main',main:true,git:true,git_status:'repository',can_move:true};
  const globals = {
    Git:class {
      init() {
        this.text = {};
        this.repo = {invalidate:async()=>{reads++;return false;}};
        this.invalidate();
      }
    },
    readLocation:()=>location,
    readScope:()=>({worktree:'captured-checkout'}),
    publishContributions:()=>{},
  };
  const Context=vm.runInNewContext(`${contextSource}\nContext`,{...policy,...globals});
  const owner=new Context();
  owner.init({}, {spawn:()=>{},notify:()=>{},focus_handle:()=>({focus(){}})});
  await settle();
  assert.equal(owner.location,location);
  assert.equal(reads,0);
});

test('unassigned controllers do not read or publish a repository contribution',async()=>{
  const contextSource = (await readFile(new URL('../dev.sailry.platform/desktop/context.js', import.meta.url), 'utf8'))
    .replace(/^import[\s\S]*?;\n/gm, '').replace('export default class', 'class');
  let reads = 0, published;
  const globals = {
    Git:class {
      init() {
        this.text = {};
        this.repo = {invalidate:async()=>{reads++;return false;}};
        this.invalidate();
      }
    },
    readScope:()=>({worktree:null}),
    // A later presentation update cannot grant a retained controller a checkout.
    readLocation:()=>({surface:'composer',worktree:'new-checkout',branch:'main',main:true,git:true,git_status:'repository',can_move:true}),
    publishContributions:states=>{published=states;},
  };
  const Context=vm.runInNewContext(`${contextSource}\nContext`,{...policy,...globals});
  const owner=new Context();
  owner.init({}, {spawn:()=>{},notify:()=>{},focus_handle:()=>({focus(){}})});
  await owner.invalidate();
  await settle();
  assert.equal(reads,0);
  assert.deepEqual(Array.from(published,state=>state.visible),[false,false]);
});

test('composer metadata changes never duplicate Node scans',async()=>{
  for (const project of [null,'plain-directory']) {
    let reads=0,published;
    const updates=[deferred(),deferred()]; let cursor=0;
    const location={cursor:'0',surface:'composer',project,worktree:'captured',main:true,git:false,git_status:'plain_directory',can_move:true};
    const globals={Git:class {init(){this.text={};this.repo={invalidate:async()=>{reads++;return false;}};this.invalidate();}},
      readScope:()=>({worktree:'captured'}),readLocation:()=>location,
      nextLocationChange:()=>updates[cursor++].promise,nextContributionEvent:()=>new Promise(()=>{}),
      publishContributions:states=>{published=states;}};
    const Context=vm.runInNewContext(`${contextSource}\nContext`,{...policy,...globals}),owner=new Context();
    const live={notify:()=>{},spawn:task=>task(live)};
    owner.init({},live);await owner.invalidate();await settle();
    assert.equal(reads,0);assert.deepEqual(Array.from(published,state=>state.visible),[false,false]);
    updates[0].resolve({...location,cursor:'1',project:'initialized',git:true,git_status:'repository'});await settle();
    assert.equal(reads,0);
    assert.deepEqual(Array.from(published,state=>state.visible),[true,false]);
    updates[1].resolve({...location,cursor:'2'});await settle();
    assert.equal(reads,0);
    assert.deepEqual(Array.from(published,state=>state.visible),[false,false]);
  }
});

test('hidden repository controls omit initial path labels',()=>{
  const path = `/isolated/${'workspace/'.repeat(24)}session`;
  const location = {surface:'composer',project:null,worktree:'captured-checkout',branch:path,
    main:true,git:false,git_status:'plain_directory',can_move:true};
  let published;
  const globals = {
    Git:class {init() {this.text = {git_detached:'Detached HEAD'};}},
    readScope:()=>({worktree:location.worktree}),
    readLocation:()=>location,
    publishContributions:states=>{
      for (const state of states) if (state.label) {
        assert(Buffer.byteLength(state.label.label) <= 128);
      }
      published=states;
    },
  };
  const Context=vm.runInNewContext(`${contextSource}\nContext`,{...policy,...globals});
  const owner=new Context();
  owner.init({}, {spawn:()=>{},notify:()=>{},focus_handle:()=>({focus(){}})});
  const branch = published.find(state=>state.id === 'branch');
  assert.equal(branch.visible,false);
  assert.equal(branch.label,undefined);
  assert.equal(owner.location.branch,path);
  assert.equal(owner.worktree,location.worktree);

  owner.location = {...location,project:'project',git:true,git_status:'repository',branch:'feature/visible-label'};
  owner.publish();
  const visible = published.find(state=>state.id === 'branch');
  assert.equal(visible.visible,true);
  assert.equal(visible.label.label,'feature/visible-label');
});

test('unknown inspection hides the branch',()=>{
  const state=contextFixture();
  state.owner.text.git_loading='Loading';
  state.owner.location={...state.owner.location,project:'project',git:false,git_status:'unknown'};
  state.owner.publish();
  let branch=state.published.at(-1).find(item=>item.id==='branch');
  assert.equal(branch.visible,false);assert.equal(branch.enabled,false);assert.equal(branch.loading,false);
  assert.equal(branch.label,undefined);
  state.owner.location={...state.owner.location,connected:false};state.owner.publish();
  assert.equal(state.published.at(-1).find(item=>item.id==='branch').visible,false);
  state.owner.location={...state.owner.location,connected:true,git:true,git_status:'repository'};state.owner.publish();
  branch=state.published.at(-1).find(item=>item.id==='branch');
  assert.equal(branch.visible,true);assert.equal(branch.enabled,true);assert.equal(branch.loading,false);
  state.owner.location={...state.owner.location,git:false,git_status:'plain_directory'};state.owner.publish();
  assert.equal(state.published.at(-1).find(item=>item.id==='branch').visible,false);
  state.owner.location={...state.owner.location,worktree:'different',git_status:'unknown'};state.owner.publish();
  assert.equal(state.published.at(-1).find(item=>item.id==='branch').visible,false);
});

test('repository notifications use a spawned context after initialization returns',async()=>{
  const discovery=deferred();
  const state=fixture(discovery);
  state.cx.notify=()=>{throw Error('initialization context expired');};
  state.owner.repo.notify();
  assert.equal(state.notifications(),1);
  state.changes[0].reject(Error('closed'));await settle();
});

test('event streams and actions receive the spawned context',async()=>{
  const state=fixture(deferred());let received,resolve;
  delete state.owner.watch;
  const ready=new Promise(done=>{resolve=done;});let reads=0;
  state.owner.watch(()=>reads++===0?ready:new Promise(()=>{}),(event,cx)=>{
    received=cx;
    state.owner.run(async cx=>{await Promise.resolve();cx.notify();},cx);
  },state.cx);
  state.cx.notify=()=>{throw Error('initialization context expired');};
  resolve({id:'git-refresh'});await settle();
  assert.equal(received,state.live);assert.equal(state.notifications(),3);
  state.changes[0].reject(Error('closed'));await settle();
});

test('history controls open the captured commit without shadowing the commit action',async()=>{
  const state=fixture(deferred()),commit='b'.repeat(40),requests=[];
  let commits=0;
  state.owner.actions.invoke=value=>{requests.push(value);};
  state.owner.actions.commit=()=>{commits++;};
  state.owner.control(`git-commit-${commit}`,state.live);
  await settle();
  assert.equal(JSON.stringify(requests),JSON.stringify([{kind:'open',request:{kind:'commit',commit}}]));
  state.owner.control('git-commit-action',state.live);
  await settle();
  assert.equal(commits,1);assert.equal(requests.length,1);
  state.changes[0].reject(Error('closed'));await settle();
});

test('native branch controls open the existing captured chooser only when loaded',async()=>{
  const state=fixture(deferred()),choices=[],items=[{id:'main'}];
  state.owner.actions.choose=(title,value)=>choices.push({title,value});
  state.owner.menus.checkout=()=>items;
  state.owner.repo.branches=null;
  state.owner.control('git-branch-menu',state.live);
  assert.equal(choices.length,0);
  state.owner.repo.branches={};
  state.owner.control('git-branch-menu',state.live);
  assert.deepEqual(choices,[{title:'git_checkout',value:items}]);
  state.changes[0].reject(Error('closed'));await settle();
});

test('closing the selected document enters the successor native leaf',()=>{
  const fallback=[],entries=[];
  const Git=vm.runInNewContext(`${source}\nGit`,{View:class {},focusEntry:id=>entries.push(id),
    collapseOpen:id=>id.endsWith('/hidden.txt') ? false : null,views:{defaultOpen:true}});
  const owner=new Git();
  owner.repo=new Repository({},()=>{});owner.contentFocus={focus:()=>fallback.push(true)};
  const documents=[
    {id:'first',request:{kind:'file'},files:[{path:'first.txt',text:'diff'}]},
    {id:'output',request:{kind:'output'},output:'captured output'},
    {id:'combined',request:{kind:'changes'},files:[{path:'hidden.txt',text:'diff'},{path:'visible.txt',text:'diff'}]},
  ];
  owner.repo.tabs=documents.map(document=>document.id);
  owner.repo.documents=new Map(documents.map(document=>[document.id,document]));
  owner.repo.selected='first';owner.current='tracked/first.txt';
  owner.closeDocument('first');assert.equal(owner.repo.selected,'output');
  assert.equal(owner.current,null);
  owner.closeDocument('output');assert.equal(owner.repo.selected,'combined');
  owner.focusDocument();
  assert.deepEqual(entries,['focus-git-output-output','focus-git-diff-combined/visible.txt','focus-git-diff-combined/visible.txt']);
  assert.equal(fallback.length,0);
  owner.closeDocument('combined');assert.equal(owner.repo.selected,null);
  assert.equal(entries.at(-1),null);
  assert.equal(fallback.length,0,'the last close does not steal focus');
});

test('binary successors keep the content keyboard scope without requesting a diff surface',()=>{
  const fallback=[],entries=[];
  const Git=vm.runInNewContext(`${source}\nGit`,{View:class {},focusEntry:id=>entries.push(id),
    collapseOpen:()=>null,views:{defaultOpen:true}});
  const owner=new Git();owner.contentFocus={focus:()=>fallback.push(true)};
  owner.repo={selected:'binary',documents:new Map([
    ['binary',{id:'binary',request:{kind:'file'},files:[{path:'image.png',text:'raw metadata',binary:true}]}],
    ['combined',{id:'combined',request:{kind:'changes'},files:[{path:'image.png',text:'raw metadata',binary:true}]}],
  ])};
  owner.focusDocument();owner.repo.selected='combined';owner.focusDocument();
  assert.equal(fallback.length,2);
  assert.deepEqual(entries,[null,null]);
});

test('folded text diffs keep the content keyboard scope',()=>{
  const entries=[],fallback=[];
  const Git=vm.runInNewContext(`${source}\nGit`,{View:class {},focusEntry:id=>entries.push(id),
    collapseOpen:()=>false,views:{defaultOpen:true}});
  const owner=new Git();owner.contentFocus={focus:()=>fallback.push(true)};
  owner.repo={selected:'combined',documents:new Map([
    ['combined',{id:'combined',request:{kind:'changes'},files:[{path:'notes.txt',text:'diff'}]}],
  ])};
  owner.focusDocument();assert.deepEqual(entries,[null]);assert.equal(fallback.length,1);
});

test('closing a background document does not change content focus',()=>{
  const Git=vm.runInNewContext(`${source}\nGit`,{View:class {}});
  const owner=new Git();owner.contentFocus={focus:()=>assert.fail('background close must not focus')};owner.repo=new Repository({},()=>{});
  owner.repo.tabs=['first','second'];owner.repo.documents=new Map(owner.repo.tabs.map(id=>[id,{id,request:{kind:'file'}}]));
  owner.repo.selected='second';owner.current='dir/tracked/src';
  owner.closeDocument('first');assert.equal(owner.repo.selected,'second');assert.equal(owner.current,'dir/tracked/src');
});

test('released controllers cannot recreate surfaces after late notifications',()=>{
  const state=fixture(deferred());
  state.owner.repo.documents.set('captured',{id:'captured',request:{kind:'file'},files:[{path:'a.txt',text:'@@ -1 +1 @@\n-old\n+new\n'}]});
  state.owner.repo.notify();
  assert.equal(state.owner.surfaces.states.size,1);
  state.owner.release();
  assert.equal(state.owner.surfaces.states.size,0);
  state.owner.repo.notify();
  assert.equal(state.owner.surfaces.states.size,0);
});

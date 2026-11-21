import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFile} from 'node:fs/promises';

const node=[1,...Array(31).fill(0)];
const sameNode=(a,b)=>JSON.stringify(a??node)===JSON.stringify(b??node);
const editor=(await readFile(new URL('../dev.sailry.platform/desktop/editor.js',import.meta.url),'utf8'))
  .replace(/^import[\s\S]*?;\n/gm,'').replace(/^export /gm,'');
const {lead,effortKey}=vm.runInNewContext(`${editor}\n({lead,effortKey})`);

async function fixture(packageName,overrides={}) {
  const source=(await readFile(new URL(`../../${packageName}/dev.sailry.platform/desktop/main.js`,import.meta.url),'utf8'))
    .replace(/^import[\s\S]*?;\n/gm,'').replace('export default class','class');
  const jobs=[],notices=[];
  let batch={items:[],after:null},failure=null,waiting=null;
  const read=async()=>{
    if(waiting)await waiting;
    if(failure)throw new Error('Injected read failure');
    return batch;
  };
  const name=packageName==='reminders'?'Reminders':'ScheduledTasks';
  const Controller=vm.runInNewContext(`${source}\n${name}`,{
    View:class{},lead,effortKey,sameNode,sdkContext:()=>({execution_node:node}),context:()=>JSON.stringify({locale:'en'}),messages:()=>new Proxy({},{get:(_,key)=>key}),
    list:async()=>({Ok:await read()}),page:read,
    history:async()=>({Ok:{items:[],next_before:null}}),states:async()=>({}),
    readResourceCatalog:async()=>({sources:[{node,projects:[{id:'project',name:'Project'}],worktrees:[]}]}),
    listModels:async()=>({models:[]}),listTeams:async()=>[],toast:notice=>notices.push(notice),
    ...overrides,
  });
  const view=new Controller(),cx={notify(){},spawn:job=>jobs.push(job)};
  view.init(null,cx);jobs.length=0;
  return {view,cx,notices,
    hold(){let release;waiting=new Promise(resolve=>{release=resolve;});return()=>{waiting=null;release();};},
    succeed(items){batch={items,after:null};failure=null;},
    fail(){failure=true;},
    async read(){view.refresh(cx);return jobs.shift()(cx);},
    async settle(){await Promise.all(jobs.splice(0).map(job=>job(cx)));},
  };
}

for(const packageName of ['scheduled-tasks','reminders']) {
  test(`${packageName} starts independent page reads before settling`,async()=>{
    const calls=[],held=[];
    const hold=(kind,value)=>()=>{calls.push(kind);return new Promise(resolve=>held.push(()=>resolve(value)));};
    const overrides={page:hold('records',{items:[{id:'saved'}],after:null}),list:hold('records',{Ok:{items:[{id:'saved'}],after:null}}),
      history:hold('history',{Ok:{items:[],next_before:null}}),states:hold('states',{}),
      readResourceCatalog:hold('projects',{sources:[{node,projects:[],worktrees:[]}]}),listModels:hold('models',{models:[]}),listTeams:hold('teams',[])};
    const setup=await fixture(packageName,overrides);
    setup.view.panel=packageName==='reminders'?{reminder:'saved'}:{task:'saved'};
    const loading=setup.read();
    assert.deepEqual(calls,packageName==='reminders'?['records','projects']:['records','history','states','projects','models','teams']);
    assert.equal(setup.view.loaded,false);assert.equal(setup.view.loading,true);
    for(let pass=0;pass<20;pass++){held.splice(0).forEach(resolve=>resolve());await Promise.resolve();}await loading;
    assert.equal(setup.view.loaded,true);assert.equal(setup.view.items[0].id,'saved');
    assert.equal(setup.view.historyLoaded,true);assert.deepEqual(setup.notices,[]);
  });

  test(`${packageName} confirms absence only after the initial read`,async()=>{
    const setup=await fixture(packageName),release=setup.hold();
    assert.equal(setup.view.loaded,false);assert.equal(setup.view.loading,false);
    const pending=setup.read();
    assert.equal(setup.view.loaded,false);assert.equal(setup.view.loading,true);
    release();await pending;
    assert.equal(setup.view.loaded,true);assert.equal(setup.view.loading,false);
    assert.equal(setup.view.items.length,0);assert.equal(setup.notices.length,0);
  });

  test(`${packageName} retains cards and drafts during a failed refresh`,async()=>{
    const setup=await fixture(packageName),item={id:'saved',title:'Reminder',name:'Task',due_ms:null};
    setup.succeed([item]);await setup.read();
    const items=setup.view.items,catalog=setup.view.catalog,draft={text:'Retained edit'};
    setup.view.editing={draft};
    const release=setup.hold();setup.fail();const pending=setup.read();
    assert.equal(setup.view.items,items);assert.equal(setup.view.loaded,true);
    assert.equal(setup.view.editing.draft,draft);
    release();await pending;
    assert.equal(setup.view.loading,false);assert.equal(setup.view.loaded,true);
    assert.equal(setup.view.items,items);assert.equal(setup.view.catalog,catalog);
    assert.equal(setup.view.editing.draft,draft);assert.equal(setup.view.error,'loadFailed');
    assert.equal(setup.notices.length,1);assert.equal(setup.notices[0].kind,'error');
    setup.succeed([]);await setup.read();
    assert.equal(setup.view.loaded,true);assert.equal(setup.view.items.length,0);
    assert.equal(setup.view.error,null);assert.equal(setup.view.editing.draft,draft);
  });

  test(`${packageName} does not confirm absence after a failed initial read`,async()=>{
    const setup=await fixture(packageName);setup.fail();await setup.read();
    assert.equal(setup.view.loaded,false);assert.equal(setup.view.loading,false);
    assert.equal(setup.view.error,'loadFailed');assert.equal(setup.notices.length,1);
  });
}

test('team catalog failures retain choices and task drafts',async()=>{
  let available=true;
  const teams=[{id:'reviewers',name:'Reviewers',revision:'7'}];
  const setup=await fixture('scheduled-tasks',{listTeams:async()=>{
    if(!available)throw new Error('Injected team failure');
    return teams;
  }});
  setup.succeed([{id:'saved'}]);await setup.read();
  const draft={execution:'team',team:{id:'reviewers',revision:null}};
  setup.view.editing={draft};available=false;await setup.read();
  assert.equal(setup.view.loaded,true);assert.equal(setup.view.items.length,1);
  assert.equal(setup.view.teams,teams);assert.equal(setup.view.teamsError,true);
  assert.equal(setup.view.editing.draft,draft);assert.equal(setup.notices.at(-1).message,'teamsFailed');
  available=true;await setup.read();
  assert.equal(setup.view.teamsError,false);assert.equal(setup.view.teams,teams);
  assert.equal(setup.view.editing.draft,draft);
});

test('refreshed fixed leads resolve while missing teams retain drafts',async()=>{
  let teams=[{id:'reviewers',model:{provider:'provider',model:'first',effort:'low'}}];
  const calls=[],setup=await fixture('scheduled-tasks',{listTeams:async()=>teams,
    resolveSessionModel:async(model,effort,base)=>{
      calls.push({model,effort,base});return {...base,provider:'provider',model:model.split('/')[1],effort};
    }});
  const config={provider:'provider',model:'fallback',effort:'high',options:{feature:{choice:true}}};
  const draft={execution:'team',team:{id:'reviewers',revision:null},config};
  setup.view.editing={draft,model:'provider/fallback',modelVersion:0,modelLoading:false,leadKey:null,leadError:false};
  await setup.read();await setup.settle();
  assert.equal(draft.config.model,'first');assert.equal(calls.length,1);assert.equal(calls[0].base,config);
  teams=[{id:'reviewers',model:{provider:'provider',model:'second',effort:'high'}}];
  await setup.read();await setup.settle();
  assert.equal(draft.config.model,'second');assert.equal(calls.length,2);
  const retained=draft.config;teams=[];await setup.read();await setup.settle();
  assert.equal(draft.config,retained);assert.equal(draft.team.id,'reviewers');assert.equal(calls.length,2);
  assert.equal(setup.view.editing.draft,draft);assert.deepEqual(draft.config.options,config.options);
});

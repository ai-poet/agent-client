import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFile} from 'node:fs/promises';
import {choices, selection} from '../dev.sailry.platform/desktop/access.js';
import {messages} from '../dev.sailry.platform/desktop/locales.js';

async function fixture() {
  const context=vm.createContext({}), prepared=[], completed=[], forgotten=[], states=[], panes=[];
  const host={result:{Err:{code:'outcome_unknown'}},items:[],open:false};
  const exports={
    'gpui-kit':{View:class {},div:()=>null},
    sailry:{context:()=>JSON.stringify({locale:'en'})},
    'sailry/sdk':{
      listTerminals:async()=>host.items,listTerminalTools:async()=>[],
      prepareTerminal:(tool,access)=>{const id=`request-${prepared.length}`;prepared.push({id,tool,access});return id;},
      completeRequest:async id=>{completed.push(id);return host.result;},
      forgetRequest:id=>forgotten.push(id),publishContributions:state=>states.push(state),
      nextContributionEvent(){},nextChange(){},
    },
    'sailry/ui':{openPane:entry=>{panes.push(JSON.parse(JSON.stringify(entry)));return host.open;},
      publishResourceTitles(){},toast(){},dismissToast(){},nextToastEvent(){}},
    './locales.js':{messages},'./labels.js':{title:()=>null,directory:()=>null},
    './access.js':{selection,choices},
  };
  const module=new vm.SourceTextModule(await readFile(new URL('../dev.sailry.platform/desktop/project.js',import.meta.url),'utf8'),{context});
  await module.link(name=>new vm.SyntheticModule(Object.keys(exports[name]),function(){
    for(const [key,value] of Object.entries(exports[name]))this.setExport(key,value);
  },{context}));
  await module.evaluate();
  const view=new module.namespace.default();
  view.init(null,{spawn(){}});
  Object.assign(view,{loading:false,ready:true,connected:true,tools:[{tool:'claude',available:true}]});
  const tasks=[],cx={notify(){},spawn(callback){tasks.push(Promise.resolve().then(()=>callback(cx)));}};
  let cursor=0;
  const settle=async()=>{while(cursor<tasks.length)await tasks[cursor++];};
  return {view,cx,settle,host,prepared,completed,forgotten,states,panes};
}

test('controls preserve explicit workspace access across uncertain receipts',async()=>{
  const setup=await fixture();
  setup.view.publish();
  const workspace=setup.states.at(-1).find(state=>state.id==='terminal-cli').choices
    .find(choice=>choice.id==='claude:workspace');
  assert.match(workspace.label.label,/Workspace/);
  assert.equal(workspace.group.label,'Workspace');
  assert.equal(workspace.enabled,true);
  setup.view.start('claude:workspace',null,setup.cx);await setup.settle();
  assert.deepEqual(setup.prepared,[{id:'request-0',tool:'claude',access:'workspace'}]);
  assert.equal(setup.view.pending.id,'request-0');
  assert.equal(setup.view.error,'unknown');
  assert.deepEqual(setup.forgotten,[]);
  const choices=setup.states.at(-1).find(state=>state.id==='terminal-cli').choices;
  assert.equal(choices.find(choice=>choice.id==='shell:workspace').enabled,false);
  setup.host.result={Ok:{kind:'terminal',data:{id:'workspace'}}};
  setup.view.start(null,null,setup.cx);await setup.settle();
  assert.equal(setup.prepared.length,1);
  assert.deepEqual(setup.completed,['request-0','request-0']);
  assert.deepEqual(setup.forgotten,['request-0']);
  assert.equal(setup.view.pending,null);
});

test('ordinary project creation defaults to full access',async()=>{
  const setup=await fixture();setup.host.result={Ok:{kind:'terminal',data:{id:'full'}}};
  setup.view.start(null,null,setup.cx);await setup.settle();
  assert.deepEqual(setup.prepared,[{id:'request-0',tool:null,access:'unrestricted'}]);
});

test('menus omit duplicate access choices and routine refresh',async()=>{
  const setup=await fixture();
  setup.view.publish();
  const menu=setup.states.at(-1).find(state=>state.id==='terminal-menu-cli');
  assert.deepEqual(Array.from(menu.choices,choice=>choice.id),['claude']);
  const toolbar=setup.states.at(-1).find(state=>state.id==='terminal-cli');
  assert.ok(toolbar.choices.some(choice=>choice.id==='claude:workspace'));
  assert.ok(toolbar.choices.some(choice=>choice.id==='shell:workspace'));
  assert.ok(toolbar.choices.some(choice=>choice.id==='refresh'));
  setup.view.start('claude',null,setup.cx);await setup.settle();
  const pending=setup.states.at(-1).find(state=>state.id==='terminal-menu-cli').choices;
  assert.deepEqual(Array.from(pending,choice=>choice.id),['claude','retry']);
  assert.equal(pending[0].enabled,false);
  assert.equal(pending[1].enabled,true);
});

test('creation retries retain their original pane interaction',async()=>{
  const setup=await fixture();
  setup.view.start(null,{id:'terminal-new',sequence:3},setup.cx);await setup.settle();
  assert.equal(setup.view.pending.id,'request-0');
  assert.equal(setup.view.pending.interaction,3);
  setup.host.result={Ok:{kind:'terminal',data:{id:'first'}}};
  setup.host.items=[{id:'first',status:{kind:'running'}}];
  setup.view.start(null,{id:'terminal-cli',sequence:4},setup.cx);await setup.settle();
  assert.equal(setup.prepared.length,1);
  assert.equal(setup.panes.at(-1).resource.id,'first');
  assert.equal(setup.panes.at(-1).interaction,3);
  setup.host.result={Ok:{kind:'terminal',data:{id:'second'}}};
  setup.host.items.push({id:'second',status:{kind:'running'}});
  setup.host.open=true;
  setup.view.start(null,{id:'terminal-new',sequence:5},setup.cx);await setup.settle();
  assert.equal(setup.prepared.length,2);
  assert.equal(setup.panes.at(-1).resource.id,'second');
  assert.equal(setup.panes.at(-1).interaction,5);
  assert.equal(setup.view.reveal,null);
  assert.ok(setup.panes.every(entry=>entry.resource.id==='first'
    ? entry.interaction===3 : entry.interaction===5));
});

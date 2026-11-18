import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';

const plain=value=>JSON.parse(JSON.stringify(value));
import {element,find,actions} from './elements.mjs';
const goal={id:'goal',revision:'5',objective:'Requested work',status:'active',state:'running',reason:null,detail:null,claim:null,evidence:null,
  turns:1,automatic_turns:1,turn_limit:25,tokens:120,token_budget:0,elapsed_ms:0,time_limit_ms:0};

async function fixture(options={}) {
  const host={goal:{...goal},history:[],hold:false,held:[],failure:null,readFailure:null,...options};
  const prepared=[],completed=[],forgotten=[],contributions=[],closed=[],notices=[],jobs=[],conversations=[];
  const listeners={change:null,composer:null,conversation:null,contribution:null},ready={},scopes=[];
  const next=kind=>{ready[kind]?.();return new Promise(resolve=>{listeners[kind]=resolve;});};
  async function emit(kind,value) {
    assert.ok(listeners[kind]);
    const first=jobs.length;
    const accepted=new Promise(resolve=>{ready[kind]=resolve;});
    const resolve=listeners[kind];listeners[kind]=null;resolve(kind==='change' ? {revision:value.cursor,...value} : value);
    await accepted;delete ready[kind];
    if(!host.hold) await Promise.all(jobs.slice(first));
  }
  const modules=new Map(),context=vm.createContext({});
  const sdk={
    nextChange:(_cursor,scope)=>{scopes.push(scope);return next('change');},
    nextConversation:(cursor,scope)=>{conversations.push({cursor,scope});return next('conversation');},
    nextContributionEvent:()=>next('contribution'),
    publishContributions:value=>contributions.push(plain(value)),
    closeContribution:id=>closed.push(id),
    prepareRequest:request=>{
      if (prepared.length-forgotten.length>=16) throw new Error('Host request capacity exceeded');
      prepared.push(plain(request));return String(prepared.length-1);
    },
    completeRequest:async id=>{
      completed.push(id);
      const request=prepared[Number(id)];
      if(request.data.handler==='inspect') {
        if(host.readFailure instanceof Error) throw host.readFailure;
        if(host.readFailure) return {Err:{code:host.readFailure,message:'Read failed'}};
        const result={Ok:{data:{goal:plain(host.goal && {...host.goal,...(host.shown ?? {})}),history:plain(host.history)}}};
        return host.hold ? new Promise(resolve=>host.held.push(()=>resolve(result))) : result;
      }
      assert.equal(request.data.handler,'control');
      if(host.failure) return {Err:{code:host.failure,message:'Unconfirmed action'}};
      const input=request.data.input,revision=String(BigInt(host.goal.revision)+1n);
      assert.equal(input.goal_id,host.goal.id);
      assert.deepEqual(Object.keys(input),['kind','goal_id']);
      host.goal=input.kind==='end' ? null
        : input.kind==='resume' ? {...host.goal,status:'active',state:'running',reason:null,revision}
        : {...host.goal,status:'paused',state:'paused',reason:'user',revision};
      return {Ok:{data:{goal:plain(host.goal),history:plain(host.history)}}};
    },
    forgetRequest:id=>forgotten.push(id),faultCode:message=>message,
  };
  const exports={
    'gpui-kit':{View:class{},div:()=>element()},
    'gpui-component':{Button:function(id){return element(id);}},
    sailry:{context:()=>JSON.stringify({locale:'en',session:'session',package:{name:'goals'}}),theme:()=>({colors:{}})},
    'sailry/sdk':sdk,
    'sailry/ui':{ScrollRegion:{new:element},
      readComposer:()=>({session:'session',connected:true,readonly:false,busy:false,mode:'code',cursor:'1'}),
      nextComposerChange:()=>next('composer'),toast:value=>{
        if(!['info','error'].includes(value.kind)) throw new TypeError('Unsupported notification kind');
        notices.push(plain(value));
      }},
  };
  async function load(name) {
    if(modules.has(name)) return modules.get(name);
    const module=exports[name] ? new vm.SyntheticModule(Object.keys(exports[name]),function(){
      for(const [key,value] of Object.entries(exports[name])) this.setExport(key,value);
    },{context}) : new vm.SourceTextModule(await readFile(
      new URL(name,new URL('../dev.sailry.platform/desktop/',import.meta.url)),'utf8'),{context,identifier:name});
    modules.set(name,module);
    if(!exports[name]) await module.link(load);
    return module;
  }
  const main=await load('./main.js');await main.evaluate();
  const view=new main.namespace.default();
  const cx={notify(){},spawn(callback){const job=Promise.resolve().then(()=>callback(cx));jobs.push(job);return job;}};
  view.init(null,cx);await jobs[0];
  async function perform(kind) {
    const first=jobs.length;
    view.perform(kind,cx);
    if(jobs.length===first) return;
    await jobs[first];
    if(!host.hold) await jobs.at(-1);
  }
  async function deliver(kind,value) {
    assert.ok(listeners[kind]);
    const resolve=listeners[kind];listeners[kind]=null;resolve(kind==='change' ? {revision:value.cursor,...value} : value);
    await new Promise(setImmediate);
  }
  return {view,host,cx,jobs,perform,prepared,completed,forgotten,contributions,closed,notices,emit,deliver,scopes,conversations};
}

test('unused conversations do not subscribe',async()=>{
  const setup=await fixture({goal:null});
  assert.equal(setup.view.goal,null);
  assert.equal(setup.prepared.length,1);
  assert.equal(setup.conversations.length,0);
  await setup.emit('composer',{...setup.view.state,cursor:'2',busy:true});
  assert.equal(setup.prepared.length,1);
  assert.equal(setup.conversations.length,0);
});

test('first goal activates run observations',async()=>{
  const setup=await fixture({goal:null});
  setup.host.goal={...goal,revision:'1'};
  await setup.deliver('change',{cursor:'2',connected:true});
  assert.equal(setup.view.goal.id,'goal');
  assert.equal(setup.conversations.length,1);
  assert.equal(setup.conversations[0].scope,'runs');
  setup.host.goal=null;
  await setup.deliver('change',{cursor:'3',connected:true});
  const count=setup.prepared.length;
  await setup.deliver('conversation',{cursor:'1',view:{snapshot:{page:{session:'session',runs:[]}}}});
  assert.equal(setup.conversations.length,1);
  assert.equal(setup.prepared.length,count);
});

test('retained sessions stop observing old history',async()=>{
  const setup=await fixture();
  assert.equal(setup.conversations.length,1);
  const count=setup.prepared.length;
  await setup.deliver('composer',{...setup.view.state,cursor:'2',session:'other'});
  await setup.deliver('conversation',{cursor:'1',view:{snapshot:{page:{session:'session',runs:[]}}}});
  assert.equal(setup.view.goal,null);
  assert.equal(setup.conversations.length,1);
  assert.equal(setup.prepared.length,count);
});

test('successful controls use supported notifications',async()=>{
  const setup=await fixture();
  await setup.perform('pause');
  assert.equal(setup.host.goal.status,'paused');
  await setup.perform('resume');
  assert.equal(setup.host.goal.status,'active');
  assert.deepEqual(setup.closed,[]);
  setup.view.open=true;
  await setup.perform('end');
  assert.equal(setup.host.goal,null);
  assert.equal(setup.view.goal,null);
  assert.equal(setup.view.pending,null);
  assert.equal(setup.view.open,false);
  assert.deepEqual(setup.closed,['status']);
  assert.ok(setup.contributions.every(entries=>entries.every(entry=>entry.id!=='status')));
  assert.deepEqual(setup.notices,[
    {id:'goal-action',kind:'info',message:setup.view.text.paused},
    {id:'goal-action',kind:'info',message:setup.view.text.resumed},
    {id:'goal-action',kind:'info',message:setup.view.text.ended},
  ]);
  const controls=setup.prepared.flatMap((request,index)=>request.data.handler==='control' ? [String(index)] : []);
  assert.equal(controls.length,3);
  for(const id of controls) {
    assert.equal(setup.completed.filter(value=>value===id).length,1);
    assert.equal(setup.forgotten.filter(value=>value===id).length,1);
  }
});

test('the objective remains literal prose with the controls of its state',async()=>{
  const setup=await fixture();
  const objective='## Requested **work** 中文 🙂\n\nKeep this paragraph and e\u0301 unchanged';
  setup.view.goal={...setup.view.goal,objective};
  const panel=setup.view.render();
  assert.deepEqual(find(panel,'goal-objective').text,[objective]);
  assert.deepEqual(actions(panel),['goal-pause','goal-end']);
});

test('forwarded status events refresh the panel',async()=>{
  const setup=await fixture();
  setup.host.goal={...setup.host.goal,status:'completed',state:'completed',objective:'Completed work',evidence:'Suite passed'};
  setup.view.readKey=null;
  await setup.emit('contribution',{id:'status',kind:'invoke',sequence:1,value:null});
  assert.equal(setup.view.open,true);
  assert.equal(setup.view.goal.status,'completed');
  assert.equal(setup.view.goal.objective,'Completed work');
  assert.deepEqual(actions(setup.view.render()),['goal-resume','goal-end']);
  await setup.emit('contribution',{id:'status',kind:'change',sequence:2,value:false});
  assert.equal(setup.view.open,false);
  assert.ok(setup.contributions.every(entries=>entries.every(entry=>entry.id!=='status')));
  assert.equal(setup.prepared.filter(request=>request.data.handler==='control').length,0);
  await setup.perform('end');
  assert.equal(setup.host.goal,null);
});

test('unsubmitted drafts never publish status',async()=>{
  const setup=await fixture({goal:null});
  setup.view.accept({...setup.view.state,cursor:'2',command:{name:'goal',package:{name:'goals'},arguments:''}});
  setup.view.publish();
  assert.equal(setup.view.draft(),'');
  assert.ok(setup.contributions.every(entries=>entries.every(entry=>entry.id!=='status')));
  assert.equal(setup.prepared.filter(request=>request.data.handler==='control').length,0);
});

test('a held refresh blocks controls until the latest read',async()=>{
  const setup=await fixture();
  assert.equal(setup.view.enabled(),true);
  setup.host.hold=true;
  await setup.perform('pause');
  assert.equal(setup.host.goal.revision,'6');
  assert.equal(setup.view.goal.revision,'5');
  assert.equal(setup.view.busy,false);
  assert.equal(setup.view.enabled(),true);
  assert.equal(setup.contributions.at(-1).find(item=>item.id==='goal').enabled,true);
  assert.ok(buttons(setup.view.render()).every(item=>item.disabledValue===true));
  const count=setup.prepared.length;
  await setup.perform('end');
  assert.equal(setup.prepared.length,count);
  assert.equal(setup.host.held.length,1);
  setup.host.hold=false;setup.host.held.shift()();await setup.jobs.at(-1);
  assert.equal(setup.view.goal.revision,'6');
  assert.equal(setup.view.enabled(),true);
  await setup.perform('end');
  // Controls name the goal, not its revision: the counters change every turn.
  const controls=setup.prepared.filter(request=>request.data.handler==='control');
  assert.deepEqual(controls.map(request=>request.data.input),[{kind:'pause',goal_id:'goal'},{kind:'end',goal_id:'goal'}]);
  assert.equal(setup.host.goal,null);
  assert.equal(setup.view.goal,null);
  assert.equal(setup.view.pending,null);
});

test('dirty refresh remains blocked until latest read',async()=>{
  const setup=await fixture();
  setup.host.hold=true;
  setup.view.refresh(setup.cx,true);
  await Promise.resolve();
  assert.equal(setup.view.enabled(),true);
  assert.ok(buttons(setup.view.render()).every(item=>item.disabledValue===true));
  setup.host.goal={...setup.host.goal,revision:'6',objective:'Changed elsewhere'};
  setup.view.refresh(setup.cx,true);
  setup.host.hold=false;setup.host.held.shift()();await setup.jobs.at(-1);
  assert.equal(setup.view.goal.revision,'6');
  assert.equal(setup.view.goal.objective,'Changed elsewhere');
  assert.equal(setup.view.enabled(),true);
  assert.equal(setup.prepared.filter(request=>request.data.handler==='inspect').length,3);
  assert.equal(setup.prepared.filter(request=>request.data.handler==='control').length,0);
});

test('refresh preserves new task command and draft',async()=>{
  const setup=await fixture();
  const argumentsText='New task 中文 🙂';
  setup.view.accept({...setup.view.state,cursor:'2',
    command:{name:'goal',package:{name:'goals'},arguments:argumentsText}});
  const first=setup.contributions.length;
  setup.host.hold=true;
  setup.view.refresh(setup.cx,true);await Promise.resolve();
  assert.equal(setup.view.draft(),argumentsText);
  assert.ok(setup.contributions.slice(first).every(entries=>entries.find(item=>item.id==='goal').enabled));
  assert.ok(buttons(setup.view.render()).every(item=>item.disabledValue===true));
  const count=setup.prepared.length;
  await setup.perform('end');
  assert.equal(setup.prepared.length,count);
  setup.host.hold=false;setup.host.held.shift()();await setup.jobs.at(-1);
  assert.equal(setup.view.draft(),argumentsText);
  assert.ok(setup.contributions.slice(first).every(entries=>entries.find(item=>item.id==='goal').enabled));
});

test('uncertain control retains exact request for retry',async()=>{
  const setup=await fixture();
  setup.view.open=true;
  setup.host.failure='outcome_unknown';
  await setup.perform('end');
  const id=setup.view.pending.id;
  assert.equal(setup.view.enabled(),false);
  assert.equal(setup.host.goal.revision,'5');
  assert.equal(setup.view.open,true);
  assert.deepEqual(setup.closed,[]);
  assert.ok(!setup.forgotten.includes(id));
  const count=setup.prepared.length;
  await setup.perform('pause');
  assert.equal(setup.prepared.length,count);
  setup.host.failure=null;
  const first=setup.jobs.length;
  setup.view.execute(setup.cx);await setup.jobs[first];await setup.jobs.at(-1);
  assert.equal(setup.prepared.filter(request=>request.data.handler==='control').length,1);
  assert.equal(setup.completed.filter(request=>request===id).length,2);
  assert.equal(setup.forgotten.filter(request=>request===id).length,1);
  assert.equal(setup.view.pending,null);
  assert.equal(setup.host.goal,null);
  assert.equal(setup.view.open,false);
  assert.deepEqual(setup.closed,['status']);
});

test('failed reads release capacity and preserve current goal',async()=>{
  const setup=await fixture(), previous=plain(setup.view.goal);
  for(let index=0;index<20;index++) {
    setup.host.readFailure=index%2 ? new Error('Read disconnected') : 'busy';
    setup.view.refresh(setup.cx,true);
    await setup.jobs.at(-1);
    assert.deepEqual(plain(setup.view.goal),previous);
    assert.equal(setup.prepared.length,setup.forgotten.length);
    assert.equal(setup.view.readError.code,index%2 ? 'Read disconnected' : 'busy');
    assert.equal(setup.view.readError.message,index%2 ? 'Read disconnected' : 'Read failed');
  }
  assert.equal(setup.notices.length,1);
  setup.host.readFailure=null;
  setup.view.refresh(setup.cx,true);await setup.jobs.at(-1);
  assert.equal(setup.view.readFailed,false);
  assert.equal(setup.view.readError,null);
  assert.equal(setup.prepared.length,22);
  assert.equal(setup.prepared.length,setup.forgotten.length);
  await setup.perform('pause');
  assert.equal(setup.host.goal.status,'paused');
  assert.equal(setup.view.pending,null);
});

test('reads stay nondurable while controls retain admission',async()=>{
  const setup=await fixture();
  assert.equal(setup.prepared[0].kind,'read_plugin_callback');
  assert.deepEqual(setup.scopes,['package']);
  await setup.emit('composer',{...setup.view.state,cursor:'2',draft:'changed',locale:'de'});
  assert.equal(setup.prepared.length,1);
  await setup.emit('composer',{...setup.view.state,cursor:'3',busy:true});
  assert.equal(setup.prepared.length,1);
  setup.host.goal={...setup.host.goal,revision:'6',objective:'Changed elsewhere'};
  await setup.emit('change',{cursor:'package-1',connected:true,entry:'0'});
  assert.equal(setup.view.goal.objective,'Changed elsewhere');
  assert.equal(setup.prepared.length,2);
  assert.ok(setup.prepared.every(request=>request.kind==='read_plugin_callback'));
  await setup.emit('composer',{...setup.view.state,cursor:'4',busy:false});
  await setup.perform('pause');
  const controls=setup.prepared.filter(request=>request.data.handler==='control');
  assert.equal(controls.length,1);
  assert.equal(controls[0].kind,'call_plugin');
  assert.equal(setup.view.pending,null);
});


test('external stops refresh without storage changes',async()=>{
  const setup=await fixture();
  const change=(cursor,status,session='session',text='')=>({cursor,view:{snapshot:{page:{session,
    runs:[{turn:'current-turn',status}],entries:[{text}]}}}});
  await setup.emit('conversation',change('1','running'));
  const before=setup.prepared.length;
  await setup.emit('conversation',change('2','running','session','Streaming text'));
  await setup.emit('conversation',change('3','cancelled','another-session'));
  assert.equal(setup.prepared.length,before);
  // A loop found dead is shown paused without a write to the goal.
  setup.host.shown={status:'paused',state:'paused',reason:'restart'};
  await setup.emit('conversation',change('4','cancelled'));
  assert.equal(setup.host.goal.status,'active');
  assert.equal(setup.view.goal.status,'paused');
  assert.equal(setup.prepared.length,before+1);
  assert.ok(setup.prepared.every(request=>request.kind==='read_plugin_callback'));
  await setup.deliver('conversation',change('5','cancelled','session','Unrelated output'));
  assert.equal(setup.prepared.length,before+1);
  assert.equal(setup.view.watching,false);
});


test('identical status triggers share a read',async()=>{
  const setup=await fixture();
  const change={cursor:'2',connected:true};
  await setup.deliver('change',change);
  const count=setup.prepared.length;
  await setup.deliver('change',change);
  setup.view.refresh(setup.cx);
  await new Promise(setImmediate);
  assert.equal(setup.prepared.length,count);
  await setup.deliver('composer',{...setup.view.state,cursor:'2',busy:true});
  assert.equal(setup.prepared.length,count);
  await setup.deliver('change',{cursor:'3',connected:true});
  assert.equal(setup.prepared.length,count+1);
});

const buttons=panel=>find(panel,'goal-actions').items;

test('/goal stays available in Plan mode',async()=>{
  const setup=await fixture();
  await setup.emit('composer',{...setup.view.state,cursor:'2',mode:'plan'});
  assert.equal(setup.view.enabled(),true);
  assert.equal(setup.contributions.at(-1).find(item=>item.id==='goal').enabled,true);
});

test('End closes the panel and says so once',async()=>{
  const setup=await fixture();
  setup.view.open=true;
  await setup.perform('end');
  assert.deepEqual(setup.notices.at(-1),{id:'goal-action',kind:'info',message:'Goal ended'});
  assert.deepEqual(setup.closed,['status']);
  assert.equal(setup.view.startEdit,undefined);
});

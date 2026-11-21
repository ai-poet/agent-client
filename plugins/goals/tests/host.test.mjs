import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';

const plain=value=>JSON.parse(JSON.stringify(value));
const names=['plugin_x_get_goal','plugin_x_create_goal','plugin_x_update_goal'];

// A small stand-in for the Node: two conversation values with revisions,
// atomic transactions, a run list and a continue_turn that can refuse.
async function fixture(options={}) {
  const values=new Map(),clock={now:1_000_000},scope={session:'session-id',turn:null};
  const session={revision:'3',config:{mode:options.mode ?? 'code'}};
  const host={runs:[],queue:'7',refuse:null,settings:{},continued:[],stopped:[],notified:[],submitted:[],unpaused:0,ids:0,state:null,conflict:false,race:null,background:null};
  const entry=key=>values.get(key) ?? {key,revision:'0',present:false,restored:false,value:null};
  const put=(key,value,restored=false)=>values.set(key,{key,revision:String(BigInt(entry(key).revision)+1n),present:true,restored,value:plain(value)});
  const requests=[];
  const sdk={
    context:()=>scope,newId:()=>`00000000-0000-4000-8000-${String(++host.ids).padStart(12,'0')}`,
    getConversationValue:async key=>plain(entry(key)),
    prepareTransaction:operations=>{requests.push(plain(operations));return String(requests.length-1);},
    completeRequest:async id=>{
      if (host.race) {const race=host.race;host.race=null;race();}
      const staged=new Map(values),effects={continued:[],stopped:[],notified:[],submitted:[],unpaused:0},outputs=[];
      const current=key=>staged.get(key) ?? {key,revision:'0',present:false,restored:false,value:null};
      for (const operation of requests[Number(id)]) {
        const data=operation.data;
        switch (operation.kind) {
        case 'conversation_write': case 'conversation_remove': {
          if (host.conflict || data.expected_revision!==current(data.key).revision) return {Err:{code:'revision_conflict',message:'plugin value changed'}};
          staged.set(data.key,{key:data.key,revision:String(BigInt(current(data.key).revision)+1n),restored:false,
            present:operation.kind==='conversation_write',value:operation.kind==='conversation_write' ? data.value : null});
          outputs.push({kind:'plugin_conversation_value',data:plain(staged.get(data.key))});break;
        }
        case 'continue':
          if (data.expected_revision!==current(data.key).revision) return {Err:{code:'revision_conflict',message:'plugin value changed'}};
          if (host.refuse) return {Err:{code:['queued_input','running','interaction','queue_paused'].includes(host.refuse) ? 'busy' : 'conflict',
            message:'refused',details:{kind:'continuation',reason:host.refuse}}};
          effects.continued.push(data);outputs.push({kind:'queued_turn',data:{id:'continuation'}});break;
        case 'stop': effects.stopped.push(data.turn);outputs.push({kind:'none'});break;
        case 'notify': effects.notified.push(data);outputs.push({kind:'none'});break;
        case 'submit': effects.submitted.push(data);outputs.push({kind:'queued_turn',data:{id:'submitted'}});break;
        case 'queue_pause': assert.equal(data.expected_revision,host.queue);assert.equal(data.paused,false);effects.unpaused++;outputs.push({kind:'none'});break;
        default: throw new Error(`Unexpected operation ${operation.kind}`);
        }
      }
      for (const [key,value] of staged) values.set(key,value);
      for (const key of ['continued','stopped','notified','submitted']) host[key].push(...effects[key]);
      host.unpaused+=effects.unpaused;
      return {Ok:{kind:'plugin_transaction',data:outputs}};
    },
    forgetRequest:()=>{},readSession:async()=>plain(session),
    readConversationRuns:async()=>({revision:1,runs:plain(host.runs),next_before:null,queue_revision:host.queue,
      ...(host.background ? {background:plain(host.background)} : {})}),
    readSettings:async()=>({values:plain(host.settings),ready:true}),
    readTurnState:()=>host.state,stageTurnState:value=>host.state=plain(value),
  };
  const context=vm.createContext({Date:{now:()=>clock.now}});
  const bridge=new vm.SyntheticModule(Object.keys(sdk),function(){
    for (const [name,value] of Object.entries(sdk)) this.setExport(name,value);
  },{context});
  const modules=new Map([['sailry/sdk',bridge]]);
  async function load(path,referrer) {
    if (modules.has(path)) return modules.get(path);
    const url=new URL(path,referrer?.identifier ?? new URL('../dev.sailry.platform/host/',import.meta.url));
    if (modules.has(url.href)) return modules.get(url.href);
    const module=new vm.SourceTextModule(await readFile(url,'utf8'),{context,identifier:url.href});
    modules.set(url.href,module);await module.link(load);return module;
  }
  const main=await load('./main.js');await main.evaluate();
  const api=main.namespace;
  let turns=0;
  // One ended turn as after_turn states it.
  const turn=(overrides={})=>({turn:`turn-${++turns}`,sequence:turns,status:'completed',mode:session.config.mode,automatic:false,notice:false,names,
    tool_calls:1,tools:{read_file:1},expired:0,usage:{input:100,output:20,cached_input:0,reasoning:0},
    session_usage:{input:0,output:0,cached_input:0,reasoning:0},background:{running:0,pending:0},chain:{count:0,limit:8},...overrides});
  const automatic=(count,overrides={})=>turn({automatic:true,continuation:{marker:null},chain:{count,limit:25},...overrides});
  // `/goal <text>` followed by the end of the user's own turn.
  async function started(text='Ship the feature and prove the tests pass') {
    await api.command({arguments:text,message:{text:`/goal ${text}`,references:[],attachments:[]}});
    await api.ended(turn());
    return goal();
  }
  const goal=()=>plain(entry('goal').value);
  const tool=(state,call)=>{host.state=state;return call();};
  return {api,host,clock,scope,session,entry,put,turn,automatic,started,goal,tool,history:()=>plain(entry('history').value?.items ?? [])};
}
const user={automatic:false,marker:null},loop={automatic:true,marker:null},audit={automatic:true,marker:'verify'};

test('the manifest declares three tools, both markers and the turn hooks',async()=>{
  const manifest=JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  const extension=manifest.extensions['dev.sailry.platform'];
  assert.equal(manifest.version,'0.2.4');assert.equal(extension.api_version,'v1');
  assert.deepEqual(extension.tools.map(tool=>tool.name),['get_goal','create_goal','update_goal']);
  assert.equal(extension.tools[1].handler.approval,true);
  assert.deepEqual(extension.host.turn,{initialize:'initialize',before_model:null,after_turn:'ended'});
  assert.deepEqual(extension.host.read_only,['read','badge','inspect']);
  assert.deepEqual(extension.host.status,{read:'badge',changes:['package','conversation']});
  assert.ok(extension.actions.includes('notifications.publish'));
  assert.equal(extension.continuation_marker.label,'Continue goal');
  assert.equal(extension.continuation_markers.verify.label,'Verify goal');
  assert.equal(extension.continuation_markers.verify.locales['zh-CN'],'核验目标');
  for (const name of [...extension.host.resources,...extension.desktop.resources,extension.settings_schema]) await readFile(new URL(`../${name}`,import.meta.url));
});

test('/goal starts at once, replaces a goal and plans in Plan mode',async t=>{
  await t.test('sets the goal and sends the text as the first turn',async()=>{
    const f=await fixture();
    const output=await f.api.command({arguments:'Ship it',message:{text:'/goal Ship it',references:[{id:'r'}],attachments:[]}});
    assert.equal(output.kind,'queued_turn');
    assert.deepEqual(f.host.submitted,[{session:'session-id',expected_revision:'3',message:{text:'Ship it',references:[{id:'r'}],attachments:[]}}]);
    assert.equal(f.goal().status,'active');assert.equal(f.goal().objective,'Ship it');
    assert.equal(f.host.continued.length,0);
  });
  await t.test('a new text replaces the goal and keeps the old as Replaced',async()=>{
    const f=await fixture();
    const first=await f.started('First goal');
    await f.api.command({arguments:'Second goal',message:{text:'/goal Second goal'}});
    assert.equal(f.goal().objective,'Second goal');assert.notEqual(f.goal().id,first.id);
    assert.deepEqual(f.history().map(item=>[item.objective,item.outcome]),[['First goal','replaced']]);
  });
  await t.test('Plan mode accepts the goal and holds pursuit',async()=>{
    const f=await fixture({mode:'plan'});
    await f.api.command({arguments:'Plan then build',message:{text:'/goal Plan then build'}});
    assert.equal(f.host.submitted.length,1);
    assert.deepEqual([f.goal().status,f.goal().reason],['paused','plan']);
    await f.api.ended(f.turn());
    assert.equal(f.host.continued.length,0);
    // The accepted plan runs a user turn in Code mode; pursuit starts after it.
    f.session.config.mode='code';
    await f.api.ended(f.turn());
    assert.equal(f.goal().status,'active');assert.equal(f.host.continued.length,1);
  });
  await t.test('/goal alone opens the panel and controls take their word alone',async()=>{
    const f=await fixture();
    assert.equal((await f.api.command({arguments:'',message:{text:'/goal'}})).ui_intent,'goal');
    await f.started();
    await f.api.command({arguments:'pause',message:{text:'/goal pause'}});
    assert.deepEqual([f.goal().status,f.goal().reason],['paused','user']);
    await f.api.command({arguments:'resume',message:{text:'/goal resume'}});
    assert.equal(f.goal().status,'active');
    await f.api.command({arguments:'end',message:{text:'/goal end'}});
    assert.equal(f.entry('goal').present,false);
    await assert.rejects(f.api.command({arguments:'pause',message:{text:'/goal pause'}}),{code:'not_found'});
    // Edit is not a control: the word begins a new goal, as any text does,
    // and so does a control's word followed by more.
    await f.api.command({arguments:'edit the README until it is accurate',message:{text:'/goal edit the README until it is accurate'}});
    assert.equal(f.goal().objective,'edit the README until it is accurate');
    const first=f.goal().id;
    await f.api.command({arguments:'end the flaky test for good',message:{text:'/goal end the flaky test for good'}});
    assert.deepEqual([f.goal().objective,f.goal().id===first,f.history()[0].outcome],['end the flaky test for good',false,'replaced']);
    // The earlier word still ends the goal.
    await f.api.command({arguments:'clear',message:{text:'/goal clear'}});
    assert.deepEqual([f.entry('goal').present,f.history()[0].outcome],[false,'cleared']);
  });
});

test('the loop continues, waits and resumes',async t=>{
  await t.test('a completed turn admits one fenced continuation with the goal limit',async()=>{
    const f=await fixture();
    await f.started();
    assert.equal(f.host.continued.length,1);
    const sent=f.host.continued[0];
    assert.equal(sent.after,'turn-1');assert.equal(sent.limit,25);assert.equal(sent.marker,undefined);assert.equal(sent.user,undefined);
    assert.equal(sent.key,'goal');assert.equal(sent.scope,'conversation');assert.equal(sent.expected_revision,f.entry('goal').revision);
    assert.match(sent.message.text,/automatic turn 1 of at most 25/);
    assert.deepEqual([f.goal().turns,f.goal().tokens,f.goal().seq],[1,120,1]);
    // A replayed callback, or one of an older turn, writes nothing.
    const revision=f.entry('goal').revision;
    await f.api.ended({...f.turn(),turn:'turn-1',sequence:1});
    assert.equal(f.host.continued.length,1);assert.equal(f.entry('goal').revision,revision);
  });
  await t.test('the settings set the limit and the budgets',async()=>{
    const f=await fixture();
    f.host.settings={turn_limit:40,token_budget:500,time_limit:90};
    await f.started();
    assert.equal(f.host.continued[0].limit,40);
    assert.match(f.host.continued[0].message.text,/Tokens used: 120 of 500000\./);
    assert.match(f.host.continued[0].message.text,/Time used: 0 of 90 minutes\./);
  });
  await t.test('running background work is waited for, and its notice turn continues',async()=>{
    const f=await fixture();
    await f.started();
    await f.api.ended(f.automatic(1,{background:{running:1,pending:0}}));
    assert.equal(f.host.continued.length,1);assert.equal(f.goal().waiting,true);assert.equal(f.goal().status,'active');
    assert.equal(plain(await f.api.badge())[0].segments[0].label.label,'Waiting');
    await f.api.ended(f.turn({automatic:true,notice:true,chain:{count:2,limit:25}}));
    assert.equal(f.host.continued.length,2);assert.equal(f.goal().waiting,false);
    assert.match(f.host.continued[1].message.text,/automatic turn 3 of at most 25/);
  });
  await t.test('a queued, running or asking turn is waited for',async()=>{
    for (const reason of ['queued_input','running','interaction']) {
      const f=await fixture();
      f.host.refuse=reason;
      await f.started();
      assert.deepEqual([f.goal().status,f.goal().waiting,f.host.continued.length,f.host.notified.length],['active',true,0,0],reason);
      // The user's message runs, and the loop continues after it.
      f.host.refuse=null;
      await f.api.ended(f.turn());
      assert.equal(f.host.continued.length,1);
    }
  });
  await t.test('a user message during a goal turn is guidance; the loop continues',async()=>{
    const f=await fixture();
    await f.started();
    // Joined guidance resets the automatic chain; the turn stays automatic.
    await f.api.ended(f.automatic(0));
    assert.equal(f.host.continued.length,2);assert.equal(f.goal().status,'active');
    assert.match(f.host.continued[1].message.text,/automatic turn 1 of at most 25/);
  });
  await t.test('a turn that hands over to its successor is not an end',async()=>{
    const f=await fixture();
    await f.started();
    await f.api.ended(f.automatic(1,{status:'continued'}));
    assert.deepEqual([f.host.continued.length,f.goal().turns,f.goal().tokens],[1,1,240]);
  });
  await t.test('a lost race decides again from the newer goal',async()=>{
    const f=await fixture();
    await f.started();
    // The user pauses between the callback's read and its commit.
    f.host.race=()=>f.put('goal',{...f.goal(),status:'paused',reason:'user',since_ms:null});
    assert.deepEqual(plain(await f.api.ended(f.automatic(1))),{});
    assert.deepEqual([f.goal().status,f.goal().reason,f.host.continued.length],['paused','user',1]);
    // A goal that keeps changing fails the callback instead of looping.
    const busy=await fixture();
    await busy.started();
    busy.host.conflict=true;
    await assert.rejects(busy.api.ended(busy.automatic(1)),{code:'revision_conflict'});
  });
  await t.test('overlapping callbacks leave the newer turn in charge',async()=>{
    const f=await fixture();
    await f.started();
    const older=f.automatic(1),newer=f.turn();
    // The user's queued turn ended and its callback ran before the older one.
    await f.api.ended(newer);
    assert.equal(f.host.continued.length,2);
    const revision=f.entry('goal').revision;
    await f.api.ended(older);
    assert.equal(f.entry('goal').revision,revision);assert.equal(f.host.continued.length,2);
  });
});

test('completion is a claim and then an audit',async t=>{
  await t.test('a claim starts the verification turn, which closes the goal',async()=>{
    const f=await fixture();
    await f.started();
    const claimed=await f.tool(loop,()=>f.api.update({status:'complete',note:'All tests pass'}));
    assert.equal(claimed.goal.status,'active');assert.equal(claimed.goal.state,'verifying');assert.match(claimed.next,/verification turn follows/);
    // A second claim in the same turn still closes nothing.
    await f.tool(loop,()=>f.api.update({status:'complete',note:'All tests pass, checked twice'}));
    assert.equal(f.goal().status,'active');
    await f.api.ended(f.automatic(1));
    const sent=f.host.continued.at(-1);
    assert.equal(sent.marker,'verify');assert.match(sent.message.text,/<claim>\nAll tests pass, checked twice\n<\/claim>/);
    assert.match(sent.message.text,/Treat the claim as unproven/);
    const closed=await f.tool(audit,()=>f.api.update({status:'complete',note:'Ran the suite: 42 passed'}));
    assert.equal(closed.goal.status,'completed');assert.equal(closed.goal.evidence,'Ran the suite: 42 passed');
    assert.equal(f.host.notified.length,0);
    await f.api.ended(f.automatic(2,{continuation:{marker:'verify'}}));
    assert.equal(f.host.continued.length,2);
    assert.deepEqual(f.host.notified.map(notice=>[notice.kind,notice.presentation.heading,notice.presentation.locales['zh-CN'],notice.title,notice.message,notice.session]),
      [['success','Goal completed','目标已完成','Ship the feature and prove the tests pass','Ran the suite: 42 passed','session-id']]);
    // Later turns neither continue nor notify again.
    await f.api.ended(f.turn());
    assert.deepEqual([f.host.continued.length,f.host.notified.length,f.goal().turns],[2,1,3]);
  });
  await t.test('an unconfirmed audit drops the claim and work goes on',async()=>{
    const f=await fixture();
    await f.started();
    await f.tool(loop,()=>f.api.update({status:'complete',note:'Probably done'}));
    await f.api.ended(f.automatic(1));
    await f.api.ended(f.automatic(2,{continuation:{marker:'verify'}}));
    assert.equal(f.goal().claim,null);assert.equal(f.goal().status,'active');
    const sent=f.host.continued.at(-1);
    assert.equal(sent.marker,undefined);assert.match(sent.message.text,/did not confirm completion/);
  });
  await t.test('a claim needs evidence and an active goal',async()=>{
    const f=await fixture();
    await f.started();
    await assert.rejects(f.tool(loop,()=>f.api.update({status:'complete'})),/evidence/);
    await assert.rejects(f.tool(loop,()=>f.api.update({status:'complete',note:'x',objective:'smaller'})),/Invalid goal arguments/);
    await f.api.control({kind:'pause',goal_id:f.goal().id});
    await assert.rejects(f.tool(user,()=>f.api.update({status:'complete',note:'done'})),{code:'conflict'});
  });
  await t.test('Reopen makes the same goal active again with its totals',async()=>{
    const f=await fixture();
    const before=await f.started();
    await f.tool(loop,()=>f.api.update({status:'complete',note:'claimed'}));
    await f.api.ended(f.automatic(1));
    await f.tool(audit,()=>f.api.update({status:'complete',note:'verified'}));
    await f.api.ended(f.automatic(2,{continuation:{marker:'verify'}}));
    const view=await f.api.control({kind:'resume',goal_id:before.id});
    assert.equal(view.goal.status,'active');assert.equal(f.goal().id,before.id);assert.equal(f.goal().turns,3);assert.equal(f.goal().evidence,null);
    assert.equal(f.host.continued.at(-1).user,true);
    assert.deepEqual(f.history(),[]);
  });
});

test('Stop pauses and a reply resumes; Pause holds until Resume',async t=>{
  await t.test('a stopped goal turn pauses silently and the next message resumes',async()=>{
    const f=await fixture();
    await f.started();
    await f.api.ended(f.automatic(1,{status:'cancelled'}));
    assert.deepEqual([f.goal().status,f.goal().reason,f.host.notified.length,f.host.continued.length],['paused','stopped',0,1]);
    const instruction=(await f.api.initialize({tools:['get_goal','create_goal','update_goal'],mode:'code',automatic:false,notice:false})).instruction;
    assert.match(instruction,/resumes automatically after this turn/);
    await f.api.ended(f.turn());
    assert.equal(f.goal().status,'active');assert.equal(f.host.continued.length,2);
    assert.equal(f.host.continued[1].user,undefined);
  });
  await t.test('a stopped reply leaves the goal paused',async()=>{
    const f=await fixture();
    await f.started();
    await f.api.ended(f.automatic(1,{status:'cancelled'}));
    await f.api.ended(f.turn({status:'cancelled'}));
    assert.deepEqual([f.goal().status,f.goal().reason,f.host.continued.length],['paused','stopped',1]);
  });
  await t.test('Pause stops the running goal turn and holds across replies',async()=>{
    const f=await fixture();
    await f.started();
    f.host.runs=[{turn:'running',sequence:2,status:'running',origin:null,error_code:null},{turn:'turn-1',sequence:1,status:'completed',origin:null,error_code:null}];
    const view=await f.api.control({kind:'pause',goal_id:f.goal().id});
    assert.deepEqual([view.goal.status,view.goal.reason],['paused','user']);
    assert.deepEqual(f.host.stopped,['running']);
    await f.api.ended(f.automatic(1,{status:'cancelled'}));
    await f.api.ended(f.turn());
    await f.api.ended(f.turn());
    assert.deepEqual([f.goal().status,f.goal().reason,f.host.continued.length,f.host.notified.length],['paused','user',1,0]);
    const instruction=(await f.api.initialize({tools:['get_goal','create_goal','update_goal'],mode:'code',automatic:false,notice:false})).instruction;
    assert.match(instruction,/Do not work on it now/);
  });
  await t.test('Resume starts a chain the user asked for and resumes the queue',async()=>{
    const f=await fixture();
    await f.started();
    await f.api.control({kind:'pause',goal_id:f.goal().id});
    const view=await f.api.control({kind:'resume',goal_id:f.goal().id});
    assert.equal(view.goal.status,'active');assert.equal(f.host.unpaused,1);
    const sent=f.host.continued.at(-1);
    assert.equal(sent.user,true);assert.equal(sent.after,null);assert.equal(sent.limit,25);
    assert.match(sent.message.text,/The user resumed the goal/);
    await assert.rejects(f.api.control({kind:'resume',goal_id:f.goal().id}),{code:'conflict'});
    await assert.rejects(f.api.control({kind:'pause',goal_id:'another'}),{code:'conflict'});
  });
  await t.test('Resume during a turn leaves the continuation to that turn',async()=>{
    const f=await fixture();
    await f.started();
    await f.api.control({kind:'pause',goal_id:f.goal().id});
    f.host.refuse='running';
    await f.api.control({kind:'resume',goal_id:f.goal().id});
    assert.deepEqual([f.goal().status,f.goal().waiting,f.host.continued.length,f.host.unpaused],['active',true,1,1]);
  });
  await t.test('Resume preserves paused reply accounting in either callback order',async()=>{
    for (const callbackFirst of [true,false]) {
      const f=await fixture();
      await f.started();
      await f.api.control({kind:'pause',goal_id:f.goal().id});
      await f.api.ended(f.automatic(1,{status:'cancelled'}));
      const reply=f.turn(),before=f.goal();
      f.host.runs=[{turn:reply.turn,sequence:reply.sequence,status:'completed',origin:null,error_code:null}];
      if (callbackFirst) await f.api.ended(reply);
      await f.api.control({kind:'resume',goal_id:f.goal().id});
      const revision=f.entry('goal').revision;
      if (!callbackFirst) await f.api.ended(reply);
      assert.equal(f.entry('goal').revision,revision,'a held reply must not withdraw the resumed continuation');
      assert.deepEqual([f.goal().turns,f.goal().tokens,f.goal().window_tokens],[before.turns,before.tokens,before.window_tokens]);
      assert.equal(f.goal().held_through,reply.sequence);
      assert.equal(f.host.continued.at(-1).user,true);
      await f.api.ended(f.automatic(1));
      assert.deepEqual([f.goal().turns,f.goal().tokens],[before.turns+1,before.tokens+120]);
    }
  });
  await t.test('Resume still counts a reply that remains in flight',async()=>{
    const f=await fixture();
    await f.started();
    await f.api.control({kind:'pause',goal_id:f.goal().id});
    await f.api.ended(f.automatic(1,{status:'cancelled'}));
    const reply=f.turn(),before=f.goal();
    f.host.runs=[{turn:reply.turn,sequence:reply.sequence,status:'running',origin:null,error_code:null}];
    f.host.refuse='running';
    await f.api.control({kind:'resume',goal_id:f.goal().id});
    assert.equal(f.goal().waiting,true);
    f.host.refuse=null;
    await f.api.ended(reply);
    assert.deepEqual([f.goal().turns,f.goal().tokens,f.goal().waiting],[before.turns+1,before.tokens+120,false]);
    assert.equal(f.host.continued.length,2);
  });
});

test('the model cannot pause, resume or end on its own',async t=>{
  const f=await fixture();
  await f.started();
  await t.test('pause and resume need a turn the user started',async()=>{
    for (const state of [loop,audit,null]) {
      await assert.rejects(f.tool(state,()=>f.api.update({status:'paused'})),{code:'permission_denied'});
    }
    await f.tool(user,()=>f.api.update({status:'paused'}));
    assert.deepEqual([f.goal().status,f.goal().reason],['paused','user']);
    // A notice turn is automatic: the model cannot undo the user's pause in it.
    await assert.rejects(f.tool(loop,()=>f.api.update({status:'active'})),{code:'permission_denied'});
    assert.equal(f.goal().status,'paused');
    await f.tool(user,()=>f.api.update({status:'active'}));
    assert.equal(f.goal().status,'active');
    // The user's own turn then continues the goal.
    await f.api.ended(f.turn());
    assert.equal(f.host.continued.length,2);
  });
  await t.test('a goal starts only on request and never replaces one',async()=>{
    await assert.rejects(f.tool(user,()=>f.api.create({objective:'Another'})),{code:'conflict'});
    const empty=await fixture();
    await assert.rejects(empty.tool(loop,()=>empty.api.create({objective:'Self-assigned'})),{code:'permission_denied'});
    const created=await empty.tool(user,()=>empty.api.create({objective:'Asked for'}));
    assert.equal(created.goal.status,'active');assert.equal(created.goal.revision,'1');
    await empty.api.ended(empty.turn());
    assert.equal(empty.host.continued.length,1);
  });
  await t.test('a completed goal is reopened only by the user',async()=>{
    const done=await fixture();
    await done.started();
    await done.tool(loop,()=>done.api.update({status:'complete',note:'claimed'}));
    await done.api.ended(done.automatic(1));
    await done.tool(audit,()=>done.api.update({status:'complete',note:'verified'}));
    await assert.rejects(done.tool(user,()=>done.api.update({status:'active'})),{code:'permission_denied'});
    // After completion the model may start the next goal the user asks for.
    await done.tool(user,()=>done.api.create({objective:'Next'}));
    assert.deepEqual(done.history().map(item=>[item.outcome,item.evidence]),[['completed','verified']]);
  });
});

test('limits wrap up and pause with their reason',async t=>{
  await t.test('the last allowed turn wraps up, then the goal pauses and notifies',async()=>{
    const f=await fixture();
    f.host.settings={turn_limit:3};
    await f.started();
    await f.api.ended(f.automatic(1));
    assert.equal(f.goal().wrap,null);
    await f.api.ended(f.automatic(2));
    assert.equal(f.goal().wrap,'turns');
    assert.match(f.host.continued.at(-1).message.text,/last automatic turn before the goal pauses, because the automatic turn limit is reached/);
    await f.api.ended(f.automatic(3));
    assert.deepEqual([f.goal().status,f.goal().reason,f.host.continued.length],['paused','turns',3]);
    assert.deepEqual(f.host.notified.map(notice=>[notice.kind,notice.presentation.heading,notice.presentation.locales['zh-CN']]),
      [['warning','Goal paused · Turn limit reached','目标已暂停 · 已达轮次上限']]);
    // The user's next message opens a new window.
    await f.api.ended(f.turn());
    assert.equal(f.goal().status,'active');assert.equal(f.host.continued.length,4);
  });
  await t.test('a refusal at the core limit pauses the same way',async()=>{
    const f=await fixture();
    await f.started();
    f.host.refuse='limit';
    await f.api.ended(f.automatic(5));
    assert.deepEqual([f.goal().status,f.goal().reason,f.host.notified.length],['paused','turns',1]);
  });
  await t.test('notice turns that reach the limit pause without another turn',async()=>{
    const f=await fixture();
    f.host.settings={turn_limit:3};
    await f.started();
    await f.api.ended(f.turn({automatic:true,notice:true,chain:{count:3,limit:3}}));
    assert.deepEqual([f.goal().status,f.goal().reason,f.host.continued.length],['paused','turns',1]);
  });
  await t.test('the token budget wraps up once and is granted again on resume',async()=>{
    const f=await fixture();
    f.host.settings={token_budget:1};
    await f.started();
    await f.api.ended(f.automatic(1,{usage:{input:900,output:100,cached_input:0,reasoning:0}}));
    assert.equal(f.goal().wrap,'tokens');
    assert.match(f.host.continued.at(-1).message.text,/because the token budget is reached/);
    await f.api.ended(f.automatic(2));
    assert.deepEqual([f.goal().status,f.goal().reason],['paused','tokens']);
    await f.api.control({kind:'resume',goal_id:f.goal().id});
    assert.deepEqual([f.goal().status,f.goal().window_tokens,f.goal().tokens],['active',0,1240]);
  });
  await t.test('the time limit counts active time',async()=>{
    const f=await fixture();
    f.host.settings={time_limit:10};
    await f.started();
    f.clock.now+=11*60000;
    await f.api.ended(f.automatic(1));
    assert.equal(f.goal().wrap,'time');
    await f.api.ended(f.automatic(2));
    assert.deepEqual([f.goal().status,f.goal().reason],['paused','time']);
    // Paused time is not counted.
    const elapsed=f.goal().elapsed_ms;
    f.clock.now+=60*60000;
    assert.equal((await f.api.inspect()).goal.elapsed_ms,elapsed);
  });
  await t.test('two automatic turns without work pause as no progress',async()=>{
    const f=await fixture();
    await f.started();
    await f.api.ended(f.automatic(1,{tool_calls:0,tools:{}}));
    assert.deepEqual([f.goal().status,f.goal().idle],['active',1]);
    // The package's own tools are not progress.
    await f.api.ended(f.automatic(2,{tool_calls:2,tools:{plugin_x_get_goal:2}}));
    assert.deepEqual([f.goal().status,f.goal().reason],['paused','no_progress']);
    assert.equal(f.host.notified.at(-1).presentation.heading,'Goal paused · No progress');
  });
  await t.test('work, a user turn or pending results reset the no-progress count',async()=>{
    const f=await fixture();
    await f.started();
    await f.api.ended(f.automatic(1,{tool_calls:0,tools:{}}));
    await f.api.ended(f.automatic(2));
    assert.equal(f.goal().idle,0);
    await f.api.ended(f.automatic(3,{tool_calls:0,tools:{}}));
    await f.api.ended(f.automatic(4,{tool_calls:0,tools:{},background:{running:0,pending:1}}));
    assert.deepEqual([f.goal().status,f.goal().idle],['active',0]);
  });
});

test('every other pause names its reason',async t=>{
  const cases=[
    // The Node notifies of the failed run; the goal adds no second one.
    ['a failed turn',{status:'failed',error:'not_configured'},'error','not_configured',0],
    ['a turn the Node lost',{status:'interrupted'},'restart',null,0],
    ['an approval or question that expired',{expired:1},'needs_you',null,1],
  ];
  for (const [name,facts,reason,detail,notices] of cases) await t.test(name,async()=>{
    const f=await fixture();
    await f.started();
    await f.api.ended(f.automatic(1,facts));
    assert.deepEqual([f.goal().status,f.goal().reason,f.goal().detail,f.host.notified.length,f.host.continued.length],['paused',reason,detail,notices,1]);
  });
  for (const [refuse,reason,notices] of [['plan','plan',0],['queue_paused','stopped',0],['worktree','error',1],['archived','error',1]]) {
    await t.test(`a continuation refused for ${refuse}`,async()=>{
      const f=await fixture();
      f.host.refuse=refuse;
      await f.started();
      assert.deepEqual([f.goal().status,f.goal().reason,f.host.notified.length],['paused',reason,notices]);
    });
  }
  await t.test('a stale source leaves the next step to the later turn',async()=>{
    const f=await fixture();
    f.host.refuse='stale';
    await f.started();
    assert.deepEqual([f.goal().status,f.goal().seq],['active',0]);
  });
  await t.test('entering Plan mode holds the goal until a turn runs outside it',async()=>{
    const f=await fixture();
    await f.started();
    await f.api.ended(f.turn({mode:'plan'}));
    assert.deepEqual([f.goal().status,f.goal().reason,f.host.notified.length],['paused','plan',0]);
    await f.api.ended(f.turn({mode:'plan'}));
    assert.equal(f.goal().status,'paused');
    await f.api.ended(f.turn());
    assert.equal(f.goal().status,'active');
    // Resume in Plan mode does not run the goal either.
    await f.api.control({kind:'pause',goal_id:f.goal().id});
    f.session.config.mode='plan';
    const view=await f.api.control({kind:'resume',goal_id:f.goal().id});
    assert.deepEqual([view.goal.status,view.goal.reason,f.host.continued.length],['paused','plan',2]);
  });
  await t.test('blocked needs a note, notifies once and ends with the next message',async()=>{
    const f=await fixture();
    await f.started();
    await assert.rejects(f.tool(loop,()=>f.api.update({status:'blocked'})),/what is needed/);
    await f.tool(loop,()=>f.api.update({status:'blocked',note:'Need the production API key'}));
    await f.api.ended(f.automatic(1));
    assert.deepEqual([f.goal().status,f.goal().detail,f.host.continued.length],['blocked','Need the production API key',1]);
    assert.deepEqual(f.host.notified.map(notice=>[notice.kind,notice.presentation.heading,notice.message]),[['warning','Goal blocked','Need the production API key']]);
    await f.api.ended(f.turn());
    assert.equal(f.goal().status,'active');assert.equal(f.goal().detail,null);
  });
});

test('restart, fork and rewind leave the goal paused',async t=>{
  await t.test('a goal whose last turn was lost is shown paused; Resume continues',async()=>{
    for (const [run,reason] of [[{status:'interrupted',error_code:null},'restart'],[{status:'cancelled',error_code:'outcome_unknown'},'restart'],
      [{status:'cancelled',error_code:'conflict'},'stopped'],[{status:'failed',error_code:'internal'},'error']]) {
      const f=await fixture();
      await f.started();
      f.host.runs=[{turn:'lost',sequence:2,origin:null,...run},{turn:'turn-1',sequence:1,status:'completed',origin:null,error_code:null}];
      const shown=(await f.api.inspect()).goal;
      assert.deepEqual([shown.status,shown.reason,f.goal().status],['paused',reason,'active']);
      assert.deepEqual(plain(await f.api.badge())[0].segments.map(segment=>segment.label.label),['Paused',{restart:'App restarted',stopped:'Stopped',error:'Error'}[reason]]);
      await f.api.control({kind:'resume',goal_id:f.goal().id});
      assert.equal(f.host.continued.at(-1).user,true);
    }
  });
  await t.test('a turn that ended before the Node started leaves the goal paused',async()=>{
    // The turn completed as the Node stopped, so its after_turn never came.
    const f=await fixture();
    await f.started();
    f.host.runs=[{turn:'unheard',sequence:2,status:'completed',origin:null,error_code:null,recovered:true},
      {turn:'turn-1',sequence:1,status:'completed',origin:null,error_code:null,recovered:true}];
    const shown=(await f.api.inspect()).goal;
    assert.deepEqual([shown.status,shown.reason,f.goal().status],['paused','restart','active']);
    await f.api.control({kind:'resume',goal_id:f.goal().id});
    assert.equal(f.host.continued.at(-1).user,true);
    // The same turn heard in this process is a running loop.
    const live=await fixture();
    await live.started();
    live.host.runs=[{turn:'turn-2',sequence:2,status:'completed',origin:null,error_code:null}];
    assert.deepEqual([(await live.api.inspect()).goal.status,(await live.api.inspect()).goal.state],['active','running']);
  });
  await t.test('unreported background work leaves a waiting goal paused',async()=>{
    const waiting=async chain=>{
      const f=await fixture();
      await f.started();
      await f.api.ended(f.automatic(chain,{background:{running:1,pending:0}}));
      assert.deepEqual([f.goal().waiting,f.goal().seq],[true,2]);
      f.host.runs=[{turn:'turn-2',sequence:2,status:'completed',origin:null,error_code:null},{turn:'turn-1',sequence:1,status:'completed',origin:null,error_code:null}];
      return f;
    };
    const state=async f=>{const shown=(await f.api.inspect()).goal;return [shown.status,shown.state,shown.reason];};
    let f=await waiting(1);
    // The work runs, or its result waits for its notice turn.
    for (const background of [null,{running:1,pending:0},{running:0,pending:1}]) {
      f.host.background=background;
      assert.deepEqual(await state(f),['active','waiting',null],JSON.stringify(background));
    }
    // The user stopped the command: no notice turn follows.
    f.host.background={running:0,pending:0};
    assert.deepEqual([...await state(f),f.goal().status],['paused','paused','stopped','active']);
    // A notice turn that is queued or running will come here when it ends.
    f.host.runs.unshift({turn:'notice',sequence:3,status:'running',origin:null,error_code:null});
    assert.deepEqual(await state(f),['active','waiting',null]);
    f.host.runs.shift();
    await f.api.control({kind:'resume',goal_id:f.goal().id});
    assert.deepEqual([f.goal().status,f.goal().waiting,f.host.continued.at(-1).user],['active',false,true]);
    // A reply continues it as well.
    f=await waiting(1);
    f.host.background={running:0,pending:0};
    await f.api.ended(f.turn({sequence:3}));
    assert.deepEqual([f.goal().status,f.goal().waiting,f.host.continued.length],['active',false,2]);
    // The result was held because the automatic turns reached the limit.
    f=await waiting(25);
    f.host.background={running:0,pending:0};
    assert.deepEqual(await state(f),['paused','paused','turns']);
  });
  await t.test('a goal restored from history waits for Resume',async()=>{
    const f=await fixture();
    const value=await f.started();
    f.put('goal',value,true);
    assert.deepEqual([(await f.api.inspect()).goal.status,(await f.api.inspect()).goal.reason],['paused','restored']);
    await f.api.ended(f.turn());
    assert.deepEqual([f.goal().status,f.goal().reason,f.host.continued.length,f.entry('goal').restored],['paused','restored',1,false]);
    await f.api.ended(f.turn());
    assert.equal(f.goal().status,'paused');
    await f.api.control({kind:'resume',goal_id:f.goal().id});
    assert.equal(f.goal().status,'active');assert.equal(f.host.continued.length,2);
  });
  await t.test('a goal saved by 0.1.x keeps its text and waits',async()=>{
    const f=await fixture();
    f.put('goal',{id:'00000000-0000-4000-8000-00000000000a',description:'Old goal',state:'active',after:null});
    const shown=(await f.api.inspect()).goal;
    assert.deepEqual([shown.objective,shown.status,shown.reason],['Old goal','paused','stopped']);
  });
});

test('End is the user\'s and there is no edit',async t=>{
  await t.test('the objective changes only by replacing the goal',async()=>{
    const f=await fixture();
    await f.started('Ship it');
    const id=f.goal().id;
    await assert.rejects(f.api.control({kind:'edit',goal_id:id,objective:'Ship it with docs'}),/Invalid goal arguments/);
    await assert.rejects(f.api.control({kind:'edit',goal_id:id}),/Invalid goal action/);
    await assert.rejects(f.api.control({kind:'clear',goal_id:id}),/Invalid goal action/);
    assert.deepEqual([f.goal().objective,f.goal().id],['Ship it',id]);
    await f.api.ended(f.automatic(1));
    assert.doesNotMatch(f.host.continued.at(-1).message.text,/edited/);
  });
  await t.test('End stops the goal turn and keeps the goal as Ended',async()=>{
    const f=await fixture();
    await f.started('Ship it');
    f.host.runs=[{turn:'running',sequence:2,status:'running',origin:null,error_code:null}];
    const view=await f.api.control({kind:'end',goal_id:f.goal().id});
    assert.equal(view.goal,null);assert.equal(f.entry('goal').present,false);
    assert.deepEqual(f.host.stopped,['running']);
    assert.deepEqual(view.history.map(item=>[item.objective,item.outcome,item.turns,item.tokens]),[['Ship it','cleared',1,120]]);
    await f.api.ended(f.automatic(1,{status:'cancelled'}));
    assert.equal(f.host.continued.length,1);
  });
  await t.test('the history keeps the last twenty goals',async()=>{
    const f=await fixture();
    for (let index=0;index<22;index++) await f.api.command({arguments:`Goal ${index}`,message:{text:`/goal Goal ${index}`}});
    assert.equal(f.history().length,20);
    assert.deepEqual([f.history()[0].objective,f.history().at(-1).objective],['Goal 20','Goal 1']);
  });
});

test('initialize pins the objective and states the turn',async()=>{
  const tools=['get_goal','create_goal','update_goal'];
  const f=await fixture();
  assert.deepEqual(plain(await f.api.initialize({tools,mode:'code',automatic:false,notice:false})),{tools,instruction:''});
  assert.deepEqual(f.host.state,user);
  await f.started('Ship it');
  const active=await f.api.initialize({tools,mode:'code',automatic:true,notice:false,continuation:{marker:'verify'}});
  assert.deepEqual(f.host.state,audit);
  assert.match(active.instruction,/Goal mode is active/);assert.match(active.instruction,/You cannot replace or end the goal/);
  assert.deepEqual(plain(active.parts),[`Goal set by the user for this conversation . Only the user can replace or end it.\n\n<objective>\nShip it\n</objective>`]);
  // The instruction is fixed text per state, so it is not repeated turn by turn.
  assert.equal((await f.api.initialize({tools,mode:'code',automatic:true,notice:false,continuation:{marker:null}})).instruction,active.instruction);
  const plan=await f.api.initialize({tools,mode:'plan',automatic:false,notice:false});
  assert.deepEqual(plain(plan.tools),['get_goal']);assert.match(plan.instruction,/Plan mode/);
  await f.tool(loop,()=>f.api.update({status:'complete',note:'claimed'}));
  await f.api.ended(f.automatic(1));
  await f.tool(audit,()=>f.api.update({status:'complete',note:'verified'}));
  const done=await f.api.initialize({tools,mode:'code',automatic:false,notice:false});
  assert.match(done.instruction,/completed/);assert.equal(done.parts,undefined);
});

test('the badge and the tool result show the goal as it stands',async()=>{
  const f=await fixture();
  assert.deepEqual(plain(await f.api.badge()),[{id:'status',visible:false}]);
  assert.deepEqual(plain(await f.api.read()),{goal:null});
  f.scope.session=null;
  assert.deepEqual(plain(await f.api.badge()),[{id:'status',visible:false}]);
  f.scope.session='session-id';
  f.host.settings={token_budget:2,time_limit:60};
  await f.started('Ship it\nand prove it');
  f.clock.now+=12*60000;
  const [badge]=plain(await f.api.badge());
  assert.deepEqual(badge.segments.map(segment=>[segment.label.label,segment.label.locales?.['zh-CN'],segment.tone]),
    [['Running','执行中','accent'],['12m',undefined,'muted']]);
  assert.deepEqual(badge.label,{label:'Goal',locales:{'zh-CN':'目标'}});
  assert.deepEqual(badge.details.map(detail=>[detail.label.label,detail.value]),
    [['Goal','Ship it\nand prove it'],['Turns','0 / 25'],['Time','12m / 1h 0m'],['Tokens','120 / 2.0k']]);
  const result=plain(await f.tool(loop,()=>f.api.read()));
  assert.deepEqual([result.goal.objective,result.goal.state,result.goal.turn_limit,result.goal.token_budget],['Ship it\nand prove it','running',25,2000]);
  assert.deepEqual(result.sailry_content.blocks,[{kind:'notice',message:{label:'Running',locales:{'zh-CN':'执行中'}}},{kind:'text',path:'/goal/objective'}]);
  await f.api.control({kind:'pause',goal_id:f.goal().id});
  assert.deepEqual(plain(await f.api.badge())[0].segments.map(segment=>[segment.label.label,segment.tone]),[['Paused','warning'],['Paused by you','muted']]);
});

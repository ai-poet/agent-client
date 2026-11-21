import assert from 'node:assert/strict';
import {execute, packageInput, runtimeTest} from '../../scripts/test-host.mjs';

const directory = new URL('../', import.meta.url);
const tools = ['review_scope', 'report_finding', 'review_complete', 'delegate_review'];
const policy = {call_id: '00000000-0000-4000-8000-000000000003',
  mode: 'code', read_only: false, turn: {}};

runtimeTest('production SDK stages the captured review policy', async () => {
  const input = await packageInput(directory, 'initialize', {
    mode: 'code', tools, roles: [{key: 'reviewer', name: 'Reviewer', read_only: false, tools: true}],
  }, {policy});
  const output = execute(input);
  assert.deepEqual(output.result.Ok.tools, tools);
  assert.deepEqual(output.staged_turn, {mode: 'code', recording: true,
    roles: [{key: 'reviewer', read_only: false, tools: true}]});
  assert.deepEqual(output.calls, []);
});

runtimeTest('production SDK denies formal delegation in Plan', async () => {
  const input = await packageInput(directory, 'delegate', {
    review_id: '00000000-0000-4000-8000-000000000004', groups: ['group'], record_receipt: true,
  }, {policy: {...policy, mode: 'plan', turn: {mode: 'plan', roles: [], recording: true}}});
  const output = execute(input);
  assert.equal(output.result.Ok.isError, true);
  assert.equal(output.result.Ok.error.code, 'permission_denied');
  assert.deepEqual(output.calls, []);
});


async function stored(handler) {
  const input = await packageInput(directory,handler,{}, {policy: {...policy,
    turn: {mode:'code',roles:[],recording:true}}});
  input.context.surface = 'workspace';
  const session = input.context.session, worktree = input.context.worktree;
  const review = '00000000-0000-4000-8000-000000000004';
  const snapshot = {target:{kind:'worktree',data:{scope:'all',head:'a'.repeat(40),index_revision:'index:1'}},
    metadata_revision:'b'.repeat(64)};
  const file = {content:{diff:{start:0,count:1},old:{start:1,count:1},new:{start:2,count:1}},path:'src/main.rs',old_path:null,kind:'modified',group:'group-1',
    metadata_revision:'c'.repeat(64),revision:'d'.repeat(64),
    rule:{path:'rules/default.md',sha256:'e'.repeat(64)}};
  const group = {id:'group-1',files:[file.path]};
  const head = {v:1,kind:'head',id:review,creator:session,worktree,
    package:{name:'code-review',digest:'a'.repeat(64),settings_revision:'0'},
    snapshot,status:'open',pages:[0],counts:{files:1,groups:1,skipped:0},
    findings:[],reviewed_groups:[],receipts:[]};
  const sessionData = {id:session,creation_order:1,archived:false,project:null,worktree,revision:1,
    activity:{title:'',attention:{revision:0,unread:false},run:null,waiting:null,queued:0},
    config:{provider:'00000000-0000-4000-8000-000000000005',model:'fixture',
      mode:'code',permission:'ask',credential:null},roles:{profiles:[],providers:[]}};
  const entry = (key,value) => ({key,revision:1,present:true,value});
  const read = (command,kind,data) => ({command,response:{Ok:{kind,data}}});
  return {input,review,snapshot,file,group,head,sessionData,entry,read};
}

runtimeTest('production SDK resumes a captured package review', async () => {
  const {input,review,snapshot,file,group,head,sessionData,entry,read}=await stored('delegate');
  const {session}=input.context;
  input.input = {review_id:review,groups:[group.id],record_receipt:true};
  input.replies = [
    read({kind:'read_session',data:{session}},'session',sessionData),
    read({kind:'read_plugin_value',data:{key:`review/${review}/head`}},'plugin_value',
      entry(`review/${review}/head`,head)),
    read({kind:'read_plugin_value',data:{key:`review/${review}/scope/0`}},'plugin_value',
      entry(`review/${review}/scope/0`,{v:1,kind:'scope',rows:[
        {kind:'file',value:file},{kind:'group',value:group}]})),
  ];
  const output = execute(input);
  assert.equal(output.result.Ok.isError,undefined);
  const captured = JSON.parse(output.result.Ok.task.split('\nFrozen review capture:\n')[1]);
  assert.equal(captured.review_id,review);
  assert.deepEqual(captured.snapshot,snapshot);
  assert.deepEqual(captured.files,[file]);
  assert.equal(output.calls.length,3);
  for (const call of output.calls) assert.deepEqual(call.plugin,input.context);
  assert.notEqual(input.context.package.digest,head.package.digest);
});

runtimeTest('production SDK reads captured source without Git',async()=>{
  const {input,review,file,group,head,sessionData,entry,read}=await stored('read');
  const {session}=input.context;
  input.input={review_id:review,path:file.path,part:'new'};
  input.policy.read_only=true;
  input.replies=[
    read({kind:'read_session',data:{session}},'session',sessionData),
    read({kind:'read_plugin_value',data:{key:`review/${review}/head`}},'plugin_value',entry(`review/${review}/head`,head)),
    read({kind:'read_plugin_value',data:{key:`review/${review}/scope/0`}},'plugin_value',entry(`review/${review}/scope/0`,
      {v:1,kind:'scope',capture:review,rows:[{kind:'file',value:file},{kind:'group',value:group}]})),
    read({kind:'read_plugin_value',data:{key:`review/${review}/data/2`}},'plugin_value',entry(`review/${review}/data/2`,
      {v:1,kind:'content',capture:review,path:file.path,part:'new',revision:file.revision,
        offset:0,next:null,text:'captured();\r\nlast_without_newline'})),
  ];
  const output=execute(input);
  assert.equal(output.result.Ok.text,'captured();\r\nlast_without_newline');
  assert.equal(output.result.Ok.next,null);
  assert.equal(output.result.Ok.revision,file.revision);
  assert.deepEqual(output.calls.map(call=>call.command.kind),['read_session','read_plugin_value','read_plugin_value','read_plugin_value']);
});


runtimeTest('production SDK rebuilds only a confirmed key conflict',async()=>{
  const {input,review,file,group,head,sessionData,entry,read}=await stored('finding');
  const intent=policy.call_id, other='00000000-0000-4000-8000-000000000009';
  head.findings=[other];
  const arguments_={review_id:review,path:file.path,revision:file.revision,side:'new',line:7,
    snippet:'return changed();',severity:'P1',title:'Changed result is lost',
    explanation:'The added return loses its result.',evidence:'The frozen changed line demonstrates the loss.'};
  input.input={arguments:arguments_,step:1,state:{intent,writes:[],result:{}},
    outcome:{isError:true,error:{code:'revision_conflict',message:'plugin value changed',
      details:{kind:'plugin_value_conflict'}}}};
  input.replies=[
    read({kind:'read_session',data:{session:input.context.session}},'session',sessionData),
    read({kind:'read_plugin_value',data:{key:`review/${review}/head`}},'plugin_value',
      {...entry(`review/${review}/head`,head),revision:2}),
    read({kind:'read_plugin_value',data:{key:`review/${review}/scope/0`}},'plugin_value',
      entry(`review/${review}/scope/0`,{v:1,kind:'scope',rows:[{kind:'file',value:file},{kind:'group',value:group}]})),
    read({kind:'read_plugin_value',data:{key:`review/${review}/data/0`}},'plugin_value',
      entry(`review/${review}/data/0`,{v:1,kind:'content',capture:review,path:file.path,part:'diff',
        revision:file.revision,offset:0,next:null,lines:[{kind:'added',old_line:null,new_line:7,text:'return changed();'}]})),
  ];
  const output=execute(input),transition=output.result.Ok;
  assert.equal(transition.call.operation,'storage.transaction');
  const operations=transition.call.arguments.operations;
  assert.equal(operations[0].data.expected_revision,'2');
  assert.deepEqual(operations[0].data.value.findings,[other,intent]);
  assert.equal(operations[1].data.key,`review/${review}/finding/${intent}`);
  assert.equal(operations[1].data.value.finding.revision,file.revision);
  assert.equal(transition.state.intent,intent);
  assert.deepEqual(output.calls.map(call=>call.command.kind),
    ['read_session','read_plugin_value','read_plugin_value','read_plugin_value']);
});

runtimeTest('production SDK preserves non-key failures without reads',async()=>{
  for (const error of [
    {code:'revision_conflict',message:'plugin configuration changed'},
    {code:'outcome_unknown',message:'Completion unavailable'},
    {code:'cancelled',message:'Turn cancelled'},
    {code:'permission_denied',message:'Package revoked'},
  ]) {
    const input=await packageInput(directory,'finding',{
      arguments:{},step:1,state:{intent:policy.call_id},outcome:{isError:true,error},
    },{policy});
    const output=execute(input);
    assert.deepEqual(output.result.Ok,{result:{isError:true,error}});
    assert.deepEqual(output.calls,[]);
  }
});

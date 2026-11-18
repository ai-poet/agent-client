import assert from 'node:assert/strict';
import {runtimeTest,execute,packageInput,translate,translationInput} from '../../scripts/test-host.mjs';

const root=new URL('../',import.meta.url);
const script={entry:'dev.sailry.platform/host/protocol.js',resources:[
  'dev.sailry.platform/host/protocol.js','dev.sailry.platform/errors.js',
]};
const response=(body,status=200)=>({status,headers:{},body:JSON.stringify(body)});
for(const [name,handler,input,expected] of [
  ['active activity','status',response({code:0,data:{active:true,today_checked_in:false}}),{active:true,checked:false}],
  ['closed activity','status',response({code:0,data:{active:false,today_checked_in:false}}),{active:false,checked:false}],
  ['completed day','status',response({code:0,data:{active:true,today_checked_in:true}}),{active:true,checked:true}],
  ['duplicate claim','claimed',response({code:10001},400),{status:'already',credit:null}],
  ['measured claim','claimed',response({code:0,data:{credit:'37.25'}}),{status:'claimed',credit:37.25}],
  ['unknown claim amount','claimed',response({code:0,data:{}}),{status:'claimed',credit:null}],
]) runtimeTest(`production VM check-in ${name}`,async()=>{
  const report=translate(await translationInput(root,script,handler,input));
  assert.deepEqual(report.result.Ok,expected);assert.deepEqual(report.calls,[]);
});

const id='00000000-0000-4000-8000-000000000001';
const provider={id,revision:'2',preset:{id:'china',source:{kind:'package',data:{name:'workbuddy'}}}};
const key=`checkin/day/${id}`;
const stamp='2026-10-20';
const entry=(revision,value)=>({key,revision,present:revision!==0,value});
const reply=(command,kind,data)=>({command,response:{Ok:{kind,data}}});
const read=()=>({kind:'read_plugin_value',data:{key}});
const status=()=>({kind:'request_provider_http',data:{provider:id,expected_revision:2,request:'checkin_status',input:{}}});
const claim=()=>({kind:'execute_provider_http',data:{provider:id,expected_revision:2,request:'checkin_claim',input:{}}});
const write=(revision,value)=>({kind:'write_plugin_value',data:{key,expected_revision:revision,value:{day:stamp,...value}}});

async function input(replies) {
  const value=await packageInput(root,'checkin',{provider,locale:'en'},
    {context:{package:{name:'workbuddy',digest:'f'.repeat(64),settings_revision:0},session:null,worktree:null},replies});
  // Freeze only the fixture clock; the declared handler and SDK stay unchanged.
  value.files[value.entry]+='\nDate.now = () => 1792454400000;\n';
  return value;
}

runtimeTest('production SDK skips closed activities',async()=>{
  const value={day:stamp,status:'closed'};
  const report=execute(await input([
    reply(read(),'plugin_value',entry(0,null)),
    reply(status(),'plugin_http',response({code:0,data:{active:false,today_checked_in:false}})),
    reply(write(0,{status:'closed'}),'plugin_value',entry(1,value)),
  ]));
  assert.deepEqual(report.result.Ok,{status:'closed'});
  assert.equal(report.calls.length,3);assert.equal(report.calls.some(call=>call.command.kind==='execute_provider_http'),false);
});

runtimeTest('production SDK preserves pending daily claims',async()=>{
  const report=execute(await input([
    reply(read(),'plugin_value',entry(1,{day:stamp,status:'pending'})),
    reply(status(),'plugin_http',response({code:0,data:{active:true,today_checked_in:false}})),
  ]));
  assert.deepEqual(report.result.Ok,{status:'pending',skipped:true});assert.equal(report.calls.length,2);
});

runtimeTest('production SDK admits one protected claim',async()=>{
  const value={day:stamp,status:'already',credit:null};
  const report=execute(await input([
    reply(read(),'plugin_value',entry(0,null)),
    reply(status(),'plugin_http',response({code:0,data:{active:true,today_checked_in:false}})),
    reply(write(0,{status:'pending'}),'plugin_value',entry(1,{day:stamp,status:'pending'})),
    reply(claim(),'plugin_http',response({code:10001},400)),
    reply(write(1,{status:'already',credit:null}),'plugin_value',entry(2,value)),
  ]));
  assert.deepEqual(report.result.Ok,{status:'already',credit:null});
  assert.equal(report.calls.filter(call=>call.command.kind==='execute_provider_http').length,1);
  assert.deepEqual(report.calls.map(call=>call.command.kind),[
    'read_plugin_value','request_provider_http','write_plugin_value','execute_provider_http','write_plugin_value',
  ]);
  assert.ok(report.calls.every(call=>call.plugin.package.name==='workbuddy'));
});

runtimeTest('production SDK reconciles timing without a footer',async()=>{
  const packageRef={name:'workbuddy',digest:'f'.repeat(64),settings_revision:3};
  const scheduleKey=`checkin/schedule/${id}`,scheduleId='00000000-0000-4000-8000-000000000010';
  const payload={provider,locale:'en'};
  const record={id:scheduleId,...payload,time:'09:05',settings_revision:'2'};
  const schedule={id:scheduleId,revision:5,enabled:true,topic:scheduleId,payload,
    timing:{kind:'weekly',data:{timezone:'Asia/Shanghai',weekdays:[1,2,3,4,5,6,7],hour:9,minute:5}},next_ms:null};
  const updated={...record,automatic:true,enabled:true,time:'07:30',settings_revision:'3'};
  const next={...schedule,payload:{...payload,automatic:true},timing:{...schedule.timing,data:{...schedule.timing.data,hour:7,minute:30}}};
  const operations=[
    {kind:'write',data:{key:scheduleKey,value:updated,expected_revision:2}},
    {kind:'dispatch',data:{kind:'save_schedule',data:next}},
  ];
  const report=execute(await packageInput(root,'reconcile',{key:scheduleKey},{
    context:{package:packageRef,session:null,worktree:null},replies:[
      reply({kind:'read_plugin_settings',data:{package:packageRef}},'plugin_settings',
        {package:packageRef,values:{checkin_enabled:true,checkin_time:'07:30'},configured:[],keepable:[],ready:true}),
      reply({kind:'read_plugin_value',data:{key:scheduleKey}},'plugin_value',
        {key:scheduleKey,revision:2,present:true,value:record}),
      reply({kind:'dispatch',data:{package:packageRef,action:{kind:'list_schedules'}}},'dispatch',{kind:'schedules',data:[schedule]}),
      reply({kind:'plugin_transaction',data:{operations}},'plugin_transaction',[
        {kind:'plugin_value',data:{key:scheduleKey,revision:3,present:true,value:updated}},
        {kind:'dispatch',data:{kind:'schedule',data:{...next,revision:6}}},
      ]),
    ],
  }));
  assert.deepEqual(report.result.Ok,{updated:1});assert.equal(report.calls.length,4);
  assert.ok(report.calls.every(call=>!['request_provider_http','execute_provider_http','publish_notification'].includes(call.command.kind)));
  assert.equal(report.calls.at(-1).command.data.operations.length,2);
});

runtimeTest('production SDK deduplicates failures and reports recovery',async()=>{
  const packageRef={name:'workbuddy',digest:'f'.repeat(64),settings_revision:0};
  const content=(message,kind)=>({title:'Account credits',message,kind,session:null});
  const notice=(message,kind)=>({id:'00000000-0000-4000-8000-000000000050',sequence:1,
    package:'workbuddy',timestamp_ms:1792454400000,read:false,content:content(message,kind)});
  const failed={day:stamp,status:'failed',submitted:false,error:{code:'unavailable',message:'Unable to claim daily credits'}};
  const operations=[
    {kind:'write',data:{key,expected_revision:0,value:failed}},
    {kind:'notify',data:content('Unable to claim daily credits','error')},
  ];
  const unavailable={command:status(),response:{Err:{code:'unavailable',message:'Synthetic transport unavailable'}}};
  const value=await input([
    reply(read(),'plugin_value',entry(0,null)),unavailable,
    reply({kind:'plugin_transaction',data:{operations}},'plugin_transaction',[
      {kind:'plugin_value',data:entry(1,failed)},
      {kind:'notification',data:notice('Unable to claim daily credits','error')},
    ]),
    reply(read(),'plugin_value',entry(1,failed)),unavailable,
    reply(read(),'plugin_value',entry(1,failed)),
    reply(status(),'plugin_http',response({code:0,data:{active:true,today_checked_in:false}})),
    reply(write(1,{status:'pending'}),'plugin_value',entry(2,{day:stamp,status:'pending'})),
    reply(claim(),'plugin_http',response({code:0,data:{credit:12.25}})),
    reply(write(2,{status:'claimed',credit:12.25}),'plugin_value',entry(3,{day:stamp,status:'claimed',credit:12.25})),
    reply({kind:'publish_notification',data:{package:packageRef,content:content('Daily credits received 12.25','success')}},
      'notification',notice('Daily credits received 12.25','success')),
  ]);
  // Observe three explicit calls through the same real SDK, without a retry timer.
  value.files[value.entry]+="\nimport {checkin as observed} from './checkin.js'; export async function probe(input) {return [await observed(input),await observed(input),await observed(input)];}\n";
  value.handler='probe';
  const report=execute(value);
  assert.deepEqual(report.result.Ok.map(result=>result.status),['failed','failed','claimed']);
  assert.equal(report.calls.filter(call=>call.command.kind==='execute_provider_http').length,1);
  assert.equal(report.calls.filter(call=>call.command.kind==='plugin_transaction').length,1);
  assert.equal(report.calls.filter(call=>call.command.kind==='publish_notification').length,1);
});

runtimeTest('production SDK records uncertain claims as warnings',async()=>{
  const packageRef={name:'workbuddy',digest:'f'.repeat(64),settings_revision:0};
  const error={code:'outcome_unknown',message:'Daily credit claim outcome unknown'};
  const result={status:'unknown',error};
  const content={title:'Account credits',message:error.message,kind:'warning',session:null};
  const report=execute(await input([
    reply(read(),'plugin_value',entry(0,null)),
    reply(status(),'plugin_http',response({code:0,data:{active:true,today_checked_in:false}})),
    reply(write(0,{status:'pending'}),'plugin_value',entry(1,{day:stamp,status:'pending'})),
    {command:claim(),response:{Err:{code:'outcome_unknown',message:'Synthetic interrupted observation'}}},
    reply(write(1,result),'plugin_value',entry(2,{day:stamp,...result})),
    reply({kind:'publish_notification',data:{package:packageRef,content}},'notification',
      {id:'00000000-0000-4000-8000-000000000050',sequence:1,package:'workbuddy',timestamp_ms:1792454400000,read:false,content}),
  ]));
  assert.deepEqual(report.result.Ok,result);
  assert.equal(report.calls.filter(call=>call.command.kind==='execute_provider_http').length,1);
  assert.equal(report.calls.filter(call=>call.command.kind==='publish_notification').length,1);
});

runtimeTest('production SDK skips disabled automatic claims',async()=>{
  const packageRef={name:'workbuddy',digest:'f'.repeat(64),settings_revision:3};
  const report=execute(await packageInput(root,'checkin',{provider,automatic:true},{
    context:{package:packageRef,session:null,worktree:null},replies:[
      reply({kind:'read_plugin_settings',data:{package:packageRef}},'plugin_settings',
        {package:packageRef,values:{checkin_enabled:false,checkin_time:'09:05'},configured:[],keepable:[],ready:true}),
    ],
  }));
  assert.deepEqual(report.result.Ok,{status:'disabled',skipped:true});
  assert.deepEqual(report.calls.map(call=>call.command.kind),['read_plugin_settings']);
});

for(const enabled of [false,true]) runtimeTest(`production SDK ${enabled?'reenables':'disables'} the same schedule`,async()=>{
  const packageRef={name:'workbuddy',digest:'f'.repeat(64),settings_revision:3};
  const scheduleKey=`checkin/schedule/${id}`,scheduleId='00000000-0000-4000-8000-000000000010';
  const payload={provider,locale:'en',automatic:true};
  const record={id:scheduleId,...payload,enabled:!enabled,...(enabled?{paused:true}:{}),time:'09:05',settings_revision:'2'};
  const schedule={id:scheduleId,revision:5,enabled:!enabled,topic:scheduleId,payload,
    timing:{kind:'weekly',data:{timezone:'Asia/Shanghai',weekdays:[1,2,3,4,5,6,7],hour:9,minute:5}},next_ms:null};
  const updated={id:scheduleId,...payload,enabled,...(!enabled?{paused:true}:{}),time:'07:30',settings_revision:'3'};
  const next={...schedule,enabled,timing:{...schedule.timing,data:{...schedule.timing.data,hour:7,minute:30}}};
  const handler={name:scheduleId,revision:0,enabled:true,source:{package:'workbuddy',topic:scheduleId},queue:scheduleId,
    callback:{scope:{worktree:null,session:null},completion:'command',
      command:{kind:'call_plugin',data:{handler:'checkin',input:null}},bindings:{'/data/input':'/payload'}}};
  const action=enabled?{kind:'save_handler',data:handler}
    :{kind:'remove_handler',data:{name:scheduleId,expected_revision:7,cancel_pending:true}};
  const operations=[
    {kind:'write',data:{key:scheduleKey,value:updated,expected_revision:2}},
    {kind:'dispatch',data:{kind:'save_schedule',data:next}},
    {kind:'dispatch',data:action},
  ];
  const report=execute(await packageInput(root,'reconcile',{key:scheduleKey},{
    context:{package:packageRef,session:null,worktree:null},replies:[
      reply({kind:'read_plugin_settings',data:{package:packageRef}},'plugin_settings',
        {package:packageRef,values:{checkin_enabled:enabled,checkin_time:'07:30'},configured:[],keepable:[],ready:true}),
      reply({kind:'read_plugin_value',data:{key:scheduleKey}},'plugin_value',
        {key:scheduleKey,revision:2,present:true,value:record}),
      reply({kind:'dispatch',data:{package:packageRef,action:{kind:'list_schedules'}}},'dispatch',{kind:'schedules',data:[schedule]}),
      reply({kind:'dispatch',data:{package:packageRef,action:{kind:'list_handlers'}}},'dispatch',
        {kind:'handlers',data:enabled?[]:[{...handler,revision:7}]}),
      reply({kind:'plugin_transaction',data:{operations}},'plugin_transaction',[
        {kind:'plugin_value',data:{key:scheduleKey,revision:3,present:true,value:updated}},
        {kind:'dispatch',data:{kind:'schedule',data:{...next,revision:6}}},
        {kind:'dispatch',data:enabled?{kind:'handler',data:{...handler,revision:1}}:{kind:'removed'}},
      ]),
    ],
  }));
  assert.deepEqual(report.result.Ok,{updated:1});assert.equal(report.calls.length,5);
  assert.ok(report.calls.every(call=>!['request_provider_http','execute_provider_http','publish_notification'].includes(call.command.kind)));
});

for(const phase of ['timing','off','on']) runtimeTest(`production SDK preserves manual pauses during ${phase}`,async()=>{
  const packageRef={name:'workbuddy',digest:'f'.repeat(64),settings_revision:3};
  const scheduleKey=`checkin/schedule/${id}`,scheduleId='00000000-0000-4000-8000-000000000010';
  const enabled=phase!=='off',payload={provider,locale:'en',automatic:true};
  const record={id:scheduleId,...payload,...(phase==='timing'?{}:{enabled:phase==='off'}),time:'09:05',settings_revision:'2'};
  const schedule={id:scheduleId,revision:5,enabled:false,topic:scheduleId,payload,
    timing:{kind:'weekly',data:{timezone:'Asia/Shanghai',weekdays:[1,2,3,4,5,6,7],hour:9,minute:5}},next_ms:null};
  const updated={id:scheduleId,...payload,enabled,time:'07:30',settings_revision:'3'};
  const next={...schedule,timing:{...schedule.timing,data:{...schedule.timing.data,hour:7,minute:30}}};
  const handler={name:scheduleId,revision:0,enabled:true,source:{package:'workbuddy',topic:scheduleId},queue:scheduleId,
    callback:{scope:{worktree:null,session:null},completion:'command',
      command:{kind:'call_plugin',data:{handler:'checkin',input:null}},bindings:{'/data/input':'/payload'}}};
  const action=phase==='timing'?[]:[{kind:'dispatch',data:enabled?{kind:'save_handler',data:handler}
    :{kind:'remove_handler',data:{name:scheduleId,expected_revision:7,cancel_pending:true}}}];
  const operations=[
    {kind:'write',data:{key:scheduleKey,value:updated,expected_revision:2}},
    {kind:'dispatch',data:{kind:'save_schedule',data:next}},...action,
  ];
  const replies=[
    reply({kind:'read_plugin_settings',data:{package:packageRef}},'plugin_settings',
      {package:packageRef,values:{checkin_enabled:enabled,checkin_time:'07:30'},configured:[],keepable:[],ready:true}),
    reply({kind:'read_plugin_value',data:{key:scheduleKey}},'plugin_value',
      {key:scheduleKey,revision:2,present:true,value:record}),
    reply({kind:'dispatch',data:{package:packageRef,action:{kind:'list_schedules'}}},'dispatch',{kind:'schedules',data:[schedule]}),
  ];
  if(phase!=='timing') replies.push(reply({kind:'dispatch',data:{package:packageRef,action:{kind:'list_handlers'}}},'dispatch',
    {kind:'handlers',data:enabled?[]:[{...handler,revision:7}]}));
  replies.push(reply({kind:'plugin_transaction',data:{operations}},'plugin_transaction',[
    {kind:'plugin_value',data:{key:scheduleKey,revision:3,present:true,value:updated}},
    {kind:'dispatch',data:{kind:'schedule',data:{...next,revision:6}}},
    ...(phase==='timing'?[]:[{kind:'dispatch',data:enabled?{kind:'handler',data:{...handler,revision:1}}:{kind:'removed'}}]),
  ]));
  const report=execute(await packageInput(root,'reconcile',{key:scheduleKey},{
    context:{package:packageRef,session:null,worktree:null},replies,
  }));
  assert.deepEqual(report.result.Ok,{updated:1});
  assert.equal(report.calls.length,phase==='timing'?4:5);
  const saved=report.calls.at(-1).command.data.operations;
  assert.equal(saved[1].data.data.id,scheduleId);assert.equal(saved[1].data.data.enabled,false);
  assert.equal(Object.hasOwn(saved[0].data.value,'paused'),false);
  assert.ok(report.calls.every(call=>!['request_provider_http','execute_provider_http','publish_notification'].includes(call.command.kind)));
});

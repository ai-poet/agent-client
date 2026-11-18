import assert from 'node:assert/strict';
import {execute, packageInput, runtimeTest} from '../../scripts/test-host.mjs';

runtimeTest('shows a goal whose last turn was lost as paused, through the SDK', async () => {
  const input=await packageInput(new URL('../',import.meta.url),'read',{});
  const session=input.context.session;
  const lost='00000000-0000-4000-8000-000000000004';
  const objective='Preserve the complete requested objective';
  const goal={id:'00000000-0000-4000-8000-000000000005',objective,version:1,status:'active',reason:null,detail:null,seq:39,waiting:false,
    claim:null,evidence:null,edited:false,wrap:null,idle:0,notice:null,turns:3,chain:2,tokens:900,window_tokens:900,elapsed_ms:60000,window_ms:60000,
    since_ms:null,created_ms:0,finished_ms:null};
  input.replies=[
    {command:{kind:'read_plugin_conversation_value',data:{key:'goal'}},response:{Ok:{kind:'plugin_conversation_value',data:{
      key:'goal',revision:5,present:true,restored:false,value:goal,
    }}}},
    {command:{kind:'read_conversation_runs',data:{session,before:null,limit:8}},
      response:{Ok:{kind:'conversation_runs',data:{session,revision:1,queue_revision:0,next_before:null,
        runs:[{turn:lost,sequence:40,status:'interrupted',origin:null,error_code:null}]}}}},
    {command:{kind:'read_plugin_settings',data:{package:input.context.package}},
      response:{Ok:{kind:'plugin_settings',data:{package:input.context.package,
        values:{turn_limit:17,token_budget:2,time_limit:3},configured:[],keepable:[],ready:true}}}},
  ];
  const report=execute(input);
  assert.equal(report.result.Err,undefined);
  assert.equal(report.result.Ok.goal.status,'paused');
  assert.equal(report.result.Ok.goal.reason,'restart');
  assert.equal(report.result.Ok.goal.objective,objective);
  assert.equal(report.result.Ok.goal.revision,'5');
  assert.equal(report.result.Ok.goal.turn_limit,17);
  assert.equal(report.result.Ok.goal.token_budget,2000);
  assert.equal(report.result.Ok.goal.time_limit_ms,180000);
  assert.deepEqual(report.calls.map(call=>call.command.kind),[
    'read_plugin_conversation_value','read_conversation_runs','read_plugin_settings',
  ]);
  assert.equal(report.calls.some(call=>call.command.kind==='plugin_transaction'),false);
});

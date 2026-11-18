import test from 'node:test';
import assert from 'node:assert/strict';
import * as claude from '../dev.sailry.platform/adapters/claude.js';
import * as codex from '../dev.sailry.platform/adapters/codex.js';
import {metric} from '../dev.sailry.platform/desktop/format.js';
import {messages} from '../dev.sailry.platform/desktop/locales.js';
import {runtimeTest,translate,translationInput} from '../../scripts/test-host.mjs';

const root = new URL('../',import.meta.url);
const script = {
  entry:'dev.sailry.platform/adapters/claude.js',
  resources:['dev.sailry.platform/adapters/claude.js','dev.sailry.platform/adapters/codex.js',
    'dev.sailry.platform/adapters/response.js','dev.sailry.platform/desktop/format.js'],
};

// Test-only composition runs real package modules in QuickJS without UI or HTTP.
// Authentication names select pure adapters; Node ownership is tested separately.
function inspect(input) {
  try {
    const adapter = input.authentication === 'claude_oauth' ? claude : codex;
    const snapshot = adapter.decode(input.response,input.now);
    return {snapshot,metric:metric(snapshot,input.text)};
  } catch (error) {
    return {error:{status:error.status ?? null,message:error.message}};
  }
}

const entry = `import * as claude from './adapters/claude.js';
import * as codex from './adapters/codex.js';
import {metric} from './desktop/format.js';
export ${inspect.toString()}
export function missing(input) {return metric(null,input.text);}`;

for (const [authentication,status] of [['chat_gpt',403],['chat_gpt_oauth',429],['claude_oauth',401]]) {
  for (const kind of ['measured','nullable','error']) {
    const used = kind === 'nullable' ? null : 25;
    const body = authentication === 'claude_oauth'
      ? {five_hour:{utilization:used,resets_at:null},seven_day:null}
      : {rate_limit:{primary_window:{used_percent:used,limit_window_seconds:18000,reset_at:null},secondary_window:null}};
    // Intentional Unicode upstream error content must survive the adapter catch.
    const message = 'Fixture quota denied · 原始错误';
    if (kind === 'error') body.error = {message};
    const input = {authentication,now:1000,text:messages('en'),
      response:{status:kind === 'error' ? status : 200,body:JSON.stringify(body)}};
    function check(result) {
      if (kind === 'error') {
        assert.deepEqual(result,{error:{status,message}});
      } else {
        assert.deepEqual(result.snapshot,{windows:[{key:'five_hour',used,reset:null}],credits:null,extra:null});
        assert.deepEqual(result.metric,kind === 'nullable' ? {value:null,details:[]} : {value:'75%',details:[
          {label:{label:'5h'},value:'75%',progress:75},
        ]});
      }
    }
    const name = `reads ${authentication} ${kind} usage`;
    test(name,()=>check(inspect(input)));
    runtimeTest(`production VM ${name}`,async()=>{
      const request = await translationInput(root,script,'inspect',input);
      request.entry = 'dev.sailry.platform/contract.js';
      request.files[request.entry] = entry;
      const report = translate(request);
      assert.deepEqual(report.calls,[]);
      assert.equal(report.staged_turn,null);
      assert.ok(report.result.Ok);
      check(report.result.Ok);
    });
  }
}

runtimeTest('production VM omits unavailable measurements',async()=>{
  const request=await translationInput(root,script,'missing',{text:messages('en')});
  request.entry='dev.sailry.platform/contract.js';request.files[request.entry]=entry;
  const report=translate(request);
  assert.deepEqual(report.calls,[]);
  assert.equal(report.staged_turn,null);
  assert.deepEqual(report.result.Ok,{value:null,details:[]});
});

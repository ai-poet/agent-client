import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {execFileSync} from 'node:child_process';
import * as claude from '../dev.sailry.platform/adapters/claude.js';
import * as codex from '../dev.sailry.platform/adapters/codex.js';
import {ResponseError} from '../dev.sailry.platform/adapters/response.js';
import {metric} from '../dev.sailry.platform/desktop/format.js';
import {messages} from '../dev.sailry.platform/desktop/locales.js';

const response = body => ({status:200,headers:{},body:JSON.stringify(body)});

test('reads Claude percentages and independent reset times',()=>{
  const result = claude.decode(response({five_hour:{utilization:42.5,resets_at:'2026-10-20T12:00:00Z'},
    seven_day:{utilization:16,resets_at:'2026-10-23T08:00:00Z'},seven_day_sonnet:null}));
  assert.deepEqual(result.windows,[
    {key:'five_hour',used:42.5,reset:Date.parse('2026-10-20T12:00:00Z')},
    {key:'seven_day',used:16,reset:Date.parse('2026-10-23T08:00:00Z')},
  ]);
  const output = metric(result,messages('en'));
  assert.equal(output.value,'57.5%');
  assert.equal(output.details[0].progress,57.5);
  assert.equal(output.details[1].label.label,'Reset time');
  assert.match(output.details[1].value,/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/);
  assert.equal(output.details[2].value,'84%');
});

test('formats reset times in local time',()=>{
  const module = new URL('../dev.sailry.platform/desktop/format.js',import.meta.url).href;
  const source = `import {metric} from ${JSON.stringify(module)};
    console.log(JSON.stringify(metric({windows:[{key:'primary',used:20,reset:Date.parse('2026-11-19T01:52:45Z')}],credits:null,extra:null},
      {primary:'Primary',resets:'Reset time'}).details[1]));`;
  for(const [zone,expected] of [['Asia/Shanghai','2026-11-19 09:52'],['America/New_York','2026-11-18 20:52']]) {
    const output=execFileSync(process.execPath,['--input-type=module','-e',source],
      {encoding:'utf8',env:{...process.env,TZ:zone}});
    assert.deepEqual(JSON.parse(output),{label:{label:'Reset time'},value:expected});
  }
});

test('hides unavailable percentages without inventing measurements',()=>{
  const result = claude.decode(response({five_hour:{utilization:null,resets_at:null},seven_day:null}));
  assert.deepEqual(result.windows,[{key:'five_hour',used:null,reset:null}]);
  const output = metric(result,messages('en'));
  assert.equal(output.value,null);
  assert.deepEqual(output.details,[]);
  assert.deepEqual(claude.decode(response({five_hour:null})).windows,[]);
  assert.deepEqual(metric(null,messages('en')),{value:null,details:[]});
  for(const snapshot of [claude.decode(response({five_hour:null})),codex.decode(response({rate_limit:null,credits:null}))]) {
    assert.deepEqual(metric(snapshot,messages('en')),{value:null,details:[]});
  }
});

test('keeps raw overage while showing no remaining',()=>{
  const result = claude.decode(response({seven_day:{utilization:125.5,resets_at:null}}));
  const output = metric(result,messages('en'));
  assert.equal(result.windows[0].used,125.5);
  assert.equal(output.value,'0%');
  assert.equal(output.details[0].progress,0);
});

test('projects boundaries without changing account credits',()=>{
  for(const [adapter,body] of [[claude,{five_hour:{utilization:0},seven_day:{utilization:100}}],
    [codex,{rate_limit:{primary_window:{used_percent:0,limit_window_seconds:18000},secondary_window:{used_percent:100,limit_window_seconds:604800}}}]]) {
    const snapshot=adapter.decode(response(body));
    assert.deepEqual(snapshot.windows.map(window=>window.used),[0,100]);
    assert.deepEqual(metric(snapshot,messages('en')),{value:'0%',details:[
      {label:{label:'5h'},value:'100%',progress:100},
      {label:{label:'7d'},value:'0%',progress:0},
    ]});
  }
  for(const balance of [41205.37,0]) {
    const snapshot=codex.decode(response({rate_limit:{primary_window:{}},credits:{balance}}));
    assert.deepEqual(metric(snapshot,messages('en')),{value:String(balance),details:[
      {label:{label:'Account credits'},value:String(balance)},
    ]});
  }
});

test('preserves explicit Claude extra usage units',()=>{
  const result = claude.decode(response({five_hour:null,extra_usage:{is_enabled:false,used_credits:1250,
    monthly_limit:5000,currency:'USD'}}));
  assert.deepEqual(result.extra,{used:1250,limit:5000,currency:'USD',enabled:false});
  assert.equal(metric(result,messages('en')).details[0].value,'12.5 / 50 USD');
});

test('maps GPT windows by their reported duration',()=>{
  const result = codex.decode(response({rate_limit:{primary_window:{used_percent:20,limit_window_seconds:18000,
    reset_at:1792497600},secondary_window:{used_percent:35,limit_window_seconds:604800,reset_after_seconds:60}},
    credits:{has_credits:true,unlimited:false,balance:'12.50'}}),1000);
  assert.deepEqual(result.windows,[{key:'five_hour',used:20,reset:1792497600000},
    {key:'seven_day',used:35,reset:61000}]);
  assert.deepEqual(result.credits,{available:true,unlimited:false,balance:12.5});
  const output = metric(result,messages('en'));
  assert.equal(output.value,'65%');
  assert.equal(output.details[0].value,'80%');
  assert.equal(output.details[2].value,'65%');
  assert.equal(output.details.at(-1).value,'12.5');
});

test('does not guess missing GPT durations or balances',()=>{
  const result = codex.decode(response({rate_limit:{primary_window:{used_percent:null,reset_at:null},
    secondary_window:null},credits:{has_credits:false,unlimited:false,balance:null}}));
  assert.deepEqual(result.windows,[{key:'primary',used:null,reset:null}]);
  assert.equal(result.credits.balance,null);
  assert.deepEqual(metric(result,messages('en')),{value:null,details:[]});
  const unlimited = codex.decode(response({credits:{unlimited:true,balance:null}}));
  assert.equal(metric(unlimited,messages('en')).details[0].value,'Unlimited');
});

test('rejects failed reads even with valid usage fields',()=>{
  for (const adapter of [claude,codex]) for (const status of [0,199,302,401,403,429,500]) {
    assert.throws(()=>adapter.decode({status,body:JSON.stringify({five_hour:{utilization:0},rate_limit:{},
      error:{message:'Fixture usage denied'}})}),error=>error instanceof ResponseError
        && error.status===status && error.message==='Fixture usage denied');
  }
});

test('rejects malformed payloads without fabricated quota',()=>{
  for (const value of [null,[],false,{},'unexpected']) for (const adapter of [claude,codex]) {
    assert.throws(()=>adapter.decode(response(value)));
  }
  for (const value of [-1,'bad',{},false]) {
    assert.throws(()=>claude.decode(response({five_hour:{utilization:value}})));
    assert.throws(()=>codex.decode(response({rate_limit:{primary_window:{used_percent:value}}})));
  }
  assert.throws(()=>claude.decode(response({five_hour:{utilization:10,resets_at:'not a date'}})));
  assert.throws(()=>codex.decode(response({rate_limit:{primary_window:{reset_at:Number.MAX_VALUE}}})));
});

test('declares only fixed native read resources',async()=>{
  const manifest = JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  const extension = manifest.extensions['dev.sailry.platform'];
  assert.equal(manifest.version,'0.1.10');
  assert.equal(extension.api_version,'v1');
  assert.deepEqual(extension.actions,['http.request']);
  assert.deepEqual(extension.model_resources.map(resource=>resource.authentication),
    ['chat_gpt','chat_gpt_oauth','claude_oauth']);
  assert.deepEqual(extension.model_resources.map(resource=>resource.requests.usage.url),[
    'https://chatgpt.com/backend-api/wham/usage','https://chatgpt.com/backend-api/wham/usage',
    'https://api.anthropic.com/api/oauth/usage',
  ]);
  for (const resource of extension.model_resources) {
    assert.deepEqual(Object.keys(resource.requests),['usage']);
    assert.equal(resource.requests.usage.method,'GET');
    for (const [name,value] of Object.entries(resource.requests.usage.headers)) {
      assert.ok(!['authorization','chatgpt-account-id'].includes(name.toLowerCase()));
      assert.ok(!value.includes('{'));
    }
  }
  assert.deepEqual(extension.ui.map(entry=>[entry.slot,entry.kind]),[['footer','metric'],['provider','metric']]);
});

test('localizes every shared detail label',()=>{
  const english = Object.keys(messages('en')).sort();
  for (const locale of ['en','zh-CN','zh-TW','de','es','fr','ja','ko','pt-BR','ru']) {
    const text = messages(locale);
    assert.deepEqual(Object.keys(text).sort(),english);
    for (const value of Object.values(text)) assert.ok(value.trim());
  }
});


test('shares only declared usage freshness',async()=>{
  const {readFile}=await import('node:fs/promises');
  const manifest=JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  const resources=manifest.extensions['dev.sailry.platform'].model_resources;
  assert.equal(resources.length,3);
  for(const resource of resources)assert.equal(resource.requests.usage.cache_ttl_ms,180000);
});

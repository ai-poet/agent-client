import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import test from 'node:test';
import {cases,invalid} from './fixtures.mjs';
import {runtimeTest,translate,translationInput} from '../../scripts/test-host.mjs';

const root = new URL('../',import.meta.url);
const scripts = Object.fromEntries(['adapter','auth','credits'].map(name=>[name,{
  entry:`dev.sailry.platform/${name==='credits'?'desktop/credits':name}.js`,
  resources:[`dev.sailry.platform/${name==='credits'?'desktop/credits':name}.js`,
    ...(name==='auth'?[]:['dev.sailry.platform/errors.js'])],
}]));
const modules = {};
for (const [name,script] of Object.entries(scripts)) modules[name] = await import(new URL(script.entry,root));

for (const fixture of cases) {
  test(fixture.name,()=>{
    const result = modules[fixture.script??'adapter'][fixture.handler](structuredClone(fixture.input));
    fixture.check(JSON.parse(JSON.stringify(result)));
  });
  runtimeTest(`production VM ${fixture.name}`,async()=>{
    const report = translate(await translationInput(root,scripts[fixture.script??'adapter'],fixture.handler,fixture.input));
    assert.deepEqual(report.calls,[]);
    assert.equal(report.staged_turn,null);
    assert.ok(report.result.Ok);
    fixture.check(report.result.Ok);
  });
}
for (const fixture of invalid) {
  test(fixture.name,()=>assert.throws(()=>modules[fixture.script??'adapter'][fixture.handler](structuredClone(fixture.input))));
  runtimeTest(`production VM ${fixture.name}`,async()=>{
    const report = translate(await translationInput(root,scripts[fixture.script??'adapter'],fixture.handler,fixture.input));
    assert.deepEqual(report.calls,[]);
    assert.ok(report.result.Err);
    assert.equal(report.result.Err.message,'plugin translation failed');
    assert.ok(!JSON.stringify(report).includes('fixture-private-access'));
  });
}

test('carries every Hy4 suffix split across calls',()=>{
  const closing='</think:opensource>';
  const event=(content,model)=>({event:null,data:JSON.stringify({...(model?{model}:{}),choices:[{delta:{content}}]})});
  for(let split=0;split<=closing.length;split++){
    const first=modules.adapter.decode({state:null,done:false,events:[event(`Answer${closing.slice(0,split)}`,'hy4-preview')]});
    const second=modules.adapter.decode({state:first.state,done:false,events:[event(closing.slice(split)),{event:null,data:'[DONE]'}]});
    assert.equal([...first.deltas,...second.deltas].filter(delta=>delta.kind==='text').map(delta=>delta.text).join(''),'Answer');
    assert.equal(second.state.finished,true);
  }
});


test('groups regional presets without changing route identities',async()=>{
  const manifest=JSON.parse(await readFile(new URL('plugin.json',root),'utf8'));
  const presets=manifest.extensions['dev.sailry.platform'].model_providers;
  assert.equal(manifest.version,'0.1.18');
  assert.equal(manifest.extensions['dev.sailry.platform'].min_sailry_version,'0.1.6');
  // Every sign-in states its risk once, in each language the package ships.
  for (const flow of manifest.extensions['dev.sailry.platform'].model_auth) {
    assert.match(flow.warning.text,/WorkBuddy account/);
    assert.deepEqual(Object.keys(flow.warning.locales).sort(),['de','es','fr','ja','ko','pt-BR','ru','zh-CN','zh-TW']);
    for (const text of Object.values(flow.warning.locales)) assert.ok(text.trim().length>0 && text.length<=300);
  }
  const icon=manifest.extensions['dev.sailry.platform'].display.icon;
  assert.match(icon.svg,/^<svg [^>]*viewBox="0 0 40 40"/);
  assert.deepEqual(presets.map(preset=>preset.id),['china','workbuddy-global','codebuddy-global']);
  assert.equal(new Set(presets.map(preset=>preset.name)).size,3);
  for (const preset of presets) {
    assert.equal(preset.brand,'workbuddy');
    assert.deepEqual(preset.brand_display,{label:'WorkBuddy',locales:{'zh-CN':'WorkBuddy','zh-TW':'WorkBuddy'},icon});
    assert.equal(preset.routes[0].protocol,'plugin');
    assert.equal(preset.auth.data.flow,preset.id);
  }
});

test('declares regional protected reads without shared model origins',async()=>{
  const manifest=JSON.parse(await readFile(new URL('plugin.json',root),'utf8'));
  const extension=manifest.extensions['dev.sailry.platform'];
  assert.ok(extension.actions.includes('http.request'));
  assert.deepEqual(extension.model_auth.map(flow=>flow.requests.credits.url),[
    'https://www.codebuddy.cn/v2/billing/meter/get-user-resource',
    'https://www.workbuddy.ai/v2/billing/meter/get-user-resource',
    'https://www.codebuddy.ai/v2/billing/meter/get-user-resource',
  ]);
  for(const flow of extension.model_auth){
    const request=flow.requests.credits;
    assert.equal(request.method,'POST');
    assert.deepEqual(request.body.value,{PageNumber:1,PageSize:100,ProductCode:'p_tcaca',Status:[0,3],
      PackageEndTimeRangeBegin:'{input.begin}',PackageEndTimeRangeEnd:'{input.end}'});
    assert.ok(Object.values(request.headers).every(value=>!value.includes('{')));
  }
  assert.deepEqual(extension.ui.map(entry=>[entry.id,entry.slot,entry.kind]),[
    ['credits','footer','metric'],['provider_credits','provider','metric'],
  ]);
  assert.equal(extension.ui[0].icon,'reicon:money/coins');
  assert.equal(extension.model_adapters[0].origins.includes('https://www.codebuddy.cn'),false);
});

test('declares China claims as durable provider mutations',async()=>{
  const manifest=JSON.parse(await readFile(new URL('plugin.json',root),'utf8'));
  const extension=manifest.extensions['dev.sailry.platform'];
  const china=extension.model_auth.find(flow=>flow.id==='china');
  assert.equal(china.requests.checkin_status.url,'https://copilot.tencent.com/v2/billing/meter/checkin-activity-status');
  assert.equal(china.mutations.checkin_claim.url,'https://copilot.tencent.com/v2/billing/meter/daily-checkin');
  assert.equal(china.requests.checkin_claim,undefined);
  assert.deepEqual(china.mutations.checkin_claim.body,{kind:'json',value:{}});
  for(const flow of extension.model_auth.filter(flow=>flow.id!=='china')) {
    assert.equal(flow.requests.checkin_status,undefined);assert.equal(flow.mutations,undefined);
  }
  assert.deepEqual(extension.host.handlers,['register','checkin','reconcile']);
  for(const action of ['storage.read','storage.write','dispatch.manage','notifications.publish']) assert.ok(extension.actions.includes(action));
});

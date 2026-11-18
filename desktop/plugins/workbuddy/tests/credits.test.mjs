import assert from 'node:assert/strict';
import test from 'node:test';
import {readFile} from 'node:fs/promises';
import {decode,metric,range,ResponseError} from '../dev.sailry.platform/desktop/credits.js';
const labels={remaining:'Remaining',package:'Package'};

test('shows only measured credit progress',()=>{
  const value=metric({remaining:12.5,accounts:[{name:'Cycle',remaining:12.5,capacity:25}]},labels);
  assert.deepEqual(value,{value:'12.5',details:[{label:{label:'Remaining'},value:'12.5'},
    {label:{label:'Cycle'},value:'12.5 / 25',progress:50}]});
  assert.deepEqual(metric({remaining:null,accounts:[]},labels),{value:'—',details:[]});
  assert.deepEqual(metric(decode({status:200,body:{code:0,data:{Response:{Data:{Accounts:null}}}}}),labels),
    {value:'0',details:[{label:{label:'Remaining'},value:'0'}]});
  assert.equal(metric({remaining:2,accounts:[{name:null,remaining:2,capacity:null}]},labels).details[1].progress,undefined);
});
test('preserves totals within the shared detail budget',()=>{
  const accounts=Array.from({length:100},()=>({remaining:1,capacity:null}));
  assert.deepEqual(metric({remaining:100,accounts},labels),{value:'100',details:[{label:{label:'Remaining'},value:'100'}]});
});
test('keeps original public business messages',()=>{
  for(const [body,message,code] of [
    [{code:14018,displayMsg:{zh:'当前积分不足'}},'当前积分不足','14018'],
    [{data:{error:{bizcode:'6004',displayTips:{en:'Account request limit'}}}},'Account request limit','6004'],
    [{message:'  Please wait  '},'  Please wait  ',null],
    ['not JSON','',null],
  ]) assert.throws(()=>decode({status:429,body:JSON.stringify(body)}),error=>error instanceof ResponseError
    &&error.message===message&&error.code===code&&error.status===429);
});
test('rejects invalid successful balances',()=>{
  for(const account of [{CapacityRemain:-1},{CapacityRemain:'unknown'},{CapacityRemain:Infinity}])
    assert.throws(()=>decode({status:200,body:{code:0,data:{Response:{Data:{Accounts:[account]}}}}}));
});
test('builds the documented personal resource range',()=>{
  const result=range({now:new Date(2026,0,2,3,4,5).getTime()});
  assert.equal(result.begin,'2026-01-02 03:04:00');
  assert.deepEqual(range({now:new Date(2026,0,2,3,4,59).getTime()}),result);
  assert.notDeepEqual(range({now:new Date(2026,0,2,3,5,0).getTime()}),result);
  assert.match(result.end,/^212[67]-[0-9]{2}-[0-9]{2} 03:04:00$/);
});

test('declares regional billing gateway metadata',async()=>{
  const manifest=JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  const flows=manifest.extensions['dev.sailry.platform'].model_auth;
  for(const [id,host,language,version] of [
    ['china','www.codebuddy.cn','zh-CN','5.9.3'],
    ['workbuddy-global','www.workbuddy.ai','en-US','5.9.3'],
    ['codebuddy-global','www.codebuddy.ai','en-US','5.5.2'],
  ]) {
    const flow=flows.find(flow=>flow.id===id),request=flow.requests.credits;
    assert.equal(request.url,`https://${host}/v2/billing/meter/get-user-resource`);
    assert.equal(request.headers['User-Agent'],`WorkBuddy/${version}`);
    assert.equal(request.headers['X-CodeBuddy-Request'],'1');
    assert.equal(request.headers['Accept-Language'],language);
    assert.equal(request.headers.Authorization,undefined);
    assert.equal(request.cache_ttl_ms,60000);
    for(const [name,endpoint] of Object.entries(flow.requests)) if(name!=='credits') assert.equal(endpoint.cache_ttl_ms,undefined);
    for(const endpoint of Object.values(flow.mutations ?? {})) assert.equal(endpoint.cache_ttl_ms,undefined);
    assert.deepEqual(flow.inject,[{header:'Authorization',template:'Bearer {access}'},{header:'X-Domain',template:'{extra.domain}'}]);
  }
});

import {execute, packageInput, runtimeTest, validateSchema} from '../scripts/test-host.mjs';
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
const manifest=JSON.parse(readFileSync(new URL('./plugin.json',import.meta.url)));
const extension=manifest.extensions['dev.sailry.platform'];
const fixture = await packageInput(new URL('./',import.meta.url),'request',{});
const plain=value=>JSON.parse(JSON.stringify(value));
function adapter() {
  return input => {
    const output = execute({...fixture,input});
    if (output.result.Err) throw new Error(output.result.Err.message);
    return output.result.Ok;
  };
}
function outcome(body,status=200){return {data:{status,headers:{},body:JSON.stringify(body)}};}
test('captures finite operations and origin-bound credentials',()=>{
 assert.deepEqual(extension.actions,['http.request']);
 assert.deepEqual(extension.tools[0].handler.flow.operations,['http.request']);
 assert.equal(extension.tools[0].handler.parameters.additionalProperties,false);
 for(const field of extension.connectors[0].fields){
  if(field.secret) assert.equal(field.default,undefined);
 }
});

runtimeTest('keeps graph continuations on the captured resource',()=>{
 const request=adapter();
 const initial=request({step:0,arguments:{action:'messages',query:'report',limit:3}});
 const first=new URL(initial.call.arguments.url);
 assert.equal(first.origin,'https://graph.microsoft.com'); assert.equal(first.pathname,'/v1.0/me/messages');
 assert.equal(first.searchParams.get('$search'),'"report"');
 const next='https://graph.microsoft.com/v1.0/me/messages?$skiptoken=opaque%2Btoken';
 const page=plain(request({step:1,state:initial.state,outcome:outcome({value:[{id:'one'}],'@odata.nextLink':next})}));
 assert.equal(page.result.next,next);
 const continued=request({step:0,arguments:{action:'messages',cursor:next}});
 assert.equal(continued.call.arguments.url,next);
 for(const cursor of ['https://evil.test/v1.0/me/messages?token=x','https://graph.microsoft.com/v1.0/me/events?token=x',next+'#fragment'])
  assert.throws(()=>request({step:0,arguments:{action:'messages',cursor}}));
 const files=new URL(request({step:0,arguments:{action:'files',query:"O'Brien"}}).call.arguments.url);
 assert.equal(decodeURIComponent(files.pathname),"/v1.0/me/drive/root/search(q='O''Brien')");
});

runtimeTest('preserves failures without replaying uncertain effects',async()=>{
 const request=adapter(); const uncertain={isError:true,error:{code:'outcome_unknown',message:'interrupted'}};
 assert.deepEqual(plain(await request({step:1,outcome:uncertain})),{result:uncertain});
 for(const reply of [{data:{status:200,body:'invalid'}},{data:{status:429,headers:{'retry-after':['12']},body:'private error body'}}]){
  const result=plain(await request({step:1,outcome:reply}));
  assert.equal(result.result.isError,true); assert.equal(result.call,undefined);
  assert.equal(JSON.stringify(result).includes('private error body'),false);
 }
});

const targets = [
  ...['messages','message','events','event','files','file'].map(action => ({
    args:{action,...(['message','event','file'].includes(action) ? {id:'target'} : {})},
    required:['message','event','file'].includes(action) ? ['id'] : [],
  })),
  {args:{action:'events',time_min:'2026-10-18T00:00:00Z',time_max:'2026-10-19T00:00:00Z'},
    required:['time_min','time_max']},
];
runtimeTest('action targets reach the handler with their required fields', () => {
  const request = adapter();
  for (const {args,required} of targets) {
    assert.equal(request({step:0,arguments:args}).call.operation,'http.request');
    for (const field of required) {
      const incomplete = {...args}; delete incomplete[field];
      assert.throws(() => request({step:0,arguments:incomplete}), `${args.action}: ${field}`);
    }
  }
});

runtimeTest('schema requires the target fields for each action', () => {
  const values = [{}, {action:'unknown'}], expected = [false,false];
  for (const {args,required} of targets) {
    values.push(args); expected.push(true);
    for (const field of required) {
      const incomplete = {...args}; delete incomplete[field];
      values.push(incomplete); expected.push(false);
    }
  }
  assert.deepEqual(validateSchema(extension.tools[0].handler.parameters,values),expected);
});

const largeCases = [
  ...[['messages',1000],['events',1000],['files',200]].map(([action,size]) => ({arguments:{action,limit:3001},
    check:prepared => {
      assert.equal(new URL(prepared.call.arguments.url).searchParams.get('$top'),String(size));
      assert.deepEqual(plain(prepared.state.pagination),{requested_limit:3001,page_size:size});
    }})),
  {arguments:{action:'messages',query:'q'.repeat(4097)},
    check:prepared => assert.equal(new URL(prepared.call.arguments.url).searchParams.get('$search'),JSON.stringify('q'.repeat(4097)))},
  {arguments:{action:'file',id:'i'.repeat(1025)},
    check:prepared => assert.equal(new URL(prepared.call.arguments.url).pathname,`/v1.0/me/drive/items/${'i'.repeat(1025)}`)},
  {arguments:{action:'messages',cursor:`https://graph.microsoft.com/v1.0/me/messages?$skiptoken=${'c'.repeat(4097)}`},
    check:prepared => {
      assert.equal(prepared.call.arguments.url,`https://graph.microsoft.com/v1.0/me/messages?$skiptoken=${'c'.repeat(4097)}`);
      assert.equal(prepared.state.pagination,undefined);
    }},
];
const invalidCases = [0,-1,1.5,null,Number.MAX_SAFE_INTEGER+1].map(limit => ({action:'messages',limit}));

runtimeTest('larger requests preserve source values and endpoint budgets', async () => {
  const run = adapter();
  for (const entry of largeCases) {
    const arguments_ = structuredClone(entry.arguments), original = structuredClone(arguments_);
    const prepared = plain(await run({step:entry.step ?? 0,arguments:arguments_,outcome:entry.outcome}));
    assert.deepEqual(arguments_,original);
    entry.check(prepared);
    assert.equal(prepared.call.operation,'http.request');
    if (prepared.state?.pagination) {
      const cursor = `https://graph.microsoft.com/v1.0${prepared.state.path}?$skiptoken=next`;
      const page = plain(await run({step:1,arguments:arguments_,state:prepared.state,
        outcome:outcome({value:[], '@odata.nextLink':cursor})}));
      assert.deepEqual(page.result.pagination,prepared.state.pagination);
      assert.equal(page.result.next,cursor);
      assert.equal(page.call,undefined);
    }
  }
});

runtimeTest('larger requests reach the production adapter unchanged', async () => {
  const schema = extension.tools[0].handler.parameters;
  assert.deepEqual(validateSchema(schema,[...largeCases.map(entry => entry.arguments),...invalidCases]),
    [...largeCases.map(() => true),...invalidCases.map(args => args.limit === Number.MAX_SAFE_INTEGER+1)]);
  for (const entry of largeCases) {
    const input = await packageInput(new URL('./',import.meta.url),'request',
      {step:entry.step ?? 0,arguments:entry.arguments,outcome:entry.outcome});

    const output = execute(input);
    assert.equal(output.result.Err,undefined);
    entry.check(output.result.Ok);
    assert.equal(output.calls.length,0);
  }
  const rejected=execute({...fixture,input:{step:0,arguments:invalidCases.at(-1)}});
  assert.equal(rejected.result.Ok,undefined);
  assert.match(rejected.result.Err.message,/positive safe integer/);
});


runtimeTest('rejects non-object response envelopes', async () => {
  const request = adapter();
  for (const value of [null, true, 42, 'text', []]) {
    const output = plain(await request({step:1, arguments:{action:'discover'},
      outcome:{data:{status:200,headers:{},body:JSON.stringify(value)}}}));
    assert.equal(output.result.isError, true);
    assert.equal(output.result.error.code, 'unavailable');
    assert.equal(output.call, undefined);
  }
});

runtimeTest('rejects malformed envelopes in production', async () => {
  for (const value of [null, true, 42, 'text', []]) {
    const input = await packageInput(new URL('./', import.meta.url), 'request',
      {step:1,arguments:{action:'discover'},outcome:{data:{status:200,headers:{},body:JSON.stringify(value)}}});
    const output = execute(input);
    assert.equal(output.result.Err, undefined);
    assert.equal(output.result.Ok.result.isError, true);
    assert.equal(output.result.Ok.result.error.code, 'unavailable');
    assert.equal(output.calls.length, 0);
  }
});

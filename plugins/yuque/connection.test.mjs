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

runtimeTest('reads original documents without guessing pagination',()=>{
 const request=adapter();
 const document=new URL(request({step:0,arguments:{action:'doc',repo:'owner/book',id:'entry/quoted'}}).call.arguments.url);
 assert.equal(document.origin,'https://www.yuque.com'); assert.equal(document.pathname,'/api/v2/repos/owner/book/docs/entry%2Fquoted');
 assert.equal(document.searchParams.get('raw'),'1');
 const page=plain(request({step:1,outcome:outcome({data:{body:'原始正文'},meta:{offset:4}})}));
 assert.equal(page.result.data.body,'原始正文'); assert.deepEqual(page.result.meta,{offset:4});
 assert.equal(page.result.untrusted,true);
 for(const repo of ['../secret','owner/../secret','owner/book/extra']) assert.throws(()=>request({step:0,arguments:{action:'docs',repo}}));
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


runtimeTest('provides localized protected connector fields', () => {
  const connector = extension.connectors[0];
  assert.equal(connector.fields[0].help, 'https://www.yuque.com/settings/tokens');
  assert.equal(connector.fields[0].secret, true);
  assert.equal(connector.fields[0].default, undefined);
  assert.equal(Object.keys(connector.fields[0].locales).length, 10);
});

const targets = [
  {args:{action:'search',query:'guide'},required:['query']},
  {args:{action:'repos',user:'owner'},required:['user']},
  {args:{action:'docs',repo:'owner/book'},required:['repo']},
  {args:{action:'toc',repo:'owner/book'},required:['repo']},
  {args:{action:'doc',repo:'owner/book',id:'document'},required:['repo','id']},
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
  {arguments:{action:'search',query:'q'.repeat(4097),page:100001},
    check:prepared => {
      const url = new URL(prepared.call.arguments.url);
      assert.equal(url.searchParams.get('q'),'q'.repeat(4097));
      assert.equal(url.searchParams.get('page'),'100001');
    }},
  {arguments:{action:'repos',user:'u'.repeat(1025),offset:1000001},
    check:prepared => {
      const url = new URL(prepared.call.arguments.url);
      assert.equal(url.pathname,`/api/v2/users/${'u'.repeat(1025)}/repos`);
      assert.equal(url.searchParams.get('offset'),'1000001');
    }},
  {arguments:{action:'doc',repo:`owner/${'r'.repeat(1025)}`,id:'i'.repeat(1025)},
    check:prepared => assert.equal(new URL(prepared.call.arguments.url).pathname,
      `/api/v2/repos/owner/${'r'.repeat(1025)}/docs/${'i'.repeat(1025)}`)},
];
const invalidCases = [0,-1,1.5,Number.MAX_SAFE_INTEGER+1].map(page => ({action:'search',query:'q',page}));

runtimeTest('larger requests preserve source values and endpoint budgets', async () => {
  const run = adapter();
  for (const entry of largeCases) {
    const arguments_ = structuredClone(entry.arguments), original = structuredClone(arguments_);
    const prepared = plain(await run({step:entry.step ?? 0,arguments:arguments_,outcome:entry.outcome}));
    assert.deepEqual(arguments_,original);
    entry.check(prepared);
    assert.equal(prepared.call.operation,'http.request');
  }
});

runtimeTest('larger requests reach the production adapter unchanged', async () => {
  const schema = extension.tools[0].handler.parameters;
  assert.deepEqual(validateSchema(schema,[...largeCases.map(entry => entry.arguments),...invalidCases]),
    [...largeCases.map(() => true),...invalidCases.map(args => args.page === Number.MAX_SAFE_INTEGER+1)]);
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
  assert.match(rejected.result.Err.message,/page must be a safe integer at least 1/);
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

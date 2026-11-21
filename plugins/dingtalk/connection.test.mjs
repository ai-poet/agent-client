import {execute, packageInput, runtimeTest, validateSchema} from '../scripts/test-host.mjs';
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
const manifest=JSON.parse(readFileSync(new URL('./plugin.json',import.meta.url)));
const extension=manifest.extensions['dev.sailry.platform'];
const fixture = await packageInput(new URL('./',import.meta.url),'request',{});
fixture.replies = [{command:{kind:'read_plugin_settings',data:{package:fixture.context.package}},
  response:{Ok:{kind:'plugin_settings',data:{package:fixture.context.package,
    values:{operator:'operator-id'},configured:[],keepable:[],ready:true}}}}];
const plain=value=>JSON.parse(JSON.stringify(value));
function adapter() {
  return async input => {
    const output = execute({...fixture,input,replies:input.step === 0 ? fixture.replies : []});
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

test('separates app credentials from ordinary settings',()=>{
 const schema=JSON.parse(readFileSync(new URL('./dev.sailry.platform/settings.json',import.meta.url)));
 const connector=extension.connectors[0];
 assert.deepEqual(Object.keys(schema.properties),['operator']);
 assert.deepEqual(schema.required,[]);
 assert.equal(connector.auth[0].kind,'application');
 assert.deepEqual(connector.auth[0].fields,{client_id:'app_id',client_secret:'app_secret'});
 assert.equal(connector.auth[0].token.url,'https://api.dingtalk.com/v1.0/oauth2/accessToken');
 assert.equal(connector.fields.find(field=>field.name==='app_secret').secret,true);
 assert.equal(connector.fields.find(field=>field.name==='app_id').secret,false);
 assert.equal(extension.oauth,undefined);
});

runtimeTest('uses configured operator and preserves page tokens',async()=>{
 const request=adapter();
 const initial=await request({step:0,arguments:{action:'nodes',parent:'parent',cursor:'opaque+next',limit:7}});
 assert.equal(initial.call.arguments.connector,'dingtalk');
 assert.deepEqual(plain(initial.call.arguments.headers),{Accept:'application/json'});
 const url=new URL(initial.call.arguments.url);
 assert.equal(url.origin,'https://api.dingtalk.com'); assert.equal(url.pathname,'/v2.0/wiki/nodes');
 assert.equal(url.searchParams.get('operatorId'),'operator-id'); assert.equal(url.searchParams.get('parentNodeId'),'parent');
 assert.equal(url.searchParams.get('nextToken'),'opaque+next'); assert.equal(url.searchParams.get('maxResults'),'7');
 const file=await request({step:0,arguments:{action:'files',space:'space',parent:'folder',operator:'explicit'}});
 assert.equal(file.call.arguments.connector,'dingtalk');
 const files=new URL(file.call.arguments.url); assert.equal(files.pathname,'/v1.0/drive/spaces/space/files');
 assert.equal(files.searchParams.get('unionId'),'explicit'); assert.equal(files.searchParams.get('parentId'),'folder');
 const page=plain(await request({step:1,outcome:outcome({nodes:[{id:'one'}],nextToken:'tail',hasMore:true})}));
 assert.equal(page.result.next,'tail'); assert.equal(page.result.partial,true);
 await assert.rejects(request({step:0,arguments:{action:'node',id:'..'}}));
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
  {args:{action:'workspaces'},required:[]},
  {args:{action:'nodes',parent:'root'},required:['parent']},
  {args:{action:'node',id:'node'},required:['id']},
  {args:{action:'files',space:'space',parent:'folder'},required:['space','parent']},
];
runtimeTest('action targets reach the handler with their required fields', async () => {
  const request = adapter();
  for (const {args,required} of targets) {
    assert.equal((await request({step:0,arguments:args})).call.operation,'http.request');
    for (const field of required) {
      const incomplete = {...args}; delete incomplete[field];
      await assert.rejects(request({step:0,arguments:incomplete}), `${args.action}: ${field}`);
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
  {arguments:{action:'nodes',operator:'o'.repeat(1025),parent:'p'.repeat(1025),cursor:'c'.repeat(4097),limit:1001},
    check:prepared => {
      const url = new URL(prepared.call.arguments.url);
      assert.equal(url.searchParams.get('operatorId'),'o'.repeat(1025));
      assert.equal(url.searchParams.get('parentNodeId'),'p'.repeat(1025));
      assert.equal(url.searchParams.get('nextToken'),'c'.repeat(4097));
      assert.equal(url.searchParams.get('maxResults'),'100');
      assert.deepEqual(plain(prepared.state.pagination),{requested_limit:1001,page_size:100});
    }},
];
const invalidCases = [0,-1,1.5,null,Number.MAX_SAFE_INTEGER+1].map(limit => ({action:'nodes',parent:'root',limit}));

runtimeTest('larger requests preserve source values and endpoint budgets', async () => {
  const run = adapter();
  for (const entry of largeCases) {
    const arguments_ = structuredClone(entry.arguments), original = structuredClone(arguments_);
    const prepared = plain(await run({step:entry.step ?? 0,arguments:arguments_,outcome:entry.outcome}));
    assert.deepEqual(arguments_,original);
    entry.check(prepared);
    assert.equal(prepared.call.operation,'http.request');
    if (prepared.state?.pagination) {
      const page = plain(await run({step:1,arguments:arguments_,state:prepared.state,
        outcome:outcome({nextToken:'next',hasMore:true})}));
      assert.deepEqual(page.result.pagination,prepared.state.pagination);
      assert.equal(page.result.next,'next');
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
    input.replies = [{command:{kind:'read_plugin_settings',data:{package:input.context.package}},
      response:{Ok:{kind:'plugin_settings',data:{package:input.context.package,
        values:{operator:'operator-id'},configured:[],keepable:[],ready:true}}}}];
    const output = execute(input);
    assert.equal(output.result.Err,undefined);
    entry.check(output.result.Ok);
    assert.equal(output.calls.length,1);
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

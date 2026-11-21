import {execute, packageInput, runtimeTest, validateSchema} from '../scripts/test-host.mjs';
import test from 'node:test';
import assert from 'node:assert/strict';
import {existsSync, readFileSync} from 'node:fs';

const manifest = JSON.parse(readFileSync(new URL('./plugin.json', import.meta.url)));
const extension = manifest.extensions['dev.sailry.platform'];
const connector = extension.connectors[0];
const fixture = await packageInput(new URL('./',import.meta.url),'read',{});
function read(input) {
  const output = execute({...fixture,input});
  if (output.result.Err) throw new Error(output.result.Err.message);
  return output.result.Ok;
}
const plain = value => JSON.parse(JSON.stringify(value));
const outcome = (body, status = 200) => ({data:{status, headers:{}, body:JSON.stringify(body)}});

test('declares PAT REST without an automatic MCP connection', () => {
  assert.equal(manifest.name, 'figma');
  assert.equal(manifest.version, '0.1.11');
  assert.equal(extension.api_version, 'v1');
  assert.equal(existsSync(new URL('./mcp.json', import.meta.url)), false);
  assert.deepEqual(extension.actions, ['http.request']);
  assert.deepEqual(extension.tools[0].handler.flow.operations, ['http.request']);
  assert.equal(extension.tools[0].handler.parameters.additionalProperties, false);
  assert.deepEqual(extension.tools[0].handler.parameters.properties.action.enum, ['file','nodes']);
  assert.deepEqual(connector.inject,[{origin:'https://api.figma.com',header:'X-Figma-Token'}]);
  assert.equal(connector.fields[0].secret,true);
  assert.equal(connector.fields[0].default,undefined);
  assert.equal(Object.keys(connector.fields[0].locales).length,10);
  assert.equal(connector.fields[0].help,'https://developers.figma.com/docs/rest-api/personal-access-tokens/');
});

runtimeTest('reads exact files and nodes with bounded parameters', () => {
  const file = plain(read({step:0, arguments:{action:'file', file_key:'file-key', version:'9007199254740993', depth:2}}));
  assert.equal(file.call.operation, 'http.request');
  assert.equal(file.call.arguments.method, 'GET');
  assert.equal(file.call.arguments.connector, 'figma');
  assert.equal(file.call.arguments.body, null);
  assert.deepEqual(file.call.arguments.headers, {Accept:'application/json'});
  assert.equal(file.call.arguments.url, 'https://api.figma.com/v1/files/file-key?version=9007199254740993&depth=2');
  const nodes = new URL(read({step:0, arguments:{action:'nodes', file_key:'file-key', ids:['1:2','I3:4;5:6']}}).call.arguments.url);
  assert.equal(nodes.origin, 'https://api.figma.com');
  assert.equal(nodes.pathname, '/v1/files/file-key/nodes');
  assert.equal(nodes.searchParams.get('ids'), '1:2,I3:4;5:6');
  for (const args of [
    {action:'edit',file_key:'file-key'}, {action:'file',file_key:'../secret'},
    {action:'file',file_key:'https://evil.test'}, {action:'file',file_key:'file-key',url:'https://evil.test'},
    {action:'file',file_key:'file-key',token:'secret'}, {action:'file',file_key:'file-key',headers:{}},
    {action:'file',file_key:'file-key',ids:['1:2']}, {action:'nodes',file_key:'file-key'},
    {action:'nodes',file_key:'file-key',ids:[]}, {action:'nodes',file_key:'file-key',ids:['1:2','1:2']},
    {action:'nodes',file_key:'file-key',ids:['1,2']}, {action:'nodes',file_key:'file-key',ids:['1\n2']},
    {action:'file',file_key:'file-key',version:9007199254740993},
    ...[0,1.5,null].map(depth => ({action:'file',file_key:'file-key',depth})),
  ]) assert.throws(() => read({step:0, arguments:args}));
});

runtimeTest('preserves JSON metadata and missing nodes honestly', () => {
  const data = {name:'Actual file', version:'9007199254740993', document:{id:'0:0', children:[]}};
  const result = plain(read({step:1, arguments:{action:'file',depth:1}, outcome:outcome(data)}));
  assert.deepEqual(result.result, {data,untrusted:true,depth:1,partial:true});
  const nodes = {version:'9007199254740993', nodes:{'1:2':null,'3:4':{document:{id:'3:4'}}}};
  assert.deepEqual(plain(read({step:1,arguments:{action:'nodes'},outcome:outcome(nodes)})).result,
    {data:nodes,untrusted:true,depth:null,partial:true});
});

runtimeTest('preserves errors without retry or private response bodies', () => {
  const uncertain = {isError:true,error:{code:'outcome_unknown',message:'interrupted'}};
  assert.deepEqual(plain(read({step:1,outcome:uncertain})), {result:uncertain});
  for (const status of [401,403,429]) {
    const reply = {data:{status,body:'private error marker',headers:{'retry-after':['12']}}};
    const result = plain(read({step:1,arguments:{action:'file'},outcome:reply}));
    assert.equal(result.result.isError, true);
    assert.equal(result.result.status, status);
    assert.equal(result.result.retry_after, '12');
    assert.equal(result.call, undefined);
    assert.equal(JSON.stringify(result).includes('private error marker'), false);
  }
  for (const reply of [outcome([]),outcome({}),{data:{status:200,body:'invalid'}},outcome({nodes:[]})]) {
    assert.equal(read({step:1,arguments:{action:'nodes'},outcome:reply}).result.isError, true);
  }
});

const targets = [
  {args:{action:'file',file_key:'file-key'},required:['file_key']},
  {args:{action:'nodes',file_key:'file-key',ids:['1:2']},required:['file_key','ids']},
];
runtimeTest('action targets reach the handler with their required fields', () => {
  for (const {args,required} of targets) {
    assert.equal(read({step:0,arguments:args}).call.operation,'http.request');
    for (const field of required) {
      const incomplete = {...args}; delete incomplete[field];
      assert.throws(() => read({step:0,arguments:incomplete}), `${args.action}: ${field}`);
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
  values.push({action:'file',file_key:'file-key',ids:['1:2']}); expected.push(false);
  assert.deepEqual(validateSchema(extension.tools[0].handler.parameters,values),expected);
});

const largeCases = [
  {arguments:{action:'file',file_key:'k'.repeat(129),version:'v'.repeat(129),depth:101},
    check:prepared => {
      const url = new URL(prepared.call.arguments.url);
      assert.equal(url.pathname,`/v1/files/${'k'.repeat(129)}`);
      assert.equal(url.searchParams.get('version'),'v'.repeat(129));
      assert.equal(url.searchParams.get('depth'),'101');
    }},
  {arguments:{action:'nodes',file_key:'file',ids:Array.from({length:101},(_,index) => `1:${index}`)},
    check:prepared => assert.equal(new URL(prepared.call.arguments.url).searchParams.get('ids'),
      Array.from({length:101},(_,index) => `1:${index}`).join(','))},
  {arguments:{action:'nodes',file_key:'file',ids:['n'.repeat(1025)]},
    check:prepared => assert.equal(new URL(prepared.call.arguments.url).searchParams.get('ids'),'n'.repeat(1025))},
];
const invalidCases = [{action:'file',file_key:'file',depth:1.5},
  {action:'file',file_key:'file',depth:Number.MAX_SAFE_INTEGER+1}];

runtimeTest('larger requests preserve source values and endpoint budgets', async () => {
  const run = read;
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
    [...largeCases.map(() => true),...invalidCases.map(args => args.depth === Number.MAX_SAFE_INTEGER+1)]);
  for (const entry of largeCases) {
    const input = await packageInput(new URL('./',import.meta.url),'read',
      {step:entry.step ?? 0,arguments:entry.arguments,outcome:entry.outcome});

    const output = execute(input);
    assert.equal(output.result.Err,undefined);
    entry.check(output.result.Ok);
    assert.equal(output.calls.length,0);
  }
  const rejected=execute({...fixture,input:{step:0,arguments:invalidCases.at(-1)}});
  assert.equal(rejected.result.Ok,undefined);
  assert.match(rejected.result.Err.message,/Invalid depth/);
});

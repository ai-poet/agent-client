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

runtimeTest('mail and document reads preserve paging and text',()=>{
 const request=adapter();
 const mail=plain(request({step:0,arguments:{action:'messages',query:'from:朋友@example.com',cursor:'next&token',limit:12}}));
 const url=new URL(mail.call.arguments.url);
 assert.equal(url.origin,'https://gmail.googleapis.com'); assert.equal(url.pathname,'/gmail/v1/users/me/messages');
 assert.equal(url.searchParams.get('q'),'from:朋友@example.com'); assert.equal(url.searchParams.get('pageToken'),'next&token');
 assert.equal(mail.call.arguments.connector,'google'); assert.equal(mail.call.arguments.method,'GET');
 const exportRead=request({step:0,arguments:{action:'export_doc',id:'doc/quoted?'}});
 assert.match(exportRead.call.arguments.url,/\/drive\/v3\/files\/doc%2Fquoted%3F\/export\?/);
 const result=plain(request({step:1,state:exportRead.state,outcome:{data:{status:200,body:'原文\nSecond paragraph'}}}));
 assert.deepEqual(result.result,{text:'原文\nSecond paragraph',untrusted:true});
 const page=plain(request({step:1,outcome:outcome({files:[{id:'one'}],nextPageToken:'next',incompleteSearch:true})}));
 assert.equal(page.result.next,'next'); assert.equal(page.result.partial,true); assert.equal(page.result.untrusted,true);
 assert.throws(()=>request({step:0,arguments:{action:'file',id:'..'}}));
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

const targets = ['messages','message','events','event','files','file','export_doc'].map(action => ({
  args:{action,...(['message','event','file','export_doc'].includes(action) ? {id:'target'} : {})},
  required:['message','event','file','export_doc'].includes(action) ? ['id'] : [],
}));
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
  ...[['messages','maxResults',500],['events','maxResults',2500],['files','pageSize',1000]].map(([action,field,size]) => ({
    arguments:{action,query:'q'.repeat(4097),cursor:'c'.repeat(4097),limit:3001},
    check:prepared => {
      const url = new URL(prepared.call.arguments.url);
      assert.equal(url.searchParams.get(field),String(size));
      assert.equal(url.searchParams.get('q'),'q'.repeat(4097));
      assert.equal(url.searchParams.get('pageToken'),'c'.repeat(4097));
      assert.deepEqual(plain(prepared.state.pagination),{requested_limit:3001,page_size:size});
    }})),
  {arguments:{action:'event',id:'i'.repeat(1025),calendar:'c'.repeat(1025)},
    check:prepared => assert.equal(new URL(prepared.call.arguments.url).pathname,
      `/calendar/v3/calendars/${'c'.repeat(1025)}/events/${'i'.repeat(1025)}`)},
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
      const page = plain(await run({step:1,arguments:arguments_,state:prepared.state,
        outcome:outcome({nextPageToken:'next'})}));
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

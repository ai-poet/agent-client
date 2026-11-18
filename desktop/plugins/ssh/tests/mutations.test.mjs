import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFile} from 'node:fs/promises';

function deferred(){let resolve,reject;const promise=new Promise((accept,fail)=>{resolve=accept;reject=fail;});return {promise,resolve,reject};}
const changed={Ok:{kind:'ssh_outcome',data:{kind:'files_changed'}}};
const command=(path,action={operation:'remove'})=>({kind:'modify_ssh_file',data:{profile:'captured',expected_revision:7,path,action}});
const settle=async()=>{for(let step=0;step<24;step++)await Promise.resolve();};

async function fixture() {
  const context=vm.createContext({}),prepared=[],admitted=[],forgotten=[],inspected=[],held=new Map(),outcomes=new Map(),cache=new Map();
  const modules={
    'sailry/sdk':{
      prepareRequest(value){const id=`request-${prepared.length}`;prepared.push({id,value});return id;},
      completeRequest(id){admitted.push(id);const wait=deferred();held.set(id,wait);return wait.promise;},
      forgetRequest:id=>forgotten.push(id),
    },
    'sailry/connections':{requestOutcome:async id=>{inspected.push(id);return outcomes.get(id);}},
  };
  const resolve=async name=>{
    if(cache.has(name))return cache.get(name);
    const local=name.startsWith('./');
    const module=local
      ?new vm.SourceTextModule(await readFile(new URL(`../dev.sailry.platform/desktop/${name.slice(2)}`,import.meta.url),'utf8'),{context})
      :new vm.SyntheticModule(Object.keys(modules[name]),function(){for(const [key,value]of Object.entries(modules[name]))this.setExport(key,value);},{context});
    cache.set(name,module);if(local)await module.link(resolve);return module;
  };
  const module=await resolve('./mutations.js');await module.evaluate();
  return {Mutations:module.namespace.Mutations,prepared,admitted,forgotten,inspected,held,outcomes,cx:{notify(){}}};
}

test('independent changes all start before any completion',async()=>{
  const setup=await fixture(),commands=Array.from({length:6},(_,index)=>command(`/remote/${index}`));
  const batch=new setup.Mutations(commands,()=>true),loading=batch.start(setup.cx);
  assert.deepEqual(setup.admitted,setup.prepared.map(value=>value.id));
  assert.equal(setup.admitted.length,6);assert.equal(batch.running,true);assert.equal(batch.done,false);
  for(const wait of [...setup.held.values()].reverse())wait.resolve(changed);
  await loading;assert.equal(batch.done,true);assert.equal(batch.error,null);assert.equal(batch.output.data.kind,'files_changed');
  assert.deepEqual(Array.from(batch.requests,request=>request.command.data.path),commands.map(value=>value.data.path));
  batch.release();assert.deepEqual(setup.forgotten,setup.prepared.map(value=>value.id));
});

test('partial and uncertain results keep their original identities',async()=>{
  const setup=await fixture(),batch=new setup.Mutations(['/ok','/denied','/uncertain'].map(path=>command(path)),()=>true);
  const loading=batch.start(setup.cx),ids=setup.prepared.map(value=>value.id);
  assert.equal(setup.admitted.length,3);
  setup.held.get(ids[0]).resolve(changed);
  setup.held.get(ids[1]).resolve({Err:{code:'permission_denied',message:'Remote path is read-only'}});
  setup.held.get(ids[2]).reject(Error('transport lost after admission'));
  await loading;assert.equal(batch.done,false);assert.equal(batch.unknown,true);assert.equal(batch.output,null);
  assert.equal(batch.requests[0].output.data.kind,'files_changed');assert.equal(batch.requests[1].error.code,'permission_denied');
  await batch.start(setup.cx);assert.equal(setup.admitted.length,3,'starting again never repeats admitted writes');
  setup.outcomes.set(ids[2],{kind:'admitted'});await batch.check(setup.cx);assert.equal(batch.unknown,true);
  setup.outcomes.set(ids[2],{kind:'completed',data:changed});await batch.check(setup.cx);
  assert.equal(batch.done,true);assert.equal(batch.error.code,'permission_denied');assert.equal(batch.output,null);
  assert.equal(batch.requests[2].output.data.kind,'files_changed');assert.deepEqual(setup.inspected,[ids[2],ids[2]]);
  assert.equal(setup.prepared.length,3);assert.equal(setup.admitted.length,3);
  batch.release();assert.deepEqual(setup.forgotten,ids);
});

test('scope changes stop dependent admissions only',async()=>{
  const setup=await fixture();let current=true;
  const batch=new setup.Mutations([
    command('/source',{operation:'copy',destination:'/target'}),command('/target/child'),command('/independent'),
  ],()=>current);
  const loading=batch.start(setup.cx),ids=setup.prepared.map(value=>value.id);
  assert.deepEqual(setup.admitted,[ids[0],ids[2]],'unrelated work starts while its sibling waits for an actual dependency');
  current=false;setup.held.get(ids[0]).resolve(changed);setup.held.get(ids[2]).resolve(changed);
  await loading;assert.deepEqual(setup.admitted,[ids[0],ids[2]]);assert.equal(batch.done,true);
  assert.equal(batch.requests[1].error.code,'cancelled');assert.equal(batch.requests[0].output.data.kind,'files_changed');
});

test('related paths retain order without blocking independent paths',async()=>{
  const setup=await fixture(),batch=new setup.Mutations([
    command('/tree/item'),command('/tree/./item/child'),command('/tree/other'),
  ],()=>true);
  const loading=batch.start(setup.cx),ids=setup.prepared.map(value=>value.id);
  assert.deepEqual(setup.admitted,[ids[0],ids[2]]);
  setup.held.get(ids[0]).resolve(changed);await settle();
  assert.deepEqual(setup.admitted,[ids[0],ids[2],ids[1]],'only the overlapping resource waits');
  setup.held.get(ids[1]).resolve(changed);setup.held.get(ids[2]).resolve(changed);await loading;
  assert.equal(batch.output.data.kind,'files_changed');
});

test('uncertain dependencies never admit or replay later writes',async()=>{
  const setup=await fixture(),batch=new setup.Mutations([
    command('/source',{operation:'move',destination:'/target'}),command('/target/child'),command('/independent'),
  ],()=>true);
  const loading=batch.start(setup.cx),ids=setup.prepared.map(value=>value.id);
  setup.held.get(ids[0]).reject(Error('transport lost'));setup.held.get(ids[2]).resolve(changed);await loading;
  assert.equal(batch.requests[1].error.code,'cancelled');assert.equal(batch.unknown,true);
  setup.outcomes.set(ids[0],{kind:'completed',data:changed});await batch.check(setup.cx);
  assert.equal(batch.done,true);assert.deepEqual(setup.admitted,[ids[0],ids[2]],'inspection cannot dispatch the blocked dependent write');
  assert.equal(batch.requests[2].output.data.kind,'files_changed');
});

test('copies share a read source without a write gate',async()=>{
  const setup=await fixture(),batch=new setup.Mutations([
    command('/source',{operation:'copy',destination:'/one'}),command('/source',{operation:'copy',destination:'/two'}),
  ],()=>true),loading=batch.start(setup.cx);
  assert.equal(setup.admitted.length,2,'shared read access does not serialize distinct destinations');
  for(const wait of setup.held.values())wait.resolve(changed);await loading;assert.equal(batch.output.data.kind,'files_changed');
});

test('host key outcomes stay explicit instead of reporting success',async()=>{
  const setup=await fixture(),batch=new setup.Mutations(['/one','/two'].map(path=>command(path)),()=>true),loading=batch.start(setup.cx);
  const key={kind:'host_key_required',key:'captured-fingerprint'};
  for(const wait of setup.held.values())wait.resolve({Ok:{kind:'ssh_outcome',data:key}});
  await loading;assert.equal(batch.done,true);assert.equal(batch.output.data,key);assert.equal(batch.requests.length,2);
});

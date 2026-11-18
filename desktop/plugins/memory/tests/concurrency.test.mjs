import test from 'node:test';
import assert from 'node:assert/strict';
import {fixture,entry,id,project} from './store.mjs';

const settings={enabled:true,auto_write:true,context_bytes:65536,review_after_days:90};
const plain=value=>JSON.parse(JSON.stringify(value));
const argumentsFor=(body,changes={})=>({expected_revision:0,kind:'project',title:body,body,...changes});
async function setup(entries=[]) {
  const f=await fixture(); f.configure(settings); f.seed(entries);
  await f.main.initialize({tools:['search_memory','save_memory','forget_memory','consolidate_memories'],project,resources:false});
  return f;
}
async function start(f,name,args) {
  return {name,args,step:0,transition:await f.main[name]({arguments:args,step:0})};
}
function commit(f,job) {
  assert.equal(job.transition.call.operation,'storage.transaction');
  try { return f.apply(job.transition.call.arguments.operations); }
  catch (error) { return {isError:true,error:{code:error.code,message:error.message,details:error.details}}; }
}
async function resume(f,job,outcome) {
  job.step++;
  job.transition=await f.main[job.name]({arguments:job.args,step:job.step,state:job.transition.state,outcome});
  return job.transition;
}
async function authorize(f,ids) {
  const results=[];
  for (const id of ids) results.push({name:'search_memory',response:await f.main.search({query:id})});
  f.main.beforeModel({results});
}

test('merges six creates while retaining each business identifier',async()=>{
  const f=await setup(), jobs=[];
  for (let index=0;index<6;index++) jobs.push(await start(f,'save',argumentsFor(`Verified constraint ${index}`)));
  const identifiers=jobs.map(job=>job.transition.state.id);
  assert.equal(new Set(identifiers).size,6);
  let remaining=jobs;
  while (remaining.length) {
    const outcomes=remaining.map(job=>commit(f,job)), next=[];
    for (const [index,job] of remaining.entries()) {
      await resume(f,job,outcomes[index]);
      if (job.transition.call) {
        assert.equal(job.transition.state.id,identifiers[jobs.indexOf(job)]);
        next.push(job);
      } else assert.equal(job.transition.result.updated[0].id,identifiers[jobs.indexOf(job)]);
    }
    remaining=next;
  }
  assert.equal(f.get('memory/catalog').value.active,6);
  assert.deepEqual(Object.values(f.get('memory/catalog').value.pages),[2,2,2]);
  assert.equal(f.indexes.size,6);
  for (const [index,identifier] of identifiers.entries()) assert.equal((await f.storage.readMemory(identifier)).body,`Verified constraint ${index}`);
});

test('replans unrelated updates and deletion without lost bodies',async()=>{
  const f=await setup([entry(1,'Original first'),entry(2,'Original second'),entry(3,'Delete this')]);
  await authorize(f,[id(1),id(2)]);
  const jobs=[await start(f,'save',argumentsFor('Updated first',{id:id(1),expected_revision:1})),
    await start(f,'save',argumentsFor('Updated second',{id:id(2),expected_revision:1})),
    await start(f,'forget',{id:id(3),expected_revision:1})];
  const first=jobs.map(job=>commit(f,job));
  for (const [index,job] of jobs.entries()) {
    let outcome=first[index];
    while ((await resume(f,job,outcome)).call) outcome=commit(f,job);
    assert.equal(job.transition.result.isError,undefined);
    assert.equal(job.transition.result.receipt.action,job.name==='save' ? 'updated' : 'deleted');
    if (job.name==='save') assert.equal(job.transition.result.receipt.body,job.args.body);
  }
  assert.equal((await f.storage.readMemory(id(1))).body,'Updated first');
  assert.equal((await f.storage.readMemory(id(2))).body,'Updated second');
  assert.equal(f.get('memory/entry/'+id(3)).present,false);
  assert.equal(f.get('memory/catalog').value.active,2);
});

test('recognizes equivalent concurrent creates without duplicating them',async()=>{
  const f=await setup(), first=await start(f,'save',argumentsFor('Use concise replies'));
  const second=await start(f,'save',argumentsFor('use  concise\nreplies',{title:'Concise preference'}));
  const winner=commit(f,first), rejected=commit(f,second);
  await resume(f,first,winner);
  await resume(f,second,rejected);
  assert.equal(second.transition.result.context.status,'changed');
  assert.equal(second.transition.result.context.saved,false);
  assert.equal(second.transition.result.receipt,undefined);
  const latest=JSON.parse(second.transition.result.context.latest[0].parts[0].text);
  assert.equal(latest.summary.id,first.transition.result.updated[0].id);
  assert.equal(latest.body,'Use concise replies');
  assert.equal(f.get('memory/catalog').value.active,1);
});

test('returns latest content for automatic intent reconciliation',async()=>{
  const f=await setup([entry(1,'Keep the original condition')]);
  await authorize(f,[id(1)]);
  const job=await start(f,'save',argumentsFor('Original condition plus new exception',{id:id(1),expected_revision:1}));
  const current=await f.storage.readMemory(id(1)); current.body+=' and another verified fact';
  f.apply((await f.mutations.putOperations(current,1)).operations);
  const rejected=commit(f,job);
  assert.equal(rejected.error.details.kind,'plugin_value_conflict');
  const done=await resume(f,job,rejected), changed=done.result.context;
  assert.equal(changed.status,'changed'); assert.equal(changed.saved,false);
  const latest=JSON.parse(changed.latest[0].parts[0].text);
  assert.equal(latest.body,current.body); assert.equal(latest.summary.revision,2);
  assert.equal(latest.body_truncated,false);
  f.main.beforeModel({results:[{name:'save_memory',response:{...rejected,context:changed}}]});
  const reconciled=await start(f,'save',argumentsFor(current.body+' plus new exception',{id:id(1),expected_revision:2}));
  const written=commit(f,reconciled);
  await resume(f,reconciled,written);
  assert.equal(reconciled.transition.result.updated[0].revision,3);
  assert.equal((await f.storage.readMemory(id(1))).body,current.body+' plus new exception');
});

test('preserves deletion when an older save loses CAS',async()=>{
  const f=await setup([entry(1,'The original fact')]); await authorize(f,[id(1)]);
  const job=await start(f,'save',argumentsFor('An older edit',{id:id(1),expected_revision:1}));
  f.apply((await f.mutations.removeOperations(id(1),1)).operations);
  const done=await resume(f,job,commit(f,job));
  assert.equal(done.result.context.status,'deleted');
  assert.equal(done.result.context.saved,false);
  assert.equal(done.result.receipt,undefined);
  assert.deepEqual(plain(done.result.context.deleted),[id(1)]);
  assert.equal(f.get('memory/entry/'+id(1)).present,false);
  assert.equal(f.get('memory/catalog').value.active,0);
});

test('never reuses a tombstoned create identifier',async()=>{
  const f=await setup(), job=await start(f,'save',argumentsFor('Original creation'));
  const identifier=job.transition.state.id;
  f.apply(job.transition.call.arguments.operations);
  f.apply((await f.mutations.removeOperations(identifier,1)).operations);
  const done=await resume(f,job,commit(f,job));
  assert.equal(done.result.context.status,'deleted');
  assert.equal(f.get('memory/entry/'+identifier).present,false);
  assert.equal(f.get('memory/entry/'+identifier).revision,'2');
  assert.equal(f.get('memory/catalog').value.active,0);
});

test('deletion wins over a concurrently changed body',async()=>{
  const f=await setup([entry(1,'The original fact')]);
  const job=await start(f,'forget',{id:id(1),expected_revision:1});
  const current=await f.storage.readMemory(id(1)); current.summary.title='A newer title'; current.body='A newer fact';
  f.apply((await f.mutations.putOperations(current,1)).operations);
  assert.ok((await resume(f,job,commit(f,job))).call);
  await resume(f,job,commit(f,job));
  assert.deepEqual(plain(job.transition.result.removed),[id(1)]);
  assert.deepEqual(plain(job.transition.result.receipt),{
    action:'deleted',title:'A newer title',project,kind:'project',body:'A newer fact',body_truncated:false,
  });
  assert.equal(f.get('memory/entry/'+id(1)).present,false);
});

test('reconciles deletion during consolidation before preparing any write',async()=>{
  for (const removed of [1,2]) {
    const f=await setup([entry(1,'Retained target conditions'),entry(2,'Retained source conditions')]);
    await authorize(f,[id(1),id(2)]);
    const deletion=await f.mutations.removeOperations(id(removed),1);
    let reads=0;
    f.onRead=key=>{
      if (key===`memory/entry/${id(removed)}` && ++reads===2) {
        f.onRead=null;f.apply(deletion.operations);
      }
    };
    const job=await start(f,'consolidate',{id:id(1),expected_revision:1,title:'Combined intent',
      body:'Retain both conditions and the new intent',sources:[{id:id(2),revision:1}]});
    const result=job.transition.result;
    assert.equal(result.status,'changed');assert.equal(result.saved,false);
    assert.equal(job.transition.call,undefined);
    assert.deepEqual(plain(result.deleted),[id(removed)]);
    const remaining=await f.storage.readMemory(id(removed===1 ? 2 : 1));
    assert.equal(remaining.summary.revision,1);
    assert.equal(remaining.summary.archived,false);
    assert.equal(remaining.body,removed===1 ? 'Retained source conditions' : 'Retained target conditions');
    assert.equal(f.get('memory/catalog').value.active,1);
    assert.equal(f.indexes.size,1);
  }
});

test('bounds repeated zero-write conflicts without claiming saved',async()=>{
  const f=await setup(), job=await start(f,'save',argumentsFor('Never committed'));
  for (let step=0;step<8;step++) {
    const head=f.get('memory/catalog'); f.store(head.key,head.value,String(BigInt(head.revision)+1n));
    const outcome=commit(f,job);
    assert.equal(outcome.isError,true);
    await resume(f,job,outcome);
    assert.equal(Boolean(job.transition.call),step<7);
  }
  assert.deepEqual(plain(job.transition.result.context),{status:'pending',saved:false});
  assert.equal(job.transition.result.receipt,undefined);
  assert.equal(f.get('memory/catalog').value.active,0);
  assert.equal(f.indexes.size,0);
});

for (const code of ['outcome_unknown','cancelled','permission_denied','unavailable','internal','revision_conflict']) {
  test(`never resumes an unclassified ${code} failure`,async()=>{
    const f=await setup(), job=await start(f,'save',argumentsFor('No replay'));
    const outcome={isError:true,error:{code,message:'Original failure'}}, before=f.reads.length;
    await resume(f,job,outcome);
    assert.deepEqual(plain(job.transition.result),outcome);
    assert.equal(f.reads.length,before);
    assert.equal(job.transition.call,undefined);
  });
}

test('confirmed snapshots preserve scope and accepted content',async()=>{
  for (const global of [false,true]) {
    const f=await setup();
    const args=argumentsFor('Keep the exact Unicode condition 中文 🙂',{title:'Accepted preference',kind:'feedback',global});
    const job=await start(f,'save',args);
    assert.equal(job.transition.result,undefined);
    const outcome=commit(f,job), before=f.reads.length;
    await resume(f,job,outcome);
    assert.equal(f.reads.length,before);
    assert.equal(job.transition.result.updated.length,1);
    assert.deepEqual(plain(job.transition.result.receipt),{
      action:'created',title:args.title,project:global ? null : project,kind:'feedback',body:args.body,body_truncated:false,
    });
    assert.equal((await f.storage.readMemory(job.transition.result.updated[0].id)).body,args.body);
  }
});

test('merge snapshots select the retained target',async()=>{
  const f=await setup([entry(1,'Target condition',{title:'Target'}),entry(2,'Source condition',{title:'Source'})]);
  await authorize(f,[id(1),id(2)]);
  const args={id:id(1),expected_revision:1,title:'Combined conditions',body:'Target and source conditions 中文 🙂',sources:[{id:id(2),revision:1}]};
  const job=await start(f,'consolidate',args);
  await resume(f,job,commit(f,job));
  assert.equal(job.transition.result.updated[0].id,id(2));
  assert.equal(job.transition.result.updated[0].archived,true);
  assert.deepEqual(plain(job.transition.result.receipt),{
    action:'merged',title:args.title,project,kind:'project',body:args.body,body_truncated:false,
  });
  assert.equal((await f.storage.readMemory(id(1))).body,args.body);
});

test('snapshots require the exact committed target and revision',async()=>{
  for (const changed of ['target','revision']) {
    const f=await setup(),job=await start(f,'save',argumentsFor('Accepted condition'));
    const outcome=commit(f,job),target=job.transition.state.id;
    const stored=outcome.data.find(value=>value.data?.key===`memory/entry/${target}`);
    if (changed==='target') {
      stored.data.key=`memory/entry/${id(123)}`;
      stored.data.value.summary.id=id(123);
    } else stored.data.revision=String(BigInt(stored.data.revision)+1n);
    await resume(f,job,outcome);
    assert.equal(job.transition.result.updated.length,1);
    assert.equal(job.transition.result.receipt,undefined);
    assert.equal(job.transition.result.updated[0].id,changed==='target' ? id(123) : target);
  }
});

test('snapshot bodies reuse the existing Unicode budget',async()=>{
  const f=await setup();f.configure({...settings,context_bytes:2048});
  await f.main.initialize({tools:['save_memory'],project,resources:false});
  const body='Retain conditions 中文 🙂 '.repeat(4000),job=await start(f,'save',argumentsFor(body,{title:'Complete conditions'}));
  await resume(f,job,commit(f,job));
  const snapshot=job.transition.result.receipt;
  assert.equal(snapshot.body_truncated,true);
  assert.ok(snapshot.body.length>0);
  assert.ok(body.startsWith(snapshot.body));
  assert.ok(Buffer.byteLength(snapshot.body)<=2048);
  assert.equal(snapshot.body.includes('\ufffd'),false);
  assert.equal((await f.storage.readMemory(job.transition.result.updated[0].id)).body,body);
});

test('missing deletions have no confirmed snapshot',async()=>{
  const f=await setup(),job=await start(f,'forget',{id:id(1),expected_revision:1});
  assert.deepEqual(plain(job.transition.result),{status:'deleted',saved:false,deleted:[id(1)]});
  assert.equal(job.transition.call,undefined);
});

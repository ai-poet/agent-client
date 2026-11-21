import test from 'node:test';
import assert from 'node:assert/strict';
import {fixture,entry,id,project,fault} from './store.mjs';

const settings={enabled:true,auto_write:true,context_bytes:8192,review_after_days:90};
const initialize=(f,message=null)=>f.main.initialize({tools:['search_memory'],project,resources:true,message});
const tag='Relevant memory: ';
const relevant=result=>Array.from(result.parts).filter(part=>part.startsWith(tag)).map(part=>JSON.parse(part.slice(tag.length)));

for(const kind of ['search']) {
  test(`${kind} reads independent body pages concurrently`,async()=>{
    const f=await fixture(),held=[];
    f.seed(Array.from({length:6},(_,position)=>entry(position+1,`Preference ${position+1}`,{kind:'user'})));
    f.onRead=key=>key.startsWith('memory/body/')?new Promise(resolve=>held.push({key,resolve})):undefined;
    const loading=f.retrieval.searchMemories(project,'');
    for(let step=0;step<64&&held.length<3;step++)await Promise.resolve();
    assert.equal(held.length,3,'all unique pages start before any body page settles');
    assert.equal(new Set(held.map(entry=>entry.key)).size,3);
    for(const pending of [...held].reverse())pending.resolve();
    const result=await loading;assert.equal(result.length,6);
    assert.deepEqual(Array.from(result,value=>value.summary.id),Array.from({length:6},(_,position)=>id(position+1)));
    assert.equal(f.prepared.length,0);assert.equal(f.completed.length,0);
  });
}

test('turns without a message read no storage and add no context',async()=>{
  const f=await fixture();
  f.configure(settings);
  f.seed([entry(1,'First preference',{kind:'user',title:'First'}),entry(2,'Second fact',{title:'Second',updated_at_ms:5})]);
  const before=f.reads.length;
  for (const message of [null,'',' \n']) {
    const result=await initialize(f,message);
    assert.deepEqual(Array.from(result.parts),[]);
    assert.equal(result.instruction,'');
  }
  assert.equal(f.reads.slice(before).some(key=>key.startsWith('memory/body/') || key==='memory/catalog'),false);
  assert.equal(f.prepared.length,0);
});

test('a miss adds no context and never lists titles',async()=>{
  const f=await fixture();
  f.configure(settings);
  f.seed([entry(1,'First preference',{kind:'user',title:'First'}),entry(2,'Second fact',{title:'Second',updated_at_ms:5})]);
  const result=await initialize(f,'Explain the stack trace');
  assert.deepEqual(Array.from(result.parts),[]);
  assert.equal(result.instruction,'');
  assert.equal(JSON.stringify(result).includes('First'),false);
  assert.equal(f.prepared.length,0);
  assert.equal(f.completed.length,0);
});

for (const change of ['body','delete','foreign']) {
  test(`relevant recall recaptures after a concurrent ${change} change`,async()=>{
    const f=await fixture();
    f.configure(settings);
    f.seed([entry(1,'Original preference',{kind:'user'})]);
    const current=await f.storage.readMemory(id(1));
    current.body='Current complete preference';
    const pending=change==='delete' ? await f.mutations.removeOperations(id(1),1)
      : await f.mutations.putOperations(change==='foreign' ? entry(2,'Foreign project',{project:id(9000)}) : current,change==='foreign' ? 0 : 1);
    f.onRead=key=>{if(key.startsWith('memory/body/')){f.onRead=null;f.apply(pending.operations);}};
    // Both versions of the body share this term, which is also a save cue.
    const result=await initialize(f,'preference');
    if (change==='delete') assert.deepEqual(relevant(result),[]);
    else {
      const [value]=relevant(result);
      assert.equal(value.id,id(1));
      assert.equal(value.body,change==='body' ? current.body : 'Original preference');
    }
    assert.equal(relevant(result).length,change==='delete' ? 0 : 1);
    assert.equal(result.parts.length,relevant(result).length+1,'bodies and the cue');
    assert.equal(f.prepared.length,0);
    assert.deepEqual(f.state.reads,[]);
  });
}

test('retries a head change while capturing relevant bodies',async()=>{
  const f=await fixture();
  f.configure(settings);
  f.seed([entry(1,'Original preference',{kind:'user'})]);
  const current=await f.storage.readMemory(id(1));
  current.summary.title='Current title';
  const pending=await f.mutations.putOperations(current,1);
  let heads=0;
  f.onRead=key=>{if(key==='memory/catalog' && ++heads===2)f.apply(pending.operations);};
  assert.deepEqual(relevant(await initialize(f,'original')).map(value=>value.title),['Current title']);
  assert.equal(heads,4);
  assert.equal(f.prepared.length,0);
  assert.equal(f.completed.length,0);
});

test('adds no context after a contended catalog without ending initialization',async()=>{
  const f=await fixture();
  f.configure(settings);
  f.seed([entry(1,'Original preference',{kind:'user'})]);
  let heads=0;
  f.onRead=key=>{
    if(key==='memory/catalog') {
      const head=f.get('memory/catalog');
      f.store(head.key,head.value,String(BigInt(head.revision)+1n));
      heads++;
    }
  };
  const output=await initialize(f,'original');
  assert.deepEqual(Array.from(output.parts),[]);
  assert.equal(output.instruction,'');
  assert.equal(heads,6,"three bounded attempts");
  assert.deepEqual(f.state.reads,[]);
  assert.equal(f.prepared.length,0);
  assert.equal(f.completed.length,0);
});

for (const code of ['cancelled','permission_denied','unavailable','internal']) {
  test(`${code} during recall adds no context and is not retried`,async()=>{
    const f=await fixture();
    f.configure(settings);
    f.seed([entry(1,'Original preference',{kind:'user'})]);
    let heads=0;
    f.onRead=key=>{if(key==='memory/catalog'){heads++;throw fault(code,'Original failure');}};
    const output=await initialize(f,'original');
    assert.deepEqual(Array.from(output.parts),[]);
    assert.equal(heads,1);
    assert.deepEqual(f.state.reads,[]);
  });
}

test('relevance failures add no context',async()=>{
  const f=await fixture();
  f.configure(settings);
  f.seed([entry(1,'Original preference',{kind:'user',title:'Preference'})]);
  f.onRead=key=>{if(key.startsWith('memory/body/'))throw fault('unavailable','Search failed');};
  const output=await initialize(f,'original');
  assert.deepEqual(Array.from(output.parts),[]);
  assert.equal(output.instruction,'');
});

test('invalid stored catalogs add no context and are not retried',async()=>{
  const f=await fixture();
  f.configure(settings);
  f.seed([entry(1,'Original preference',{kind:'user'})]);
  f.store('memory/catalog',{v:2,pages:{}});
  let heads=0;
  f.onRead=key=>{if(key==='memory/catalog')heads++;};
  const output=await initialize(f,'original');
  assert.deepEqual(Array.from(output.parts),[]);
  assert.equal(heads,1);
  assert.equal(f.prepared.length,0);
  assert.equal(f.completed.length,0);
});

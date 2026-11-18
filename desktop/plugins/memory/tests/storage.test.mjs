import test from 'node:test';
import assert from 'node:assert/strict';
import {fixture,entry,id,project} from './store.mjs';
import {readFile} from 'node:fs/promises';

const plain=value=>JSON.parse(JSON.stringify(value));
const commit=async(f,input,revision=0)=>f.apply((await f.mutations.putOperations(input,revision)).operations);

test('reprepares settings after head contention without overwriting changes',async()=>{
  const f=await fixture(),initial=await f.settings.readMemorySettings();
  const draft={...initial,context_bytes:4096};
  const request=await f.settings.prepareMemorySettings(draft);
  await commit(f,entry(1,'Independent entry'));
  const conflict=await f.sdk.completeRequest(request);
  assert.deepEqual(plain(conflict.Err.details),{kind:'plugin_value_conflict'});
  assert.equal((await f.settings.readMemorySettings()).revision,0);
  const next=await f.settings.prepareMemorySettings(draft);
  const saved=await f.sdk.completeRequest(next);
  assert.equal(f.settings.outcome(saved.Ok).context_bytes,4096);
  assert.equal(f.get('memory/catalog').value.active,1);
  const attempts=f.prepared.length;
  await assert.rejects(f.settings.prepareMemorySettings({...draft,enabled:false}),error=>error.code==='revision_conflict');
  assert.equal(f.prepared.length,attempts);
  assert.equal((await f.settings.readMemorySettings()).enabled,true);
});

test('opts only metadata into semantic indexing',async()=>{
  const manifest=JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  const collections=manifest.extensions['dev.sailry.platform'].storage.collections;
  assert.deepEqual(collections.filter(collection=>collection.index?.semantic).map(collection=>collection.prefix),['memory/entry/']);
  assert.equal(collections.filter(collection=>collection.index?.semantic).length,1);
});

test('references complete paged bodies and preserves original semantic queries',async()=>{
  const f=await fixture(), title='Keep the Original Title!';
  await commit(f,entry(1,'Use a complete raw body with conditions',{title}));
  await commit(f,entry(2,'Other body sharing the page'));
  const key='memory/entry/'+id(1), metadata=f.get(key), index=f.indexes.get(key);
  assert.deepEqual(index.source,{key:'memory/body/0',pointer:`/bodies/${id(1)}/body`});
  assert.equal(index.fields[0],title);
  assert.equal(f.get(index.source.key).value.bodies[id(1)].body,'Use a complete raw body with conditions');
  const other=await f.storage.readMemory(id(2));other.body='Updated neighboring body';
  await commit(f,other,1);
  assert.equal(f.get(key).revision,metadata.revision);
  assert.equal(f.get(index.source.key).revision,'3');
  assert.equal(f.get(index.source.key).value.bodies[id(1)].body,'Use a complete raw body with conditions');
  const query='  raw body with conditions!  ';
  await f.retrieval.searchMemories(project,query);
  assert.equal(f.searches.at(-1).query,query);
  assert.equal(f.searches.at(-1).mode,'hybrid');
  assert.notEqual(f.searches.at(-1).terms.join(' '),query);
});

test('allocates shared pages and keeps metadata, index and bodies coherent through removal',async()=>{
  const f=await fixture();
  await commit(f,entry(1,'A retired constraint'));
  await commit(f,entry(2,'Another confirmed exception'));
  assert.equal(f.get('memory/entry/'+id(1)).value.body_page,'0');
  assert.equal(f.get('memory/entry/'+id(2)).value.body_page,'0');
  const original=await f.storage.readMemory(id(2));
  const changed=await f.storage.readMemory(id(1));changed.body='Updated with preserved conditions';
  await commit(f,changed,1);
  assert.deepEqual(plain(await f.storage.readMemory(id(2))),plain(original));
  assert.equal((await f.retrieval.searchMemories(project,'Updated'))[0].summary.revision,2);
  assert.equal((await f.retrieval.searchMemories(project,'retired')).length,0);
  f.apply((await f.mutations.removeOperations(id(1),2)).operations);
  assert.deepEqual(plain(await f.storage.readMemory(id(2))),plain(original));
  assert.equal(f.get('memory/catalog').value.pages['0'],1);
  f.apply((await f.mutations.removeOperations(id(2),1)).operations);
  assert.equal(f.get('memory/body/0').present,false);
  assert.deepEqual(f.get('memory/catalog').value.pages,{});
  assert.equal(f.get('memory/catalog').value.active,0);
});

test('prevents concurrent equivalent creates through head CAS without partial pages or indexes',async()=>{
  const f=await fixture();
  const [first,second]=await Promise.all([f.mutations.putOperations(entry(1,'Use  concise\nreplies'),0),
    f.mutations.putOperations(entry(2,'use concise replies'),0)]);
  f.apply(first.operations);
  assert.throws(()=>f.apply(second.operations),error=>error.code==='revision_conflict');
  assert.equal(f.get('memory/entry/'+id(2)).present,false);
  assert.equal(f.indexes.size,1);
  assert.equal(f.get('memory/catalog').value.active,1);
  await assert.rejects(f.mutations.putOperations(entry(3,'USE concise replies'),0),error=>error.code==='conflict');
  await commit(f,entry(4,'Use C++'));
  await commit(f,entry(5,'Use C#'));
  assert.equal((await f.retrieval.searchMemories(project,'C++'))[0].summary.id,id(4));
});

test('recaptures a changed body without pairing old metadata',async()=>{
  const f=await fixture();
  f.seed([entry(1,'Original verified conditions')]);
  const captured=f.get('memory/entry/'+id(1)), changed=await f.storage.readMemory(id(1));
  changed.body='New verified conditions';
  const prepared=await f.mutations.putOperations(changed,1);
  f.onRead=key=>{if(key.startsWith('memory/body/')) {f.onRead=null;f.apply(prepared.operations);}};
  const recaptured=await f.storage.readMemory(id(1));
  assert.equal(recaptured.summary.revision,2);
  assert.equal(recaptured.body,changed.body);
  await assert.rejects(f.storage.body(f.storage.metadata(captured)),error=>error.code==='revision_conflict');
  const current=await f.storage.readMemory(id(1));
  assert.equal(current.summary.revision,2);
  assert.equal(current.body,changed.body);
  current.summary.archived=true;
  await commit(f,current,2);
  assert.equal((await f.storage.readMemory(id(1))).body,changed.body);
  assert.equal(f.get('memory/entry/'+id(1)).value.body_revision,2);
});

test('scope, archive, exact IDs and literal search stay in the package',async()=>{
  const f=await fixture();
  f.seed([entry(1,'Current keyword'),entry(2,'Global keyword',{project:null,kind:'user'}),
    entry(3,'Foreign keyword',{project:id(9000)}),entry(4,'Archived keyword',{archived:true})]);
  assert.deepEqual(plain((await f.retrieval.searchMemories(project,'keyword')).map(entry=>entry.summary.id)),[id(1),id(2)]);
  assert.deepEqual(plain((await f.retrieval.browseMemories({project,all_projects:false,archived:false,query:''})).map(entry=>entry.id)),[id(1)]);
  assert.deepEqual(plain((await f.retrieval.browseMemories({project,all_projects:false,archived:true,query:id(4)})).map(entry=>entry.id)),[id(4)]);
  assert.equal((await f.retrieval.searchMemories(project,id(3))).length,0);
  assert.equal((await f.retrieval.searchMemories(project,id(4))).length,0);
  assert.equal((await f.retrieval.searchMemories(project,'how is the weather')).length,0);
  assert.equal((await f.retrieval.searchMemories(project,'"keyword" OR (')).length,2);
  const before=f.reads.length;
  await assert.rejects(f.storage.readMemory(id(3),project),error=>error.code==='permission_denied');
  assert.deepEqual(f.reads.slice(before),['memory/catalog','memory/entry/'+id(3)]);
  await assert.rejects(f.mutations.removeOperations(id(3),1,project),error=>error.code==='permission_denied');
});

const tag='Relevant memory: ';
const relevant=parts=>Array.from(parts).filter(part=>part.startsWith(tag)).map(part=>JSON.parse(part.slice(tag.length)));

test('new turns receive only relevant scoped bodies as parts',async()=>{
  const f=await fixture(),preference='Use unittest and four spaces for Python tests',boundary='中文'.repeat(50);
  f.configure({enabled:true,auto_write:true,context_bytes:8192,review_after_days:90});
  f.seed([entry(1,preference,{kind:'user',project:null,title:'Python tests'}),
    entry(2,boundary,{kind:'feedback'}),entry(3,boundary+'!',{kind:'feedback'}),
    entry(4,'Project body stays out of the index'),entry(5,'Foreign preference',{project:id(9000),kind:'user'}),
    entry(6,'Archived preference',{archived:true,kind:'feedback'})]);
  const initialized=await f.main.initialize({tools:['search_memory','save_memory'],project,resources:true,message:'How should Python tests look?',mode:'code'});
  assert.equal(initialized.instruction,'');
  assert.deepEqual(relevant(initialized.parts).map(value=>[value.id,value.body,value.global]),[[id(1),preference,true]]);
  assert.equal(initialized.parts.length,1);
  const text=JSON.stringify(initialized);
  for(const number of [2,3,4,5,6])assert.equal(text.includes(id(number)),false,'only matched entries appear; titles are never indexed');
  assert.equal(f.state.remaining,8192-Buffer.byteLength(initialized.parts[0]));
  assert.equal(f.prepared.length,0,'recall writes no statistics');
  assert.equal([...f.records.keys()].some(key=>key.startsWith('memory/recall/')),false);
  assert.equal(f.get('memory/entry/'+id(1)).revision,'1');
  const denied=await f.main.save({arguments:{id:id(2),expected_revision:1,kind:'feedback',title:'Replacement',body:'Changed'},step:0});
  assert.equal(denied.result.error.code,'invalid_request');
  assert.match(denied.result.error.message,/call search_memory with this ID/);
});

test('relevant recall reads the latest consistent snapshot',async()=>{
  const f=await fixture();
  f.seed([entry(1,'Use unittest',{kind:'user'})]);
  const changed=await f.storage.readMemory(id(1));changed.summary.title='Updated preference';
  const prepared=await f.mutations.putOperations(changed,1);
  f.onRead=key=>{if(key.startsWith('memory/body/')){f.onRead=null;f.apply(prepared.operations);}};
  const initialized=await f.main.initialize({tools:['search_memory'],project,resources:true,message:'unittest',mode:'plan'});
  const [value]=relevant(initialized.parts);
  assert.equal(value.title,'Updated preference');
  assert.equal(value.body,'Use unittest');
  assert.equal(f.prepared.length,0);
  assert.equal(f.completed.length,0);
});

test('scoped writes reuse the captured project without reading a management catalog',async()=>{
  const f=await fixture();
  f.apply((await f.mutations.putOperations(entry(1,'Captured project constraint'),0,project)).operations);
  assert.equal(f.catalogReads.length,0);
  await assert.rejects(f.mutations.putOperations(entry(2,'Foreign project constraint',{project:id(9000)}),0,project),error=>error.code==='permission_denied');
  assert.equal(f.catalogReads.length,0);
  await commit(f,entry(3,'Explicit management constraint'));
  assert.equal(f.catalogReads.length,1);
  await assert.rejects(f.mutations.putOperations(entry(4,'Unavailable project constraint',{project:id(9001)}),0),error=>error.code==='not_found');
});

test('retains archive bodies and checks every source revision in one atomic consolidation',async()=>{
  const f=await fixture();
  f.seed([entry(1,'Original complete conditions'),entry(2,'Additional verified exception'),entry(3,'Other fact')]);
  const target=await f.storage.readMemory(id(1));target.body='Merged complete conditions and exception';
  const original=await f.storage.readMemory(id(2));
  const prepared=await f.mutations.mergeOperations(target,[{id:id(2),revision:1}]);
  const output=f.apply(prepared.operations), outcome=f.mutations.memoryOutput(output);
  assert.equal(outcome.data.length,2);
  const archived=await f.storage.readMemory(id(2));
  assert.equal(archived.body,original.body);
  assert.equal(archived.summary.archived,true);
  assert.equal(archived.summary.revision,2);
  assert.equal((await f.retrieval.searchMemories(project,id(2))).length,0);
  assert.equal((await f.retrieval.browseMemories({project,all_projects:false,archived:true,query:'exception'}))[0].id,id(2));
  assert.equal(f.get('memory/catalog').value.active,2);
  assert.equal(f.get('memory/catalog').value.archived,1);
  await assert.rejects(f.mutations.mergeOperations(await f.storage.readMemory(id(1)),[{id:id(3),revision:9}]),error=>error.code==='revision_conflict');
});

test('uses independent configuration revisions and revokes prepared writes through the shared head',async()=>{
  const f=await fixture();
  const settings=await f.settings.readMemorySettings();
  const prepared=await f.mutations.putOperations(entry(1,'Waiting on approval'),0,project);
  const request=await f.settings.prepareMemorySettings({...settings,auto_write:false});
  const outcome=await f.sdk.completeRequest(request);
  assert.equal(f.settings.outcome(outcome.Ok).revision,1);
  assert.throws(()=>f.apply(prepared.operations),error=>error.code==='revision_conflict');
  assert.equal(f.get('memory/entry/'+id(1)).present,false);
  await assert.rejects(f.mutations.putOperations(entry(1,'Denied live write'),0,project),error=>error.code==='permission_denied');
  await commit(f,entry(2,'Explicit management remains available'));
  assert.equal((await f.settings.readMemorySettings()).revision,1);
  await assert.rejects(f.sdk.completeRequest('missing'),()=>true).catch(()=>{});
});

test('retains growing catalogs and preserves incompatible private data',async()=>{
  const f=await fixture();
  f.seed(Array.from({length:512},(_,index)=>entry(index,`Active constraint ${index}`)));
  await commit(f,entry(8000,'Additional active fact'));
  assert.equal(f.get('memory/catalog').value.active,513);
  const archived=await f.storage.readMemory(id(1));archived.summary.archived=true;
  await commit(f,archived,1);
  await commit(f,entry(8001,'Available active slot'));
  assert.equal(f.get('memory/catalog').value.active,513);
  assert.equal(f.get('memory/catalog').value.archived,1);
  f.store('memory/catalog',{v:2,pages:{},active:0,archived:0,next:0});
  const original=f.get('memory/catalog');
  await assert.rejects(f.mutations.putOperations(entry(8001,'No automatic conversion'),0),/incompatible/);
  assert.deepEqual(f.get('memory/catalog'),original);
});

test('reviews a populated catalog within host SDK call capacity',async()=>{
  const f=await fixture();
  f.seed(Array.from({length:512},(_,index)=>entry(index,'Shared verified constraints')));
  const hints=await f.retrieval.reviewMemories({review_after_days:90},{project,all_projects:false});
  assert.equal(hints.length,512);
  const calls=f.reads.length+f.searches.length;
  assert.ok(calls > 256);
  assert.ok(calls < 1024,`${calls} SDK calls`);
  assert.equal(f.reads.filter(key=>key.startsWith('memory/body/')).length,256);
});

test('review recaptures metadata changed during body reads',async()=>{
  const f=await fixture();
  f.seed([entry(1,'Verified conditions remain unchanged')]);
  const changed=await f.storage.readMemory(id(1));changed.summary.title='Updated title';
  const prepared=await f.mutations.putOperations(changed,1);
  f.now=200*86_400_000;
  f.onRead=key=>{if(key.startsWith('memory/body/')) {f.onRead=null;f.apply(prepared.operations);}};
  const hints=await f.retrieval.reviewMemories({review_after_days:90},{project,all_projects:false});
  assert.equal(hints[0].summary.title,'Updated title');
  assert.equal(hints[0].summary.revision,2);
});

test('maximal consolidation and full catalogs respect operation, value, key and byte bounds',async()=>{
  const f=await fixture(), han=Array.from({length:2730},(_,index)=>String.fromCodePoint(0x4e00+index)).join('');
  const escaped='\u0001'.repeat(8191)+'a', title='"'.repeat(160);
  const entries=Array.from({length:512+2048-16},(_,index)=>entry(index,index < 34 ? (index%2 ? '\u0001'.repeat(8190)+String(index).padStart(2,'0') : han+String(index).padStart(2,'0')) : 'Retained fact '+index,
    {title,archived:index >= 512}));
  f.seed(entries);
  const target=await f.storage.readMemory(id(0));target.body=escaped;
  const sources=Array.from({length:16},(_,index)=>({id:id((index+1)*2),revision:1}));
  const prepared=await f.mutations.mergeOperations(target,sources), encoded=Buffer.byteLength(JSON.stringify(prepared));
  assert.equal(prepared.operations.length,19);
  assert.ok(encoded < 1024*1024,`${encoded} bytes`);
  assert.ok(prepared.operations.every(operation=>Buffer.byteLength(JSON.stringify(operation.data.value)) <= 256*1024));
  assert.equal(f.mutations.memoryOutput(f.apply(prepared.operations)).data.length,17);
  assert.equal(f.get('memory/catalog').value.archived,2048);
  f.seed(Array.from({length:512+2048},(_,index)=>entry(index,'\u0001'.repeat(8188)+String(index).padStart(4,'0'),
    {title,archived:index >= 512})));
  assert.equal([...f.records.values()].filter(value=>value.present).length,3841);
  assert.ok([...f.records.values()].filter(value=>value.present).reduce((sum,value)=>sum+Buffer.byteLength(JSON.stringify(value.value))
    +(f.indexes.has(value.key) ? Buffer.byteLength(JSON.stringify(f.indexes.get(value.key))) : 0),0) < 128*1024*1024);
});

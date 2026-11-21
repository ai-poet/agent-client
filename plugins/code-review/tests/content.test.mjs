import assert from 'node:assert/strict';
import test from 'node:test';
import {file,fixture,finding,receipt,uuid} from './support.mjs';

const live=current => [...current.values.values()].filter(row => row.present);
const capture=(current,id) => current.values.get(`review/${id}/capture`);

test('stored evidence survives edits and reopening',async()=>{
  const original=file('src/main.js',{old_text:'original\r\nsecond\r\nend',new_text:'changed\r\nsecond\r\nend'});
  const current=await fixture([original]);
  const scope=await current.capture();
  const read={review_id:scope.review_id,path:original.path};
  const diff=await current.read(read);
  const first=await current.read({...read,part:'new',limit:1});
  assert.equal(first.text,'changed\r\n'); assert.equal(first.next,1);
  const tail=await current.read({...read,part:'new',offset:first.next});
  assert.equal(first.text+tail.text,original.new_text); assert.equal(tail.next,null);
  assert.equal((await current.read({...read,part:'old'})).text,original.old_text);
  current.change(original.path,{revision:'changed',new_text:'unrelated replacement'});
  current.add(file('unrelated.rs'));
  current.metadata('new inventory'); current.head('b'.repeat(40));
  const before=current.calls.length;
  assert.deepEqual(await current.read(read),diff);
  assert.equal((await current.invoke('finding',finding(scope))).finding.revision,original.revision);
  assert.equal((await current.invoke('complete',receipt(scope))).status,'complete');
  assert.ok(current.calls.slice(before).every(call => call.kind === 'plugin_transaction'));
  const reopened=await fixture([original],{values:current.values});
  assert.deepEqual(await reopened.read(read),diff);
  const fresh=await current.invoke('scope',{});
  assert.notEqual(fresh.review_id,scope.review_id);
  assert.equal((await current.read({review_id:fresh.review_id,path:original.path,part:'new'})).text,'unrelated replacement');
  assert.equal(fresh.counts.files,2);
});

test('missing content never falls back to live source',async()=>{
  const current=await fixture(),scope=await current.invoke('scope',{});
  const value=current.values.get(`review/${scope.review_id}/head`).value;
  const scopeRow=current.values.get(`review/${scope.review_id}/scope/${value.pages[0]}`);
  delete scopeRow.value.rows.find(row => row.kind === 'file').value.content;
  const before=current.calls.length;
  assert.equal((await current.invoke('scope',{review_id:scope.review_id})).review_id,scope.review_id);
  assert.equal((await current.read({review_id:scope.review_id,path:scope.files[0].path})).error.code,'not_found');
  assert.equal((await current.invoke('finding',finding(scope))).error.code,'not_found');
  assert.equal((await current.invoke('complete',receipt(scope))).error.code,'not_found');
  assert.equal(current.calls.length,before);
  assert.equal(current.values.get(`review/${scope.review_id}/head`).value.status,'open');
});

test('altered captured chunks reject evidence and completion',async()=>{
  const current=await fixture(),scope=await current.invoke('scope',{});
  current.values.get(`review/${scope.review_id}/data/0`).revision='2';
  assert.equal((await current.read({review_id:scope.review_id,path:scope.files[0].path})).error.code,'internal');
  assert.equal((await current.invoke('finding',finding(scope))).error.code,'internal');
  assert.equal((await current.invoke('complete',receipt(scope))).error.code,'internal');
});

test('cleanup fences unpublished final publication',async()=>{
  const current=await fixture(),snapshots=await current.module('capture');
  const id=uuid(100),actor=current.captured;
  const owner=await snapshots.begin(id,actor);
  await snapshots.append(owner,'data',{v:1,kind:'content',path:'src/main.js',part:'diff',
    revision:'frozen',offset:0,next:null,lines:[]});
  await snapshots.flush(owner);
  await snapshots.discard(id,actor);
  await assert.rejects(snapshots.finish(owner,[{kind:'write',data:{key:`review/${id}/head`,
    value:{v:1,kind:'head'},expected_revision:'0'}}]),{code:'revision_conflict'});
  assert.equal(live(current).length,0);
  assert.ok(current.values.size>0,'Existing durable tombstones remain');
  const next=await current.invoke('scope',{});
  const before=JSON.stringify([...current.values]);
  await snapshots.discard(next.review_id,actor);
  assert.equal(JSON.stringify([...current.values]),before,'Published resources cannot be reclaimed');
});

test('uncertain publication preserves the committed snapshot',async()=>{
  const current=await fixture(undefined,{command(command,{commit}){
    if(command.kind==='plugin_transaction' && command.data.operations.some(op=>op.data.value?.kind==='head')) {
      commit(command.data.operations);
      return {Err:{code:'outcome_unknown',message:'Completion was lost'}};
    }
  }});
  assert.equal((await current.invoke('scope',{})).error.code,'outcome_unknown');
  const published=live(current).find(row=>row.value?.kind==='capture');
  assert.equal(published.value.status,'published');
  const scope=await current.invoke('scope',{review_id:published.value.id});
  assert.deepEqual((await current.read({review_id:scope.review_id,path:scope.files[0].path})).lines,
    [{kind:'added',old_line:null,new_line:7,text:'return changed();'}]);
});

test('terminal staging recovery preserves active and published captures',async()=>{
  const current=await fixture(),snapshots=await current.module('capture');
  const published=await current.invoke('scope',{});
  const abandoned=uuid(101),active=uuid(102);
  for(const id of [abandoned,active]) {
    const owner=await snapshots.begin(id,current.captured);
    await snapshots.append(owner,'data',{v:1,kind:'content',path:'src/main.js',part:'diff',
      revision:'frozen',offset:0,next:null,lines:[]});
    await snapshots.flush(owner);
  }
  await current.invoke('scope',{});
  assert.equal(capture(current,abandoned).value.status,'capturing');
  assert.equal(capture(current,active).value.status,'capturing');
  current.terminal(current.captured.turn);
  current.captured.turn=uuid(900);
  const next=await current.invoke('scope',{});
  assert.equal(capture(current,abandoned).present,false);
  assert.equal(capture(current,active).present,false);
  assert.equal(capture(current,published.review_id).value.status,'published');
  assert.equal(capture(current,next.review_id).value.status,'published');
});

test('large captures use bounded batches without a review transaction cap',async()=>{
  const lines=Array.from({length:18000},(_,index)=>({kind:'added',old_line:null,new_line:index+1,
    text:`${index}:${'x'.repeat(90)}`}));
  const current=await fixture([file('src/large.js',{lines})]);
  const scope=await current.capture();
  assert.equal(scope.counts.files,1);
  const transactions=current.calls.filter(call=>call.kind==='plugin_transaction');
  assert.ok(transactions.length>2);
  for(const transaction of transactions) {
    assert.ok(transaction.data.operations.length<=64);
    assert.ok(Buffer.byteLength(JSON.stringify(transaction.data.operations))<=2*1024*1024);
  }
  const last=await current.read({review_id:scope.review_id,path:'src/large.js',offset:17999});
  assert.deepEqual(last.lines,[lines.at(-1)]); assert.equal(last.next,null);
});

test('capture resumes stored offsets after reopening',async()=>{
  const lines=Array.from({length:12000},(_,index)=>({kind:'added',old_line:null,new_line:index+1,text:`line ${index}`}));
  const source=file('src/large.js',{lines});
  let current=await fixture([source]), before=0;
  let scope=await current.invoke('scope',{});
  assert.equal(scope.status,'capturing');
  assert.equal(current.values.has(`review/${scope.review_id}/head`),false);
  const first=current.calls.filter(call=>call.kind.startsWith('read_git_') || call.kind === 'list_git_changes');
  assert.ok(first.length<=16);
  const stored=current.values.get(`review/${scope.review_id}/capture`).value;
  assert.ok(stored.progress.offset>0);
  assert.equal(stored.progress.part,'diff');
  current=await fixture([source],{values:current.values});
  const calls=[...first];
  while (scope.status === 'capturing') {
    before=current.calls.length;
    scope=await current.invoke('scope',{review_id:scope.review_id});
    const batch=current.calls.slice(before).filter(call=>call.kind.startsWith('read_git_') || call.kind === 'list_git_changes');
    assert.ok(batch.length<=16);
    calls.push(...batch);
  }
  assert.equal(scope.status,'open');
  assert.equal(scope.counts.files,1);
  const diffs=calls.filter(call=>call.kind === 'read_git_change');
  assert.deepEqual(diffs.map(call=>call.data.offset),Array.from({length:24},(_,index)=>index*500));
  assert.equal(calls.filter(call=>call.kind === 'list_git_changes').length,2);
  const last=await current.read({review_id:scope.review_id,path:source.path,offset:11999});
  assert.deepEqual(last.lines,[lines.at(-1)]);
});

test('committed batches are reclaimed after later capture conflicts',async()=>{
  let committed=false,interrupted=false;
  const lines=Array.from({length:18000},(_,index)=>({kind:'added',old_line:null,new_line:index+1,text:'x'.repeat(90)}));
  const current=await fixture([file('src/large.js',{lines})],{command(command){
    if(command.kind==='plugin_transaction' && command.data.operations.some(op=>op.data.value?.kind==='content')) committed=true;
    if(committed && command.kind==='read_git_context' && !interrupted) {
      interrupted=true;
      return {Err:{code:'revision_conflict',message:'Concurrent source edit'}};
    }
  }});
  const result=await current.capture();
  assert.equal(result.error.code,'revision_conflict'); assert.equal(committed,true);
  assert.equal(live(current).length,0);
  assert.ok([...current.values.values()].some(row=>row.key.includes('/data/') && !row.present));
  assert.equal((await current.capture()).counts.files,1);
});

test('uncertain committed batches are fenced without replay',async()=>{
  let uncertain=0;
  const current=await fixture(undefined,{command(command,{commit}){
    if(command.kind==='plugin_transaction' && command.data.operations.some(op=>op.data.value?.kind==='content') && !uncertain++) {
      commit(command.data.operations);
      return {Err:{code:'outcome_unknown',message:'Batch completion was lost'}};
    }
  }});
  assert.equal((await current.invoke('scope',{})).error.code,'outcome_unknown');
  assert.equal(uncertain,1); assert.equal(live(current).length,0);
});

test('storage exhaustion removes only unpublished capture resources',async()=>{
  let full=false;
  const current=await fixture(undefined,{command(command){
    if(full && command.kind==='plugin_transaction'
        && command.data.operations.some(op=>op.data.value?.kind==='content')) {
      return {Err:{code:'invalid_request',message:'Plugin storage namespace budget exceeded'}};
    }
  }});
  const original=await current.invoke('scope',{}),before=JSON.stringify(live(current));
  full=true;
  const failed=await current.invoke('scope',{});
  assert.equal(failed.isError,true);
  assert.equal(JSON.stringify(live(current)),before);
  assert.equal((await current.read({review_id:original.review_id,path:original.files[0].path})).revision,original.files[0].revision);
});

test('incomplete source context cannot publish a scope',async()=>{
  const current=await fixture(undefined,{command(command){
    if(command.kind==='read_git_context') return {Ok:{kind:'git_context',data:{
      ...command.data,old_path:null,metadata_revision:command.data.expected_metadata_revision,
      revision:command.data.expected_revision,text:'',next:null,binary:false,skip_reason:'oversized_line',
    }}};
  }});
  const failed=await current.invoke('scope',{});
  assert.equal(failed.isError,true); assert.match(failed.error.message,/Complete review context is unavailable/);
  assert.equal(live(current).length,0);
});

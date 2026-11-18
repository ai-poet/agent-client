import assert from 'node:assert/strict';
import test from 'node:test';
import {file,fixture,receipt,finding,uuid} from './support.mjs';

function partial(scope, group) {
  return receipt(scope,{reviewed_groups:[group.id],reviewed_files:scope.files.filter(file => group.files.includes(file.path))
    .map(({path,revision}) => ({path,revision})),finalize:false});
}

test('zero findings still require every file and group', async () => {
  const fixtureData = await fixture(Array.from({length:15},(_,index) => file(`src/${index}.rs`)));
  const scope = await fixtureData.capture();
  const missing = await fixtureData.invoke('complete',receipt(scope,{reviewed_files:[],reviewed_groups:[]}));
  assert.equal(missing.isError,true);
  assert.equal(missing.remaining.counts.files,15);
  assert.deepEqual(missing.remaining.groups,scope.groups.map(group => group.id));
  const result = await fixtureData.invoke('complete',receipt(scope));
  assert.equal(result.status,'complete');
  assert.equal(result.reviewed_files,15);
  assert.equal(result.findings,0);
  assert.equal(result.remaining.counts.files,0);
  assert.deepEqual(result.sailry_result,{version:1,diagnostics:[],status:{label:'15 files',locales:{'zh-CN':'15 个文件'}}});
});

test('summarizes files without changing findings',async()=>{
  const f=await fixture(),scope=await f.invoke('scope',{});
  const reported=await f.invoke('finding',finding(scope));
  const before=structuredClone([...f.values].filter(([key])=>key.includes('/finding/')));
  const completed=await f.invoke('complete',receipt(scope));
  assert.equal(completed.findings,1);
  assert.deepEqual(completed.sailry_result.status,{label:'1 file',locales:{'zh-CN':'1 个文件'}});
  assert.ok(reported.finding.id);
  assert.deepEqual([...f.values].filter(([key])=>key.includes('/finding/')),before);
});

test('invalid duplicate and partial receipts are rejected', async () => {
  const fixtureData = await fixture([file('src/a.rs'),file('src/b.rs')]);
  const scope = await fixtureData.invoke('scope',{});
  const complete = receipt(scope);
  for (const invalid of [{reviewed_files:complete.reviewed_files.slice(0,1)},
    {reviewed_files:[...complete.reviewed_files,complete.reviewed_files[0]]},
    {reviewed_groups:[...complete.reviewed_groups,complete.reviewed_groups[0]]},
    {reviewed_groups:['group-unknown']},{reviewed_files:[{path:'outside.rs',revision:'anything'}]},
    {reviewed_files:complete.reviewed_files.map(file => ({...file,revision:'stale'}))}]) {
    const rejected=await fixtureData.invoke('complete',{...complete,...invalid});
    assert.equal(rejected.isError,true);
    assert.equal(rejected.sailry_result,undefined);
  }
  const head = fixtureData.values.get(`review/${scope.review_id}/head`).value;
  assert.equal(head.status,'open');
  assert.deepEqual(head.reviewed_groups,[]);
});

test('captured content survives changes despite stable metadata', async () => {
  const fixtureData = await fixture();
  const scope = await fixtureData.invoke('scope',{});
  fixtureData.change(scope.files[0].path,{revision:'external edit with restored metadata'});
  const result = await fixtureData.invoke('complete',receipt(scope));
  assert.equal(result.status,'complete');
  assert.equal(fixtureData.values.get(`review/${scope.review_id}/head`).value.status,'complete');
});

test('published inventory survives unrelated working paths', async () => {
  const fixtureData = await fixture();
  const scope = await fixtureData.invoke('scope',{});
  fixtureData.metadata('inventory:changed');
  const result = await fixtureData.invoke('complete',receipt(scope));
  assert.equal(result.status,'complete');
  assert.equal(fixtureData.values.get(`review/${scope.review_id}/head`).value.status,'complete');
});

test('commit finalization never resolves moving refs again', async () => {
  const fixtureData = await fixture();
  const scope = await fixtureData.invoke('scope',{target:{kind:'commit',data:{revision:'HEAD'}}});
  fixtureData.head('b'.repeat(40));
  fixtureData.change(scope.files[0].path,{revision:'working copy changed'});
  fixtureData.metadata('inventory:changed');
  const result = await fixtureData.invoke('complete',receipt(scope));
  assert.equal(result.status,'complete');
  const pages = fixtureData.calls.filter(call => call.kind === 'list_git_changes');
  assert.equal(pages[1].data.target,null);
  assert.deepEqual(pages[1].data.cursor,{snapshot:scope.snapshot,after_path:null});
});

test('children cover groups and creator finalizes the union', async () => {
  const fixtureData = await fixture([file('first/main.rs'),file('second/main.rs')]);
  const creator = fixtureData.captured.session;
  const scope = await fixtureData.invoke('scope',{});
  fixtureData.actor(uuid(20),creator);
  const first = await fixtureData.invoke('complete',partial(scope,scope.groups[0]));
  assert.equal(first.status,'open');
  assert.equal(first.reviewed_files,1);
  assert.equal(first.remaining.counts.files,1);
  assert.equal((await fixtureData.invoke('complete',receipt(scope,{reviewed_files:[],reviewed_groups:[],finalize:true}))).error.code,'permission_denied');
  fixtureData.actor(uuid(21),creator);
  assert.equal((await fixtureData.invoke('complete',partial(scope,scope.groups[1]))).status,'open');
  fixtureData.actor(creator);
  const final = await fixtureData.invoke('complete',receipt(scope,{reviewed_files:[],reviewed_groups:[]}));
  assert.equal(final.status,'complete');
  assert.equal(final.reviewed_files,2);
  assert.equal(fixtureData.values.get(`review/${scope.review_id}/head`).value.receipts.length,2);
  assert.equal((await fixtureData.invoke('finding',finding(scope))).error.code,'conflict');
});

test('unrelated sessions grandchildren and package identities cannot write', async () => {
  const fixtureData = await fixture();
  const creator = fixtureData.captured.session;
  const scope = await fixtureData.invoke('scope',{});
  for (const [id,parent,worktree] of [[uuid(20),null,fixtureData.captured.worktree],
    [uuid(21),uuid(20),null],[uuid(22),creator,uuid(99)]]) {
    fixtureData.actor(id,parent,worktree ?? fixtureData.captured.worktree);
    const result = await fixtureData.invoke('finding',finding(scope));
    assert.equal(result.error.code,'permission_denied');
  }
  fixtureData.actor(creator);
  fixtureData.values.get(`review/${scope.review_id}/head`).value.package.name = 'another-package';
  assert.equal((await fixtureData.invoke('complete',receipt(scope))).error.code,'permission_denied');
});

test('merges parallel findings after a storage conflict', async () => {
  const fixtureData = await fixture();
  const creator = fixtureData.captured.session;
  const scope = await fixtureData.invoke('scope',{});
  fixtureData.actor(uuid(20),creator);
  const first = await fixtureData.start('finding',finding(scope));
  fixtureData.actor(uuid(21),creator);
  const second = await fixtureData.start('finding',finding(scope,{title:'Second evidenced finding'}));
  assert.equal(first.response.call.arguments.operations[0].data.expected_revision,'1');
  assert.equal(second.response.call.arguments.operations[0].data.expected_revision,'1');
  assert.ok((await fixtureData.finish(first)).finding);
  const merged = await fixtureData.finish(second);
  assert.equal(merged.finding.id,second.response.state.intent);
  assert.equal(merged.finding.reporter,uuid(21));
  assert.equal(second.attempts.length,2);
  assert.equal(second.attempts[0][0].data.expected_revision,'1');
  assert.equal(second.attempts[1][0].data.expected_revision,'2');
  assert.equal(second.attempts[0][1].data.key,second.attempts[1][1].data.key);
  const head = fixtureData.values.get(`review/${scope.review_id}/head`).value;
  assert.deepEqual(head.findings,[first.response.state.intent,second.response.state.intent]);
  assert.equal(fixtureData.values.get(`review/${scope.review_id}/finding/${first.response.state.intent}`).value.finding.reporter,uuid(20));
});

test('merges independent child receipts without lost coverage', async () => {
  const fixtureData = await fixture([file('first/main.rs'),file('second/main.rs')]);
  const creator = fixtureData.captured.session;
  const scope = await fixtureData.invoke('scope',{});
  fixtureData.actor(uuid(20),creator);
  const first = await fixtureData.start('complete',partial(scope,scope.groups[0]));
  fixtureData.actor(uuid(21),creator);
  const second = await fixtureData.start('complete',partial(scope,scope.groups[1]));
  assert.equal((await fixtureData.finish(first)).status,'open');
  assert.equal((await fixtureData.finish(second)).status,'open');
  assert.equal(second.attempts.length,2);
  assert.equal(second.attempts[0][1].data.key,second.attempts[1][1].data.key);
  assert.deepEqual(fixtureData.values.get(`review/${scope.review_id}/head`).value.receipts,
    [first.response.state.intent,second.response.state.intent]);
  fixtureData.actor(creator);
  const result = await fixtureData.invoke('complete',receipt(scope,{reviewed_files:[],reviewed_groups:[]}));
  assert.equal(result.status,'complete');
  assert.equal(result.reviewed_files,2);
});

test('finalization races reject stale reports', async () => {
  const fixtureData = await fixture();
  const scope = await fixtureData.invoke('scope',{});
  const report = await fixtureData.start('finding',finding(scope));
  assert.equal((await fixtureData.invoke('complete',receipt(scope))).status,'complete');
  assert.equal((await fixtureData.finish(report)).error.code,'conflict');
  assert.equal(report.attempts.length,1);
  const head = fixtureData.values.get(`review/${scope.review_id}/head`).value;
  assert.deepEqual(head.findings,[]);
});


test('retries only confirmed key conflicts within the budget',async()=>{
  const f=await fixture(), scope=await f.invoke('scope',{});
  const pending=await f.start('finding',finding(scope));
  const conflict={isError:true,error:{code:'revision_conflict',message:'Value changed',
    details:{kind:'plugin_value_conflict'}}};
  const result=await f.finish(pending,()=>conflict);
  assert.deepEqual(result,conflict);
  assert.equal(pending.attempts.length,8);
  assert.ok(pending.attempts.every(operations => operations[1].data.key.endsWith(pending.response.state.intent)));
  assert.deepEqual(f.values.get(`review/${scope.review_id}/head`).value.findings,[]);
  assert.equal([...f.values.keys()].filter(key => key.includes('/finding/')).length,0);
});

test('preserves unknown cancellation and configuration fences',async()=>{
  for (const error of [
    {code:'outcome_unknown',message:'Completion unavailable'},
    {code:'cancelled',message:'Turn cancelled'},
    {code:'permission_denied',message:'Package revoked'},
    {code:'revision_conflict',message:'Configuration changed'},
    {code:'revision_conflict',message:'File changed',details:{kind:'another_conflict'}},
  ]) {
    const f=await fixture(),scope=await f.invoke('scope',{});
    const pending=await f.start('finding',finding(scope));
    const outcome={isError:true,error};
    const before=structuredClone([...f.values]);
    assert.deepEqual(await f.finish(pending,outcome),outcome);
    assert.equal(pending.attempts.length,1);
    assert.deepEqual([...f.values],before);
  }
});

test('rechecks review ownership after a retryable conflict',async()=>{
  const f=await fixture(),scope=await f.invoke('scope',{});
  const pending=await f.start('finding',finding(scope));
  const before=structuredClone([...f.values]);
  const result=await f.finish(pending,()=>{
    f.values.get(`review/${scope.review_id}/head`).value.package.name='other-package';
    return {isError:true,error:{code:'revision_conflict',message:'Value changed',
      details:{kind:'plugin_value_conflict'}}};
  });
  assert.equal(result.error.code,'permission_denied');
  assert.equal(pending.attempts.length,1);
  assert.deepEqual(f.values.get(`review/${scope.review_id}/head`).value.findings,[]);
  assert.equal([...f.values.keys()].filter(key => key.includes('/finding/')).length,0);
  assert.equal(f.values.size,before.length);
});

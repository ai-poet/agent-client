import assert from 'node:assert/strict';
import test from 'node:test';
import {file,fixture,plain,receipt,uuid} from './support.mjs';

const tools = ['review_scope','report_finding','review_complete','delegate_review'];
const roles = [{key:'reviewer',name:'Reviewer',read_only:false,tools:true},
  {key:'evidence',name:'Evidence',read_only:true,tools:true},
  {key:'no-tools',name:'No tools',read_only:false,tools:false}];

function initialize(current, overrides = {}) {
  return current.initialize({tools,roles,mode:'code',...overrides});
}
function captured(task) {
  return JSON.parse(task.split('\nFrozen review capture:\n')[1]);
}

test('handoff contains complete authoritative frozen groups', async () => {
  const current = await fixture([file('src/math.c'),file('include/math.h'),file('tests/math_test.c'),
    file('other/unrelated.rs')]);
  initialize(current);
  const scope = await current.invoke('scope',{});
  const selected = scope.files.find(file => file.path === 'src/math.c').group;
  const before = plain([...current.values]);
  const output = await current.delegate({review_id:scope.review_id,groups:[selected],
    record_receipt:true,role:'reviewer',background:true});
  assert.equal(output.role,'reviewer');
  assert.equal(output.background,true);
  assert.equal(Object.hasOwn(output,'session'),false);
  assert.equal(Object.hasOwn(output,'worktree'),false);
  const capture = captured(output.task);
  assert.equal(capture.review_id,scope.review_id);
  assert.equal(capture.record_receipt,true);
  assert.deepEqual(capture.snapshot,scope.snapshot);
  assert.deepEqual(capture.files,scope.files.filter(file => file.group === selected));
  assert.deepEqual(capture.groups,scope.groups.filter(group => group.id === selected));
  assert.ok(capture.files.every(file => file.metadata_revision && file.revision && file.rule.sha256));
  assert.match(output.task,/finalize:false/);
  assert.deepEqual(plain([...current.values]),before);
});


test('writable turns preserve receipt child selection', async () => {
  for (const mode of ['code','flagship']) {
    for (const role of [undefined,'reviewer']) {
      const current = await fixture();
      initialize(current,{mode});
      const scope = await current.invoke('scope',{});
      const result = await current.delegate({review_id:scope.review_id,groups:[scope.groups[0].id],record_receipt:true,role});
      assert.equal(result.isError,undefined);
      assert.equal(result.role,role ?? null);
      assert.equal(captured(result.task).record_receipt,true);
      assert.equal(Object.hasOwn(result,'model'),false);
      assert.match(result.task,/review_complete/);
    }
  }
});

test('initialization binds roles without adding tools', async () => {
  const current = await fixture();
  const selection = initialize(current);
  assert.deepEqual(selection.tools,tools);
  assert.deepEqual(selection.parameters.delegate_review.properties.role.enum,['reviewer','evidence','no-tools',null]);
  const limited = initialize(current,{tools:['review_scope']});
  assert.deepEqual(limited.tools,['review_scope']);
  assert.deepEqual(limited.parameters,{});
});

test('receipt mode rejects plan and read-only roles', async () => {
  for (const [mode,role,available] of [
    ['plan','reviewer',tools],['code','evidence',tools],['code','reviewer',['delegate_review']],
    ['flagship','evidence',tools],['flagship','reviewer',['delegate_review']],
  ]) {
    const current = await fixture();
    const scope = await current.invoke('scope',{});
    initialize(current,{mode,tools:available});
    const before = current.calls.length;
    const result = await current.delegate({review_id:scope.review_id,groups:[scope.groups[0].id],
      record_receipt:true,role});
    assert.equal(result.isError,true);
    assert.equal(result.error.code,'permission_denied');
    assert.equal(current.calls.length,before);
  }
});

test('tool-disabled roles cannot read review evidence', async () => {
  for (const mode of ['code','flagship']) {
    const current = await fixture();
    initialize(current,{mode});
    const scope = await current.invoke('scope',{});
    for (const record_receipt of [true,false]) {
      const result = await current.delegate({review_id:scope.review_id,groups:[scope.groups[0].id],
        record_receipt,role:'no-tools'});
      assert.equal(result.isError,true);
      assert.match(result.error.message,/cannot read frozen patches/);
    }
  }
});

test('evidence-only handoff cannot claim formal coverage', async () => {
  for (const mode of ['plan','code','flagship']) {
    const current = await fixture();
    initialize(current,{mode});
    const scope = await current.invoke('scope',{});
    const output = await current.delegate({review_id:scope.review_id,groups:[scope.groups[0].id],
      record_receipt:false,role:'evidence'});
    assert.equal(output.role,'evidence');
    assert.equal(captured(output.task).record_receipt,false);
    assert.match(output.task,/Do not call review_scope, report_finding or review_complete/);
    assert.match(output.task,/parent independently checks all files/);
    const reread = await current.invoke('scope',{review_id:scope.review_id});
    assert.equal(reread.status,'open');
    assert.deepEqual(reread.reviewed_groups,[]);
  }
});

test('unknown modes cannot delegate review groups', async () => {
  for (const mode of [undefined,null,'','chat','Flagship']) {
    const current = await fixture();
    const scope = await current.invoke('scope',{});
    initialize(current,{mode});
    const before = current.calls.length;
    for (const record_receipt of [true,false]) {
      const result = await current.delegate({review_id:scope.review_id,groups:[scope.groups[0].id],record_receipt});
      assert.equal(result.isError,true);
      assert.equal(result.error.code,'permission_denied');
    }
    assert.equal(current.calls.length,before);
  }
});

test('invalid roles groups and child reuse fail closed', async () => {
  const current = await fixture();
  initialize(current);
  const scope = await current.invoke('scope',{});
  const input = {review_id:scope.review_id,groups:[scope.groups[0].id],record_receipt:true};
  for (const change of [{role:'not-captured'},{groups:[]},{groups:['unknown']},
    {groups:[...input.groups,...input.groups]},{session:uuid(99)},{worktree:uuid(99)},
    {record_receipt:null},{background:null}]) {
    const result = await current.delegate({...input,...change});
    assert.equal(result.isError,true);
  }
  current.actor(uuid(99),scope.creator);
  assert.equal((await current.delegate(input)).error.code,'permission_denied');
});

test('missing metadata is rejected instead of omitted', async () => {
  for (const field of ['snapshot','metadata_revision','revision']) {
    const current = await fixture();
    initialize(current);
    const scope = await current.invoke('scope',{});
    for (const record of current.values.values()) {
      if (field === 'snapshot' && record.value.kind === 'head') delete record.value.snapshot.metadata_revision;
      if (field !== 'snapshot' && record.value.kind === 'scope') {
        for (const row of record.value.rows.filter(row => row.kind === 'file')) delete row.value[field];
      }
    }
    const result = await current.delegate({review_id:scope.review_id,groups:[scope.groups[0].id],record_receipt:true});
    assert.equal(result.isError,true);
  }
});

test('already covered groups cannot be delegated again', async () => {
  const current = await fixture([file('src/main.rs'),file('separate/other.rs')]);
  initialize(current);
  const scope = await current.invoke('scope',{});
  const group = scope.groups[0];
  const covered = {...scope,files:scope.files.filter(file => file.group === group.id),groups:[group]};
  assert.equal((await current.invoke('complete',receipt(covered,{finalize:false}))).status,'open');
  const result = await current.delegate({review_id:scope.review_id,groups:[group.id],record_receipt:true});
  assert.equal(result.error.code,'conflict');
});

test('oversized whole groups are never sampled', async () => {
  const current = await fixture(Array.from({length:70},(_,index) => file(`app/locales/${'x'.repeat(1800)}/file-${index}.json`)));
  initialize(current);
  const scope = await current.capture({limit:1});
  assert.equal(scope.counts.files,70);
  const group = scope.files[0].group;
  const result = await current.delegate({review_id:scope.review_id,groups:[group],record_receipt:true});
  assert.equal(result.isError,true);
  assert.match(result.error.message,/complete group exceeds the task budget/);
});


test('package upgrades continue the original frozen review', async () => {
  const current = await fixture([file('first/main.rs'),file('second/main.rs'),file('third/main.rs')]);
  initialize(current);
  const creator = current.captured.session;
  const scope = await current.invoke('scope',{});
  const original = plain(current.values.get(`review/${scope.review_id}/head`).value);
  const initial = scope.groups[0];
  current.actor(uuid(99),creator);
  const initialFiles = scope.files.filter(file => file.group === initial.id);
  const accepted = await current.invoke('complete',receipt({...scope,files:initialFiles,groups:[initial]},
    {finalize:false}));
  assert.equal(accepted.status,'open');
  current.actor(creator);
  const priorReceipt = plain([...current.values].filter(([key]) => key.includes('/receipt/')));
  current.captured.package.digest = 'digest-b';
  current.captured.package.settings_revision = '1';
  const before = plain([...current.values]);
  const resumed = await current.invoke('scope',{review_id:scope.review_id});
  assert.equal(resumed.review_id,scope.review_id);
  assert.deepEqual(resumed.snapshot,scope.snapshot);
  assert.deepEqual(resumed.files,scope.files);
  assert.deepEqual(resumed.reviewed_groups,[initial.id]);
  assert.deepEqual(plain([...current.values]),before);

  const git = await current.module('git');
  for (const [index,group] of scope.groups.slice(1).entries()) {
    const output = await current.delegate({review_id:scope.review_id,
      groups:[group.id],record_receipt:true});
    const capture = captured(output.task);
    assert.equal(capture.review_id,scope.review_id);
    assert.deepEqual(capture.snapshot,scope.snapshot);
    assert.deepEqual(capture.groups,[group]);
    assert.deepEqual(capture.files,scope.files.filter(file => file.group === group.id));
    current.actor(uuid(100+index),creator);
    const child = await current.invoke('scope',{review_id:capture.review_id});
    assert.deepEqual(child.snapshot,capture.snapshot);
    for (const file of capture.files) {
      const start = current.calls.length;
      const patch = await git.change(current.captured.worktree,capture.snapshot,file,true);
      assert.equal(patch.metadata_revision,file.metadata_revision);
      assert.equal(patch.revision,file.revision);
      for (const call of current.calls.slice(start)) {
        assert.deepEqual(call.data.snapshot,capture.snapshot);
        assert.equal(call.data.expected_metadata_revision,file.metadata_revision);
        assert.equal(call.data.expected_revision,file.revision);
      }
    }
    const receiptInput = receipt(capture,{finalize:false});
    assert.equal((await current.invoke('complete',{...receiptInput,finalize:true})).error.code,'permission_denied');
    assert.equal((await current.invoke('complete',receiptInput)).status,'open');
    current.actor(creator);
  }
  const completed = await current.invoke('complete',receipt(scope,
    {reviewed_files:[],reviewed_groups:[]}));
  assert.equal(completed.status,'complete');
  assert.equal(completed.reviewed_files,scope.files.length);
  assert.deepEqual(completed.reviewed_groups,scope.groups.map(group => group.id));
  const head = current.values.get(`review/${scope.review_id}/head`).value;
  assert.deepEqual(head.package,original.package);
  assert.deepEqual(head.snapshot,original.snapshot);
  for (const [key,value] of priorReceipt) assert.deepEqual(plain(current.values.get(key)),value);
});

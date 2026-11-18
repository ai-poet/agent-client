import assert from 'node:assert/strict';
import test from 'node:test';
import {file,fixture,finding} from './support.mjs';

const lines = [{kind:'context',old_line:2,new_line:2,text:'if (ready) {'},
  {kind:'deleted',old_line:3,new_line:null,text:'return original();'},
  {kind:'added',old_line:null,new_line:3,text:'return changed();'},
  {kind:'context',old_line:4,new_line:4,text:'}'}];

test('unique snippets correct suggested line numbers', async () => {
  const fixtureData = await fixture([file('src/main.js',{lines})]);
  const scope = await fixtureData.invoke('scope',{});
  const result = await fixtureData.invoke('finding',finding(scope,{line:999,snippet:'if (ready) {\nreturn changed();\n'}));
  assert.equal(result.finding.line,3);
  assert.equal(result.finding.side,'new');
  assert.equal(result.finding.revision,scope.files[0].revision);
  assert.equal(result.finding.reporter,fixtureData.captured.session);
});

test('deleted lines use old-side locations', async () => {
  const fixtureData = await fixture([file('src/main.js',{lines})]);
  const scope = await fixtureData.invoke('scope',{});
  const result = await fixtureData.invoke('finding',finding(scope,{side:'old',snippet:'return original();',line:55}));
  assert.equal(result.finding.line,3);
  assert.equal(result.finding.side,'old');
  assert.equal((await fixtureData.invoke('finding',finding(scope,{side:'new',snippet:'return original();'}))).isError,true);
});

test('absent ambiguous and context-only anchors fail', async () => {
  const duplicate = [...lines,{kind:'added',old_line:null,new_line:10,text:'return changed();'}];
  const fixtureData = await fixture([file('src/main.js',{lines:duplicate})]);
  const scope = await fixtureData.invoke('scope',{});
  const before=fixtureData.values.size;
  for (const snippet of ['missing();','return changed();','if (ready) {']) {
    const result = await fixtureData.invoke('finding',finding(scope,{snippet}));
    assert.equal(result.isError,true);
  }
  assert.equal(fixtureData.values.size,before);
});

test('scoped files revisions and evidence are mandatory', async () => {
  const fixtureData = await fixture();
  const scope = await fixtureData.invoke('scope',{});
  for (const change of [{path:'.env'},{revision:'stale'},{severity:'P4'},{evidence:' '},{unexpected:true}]) {
    const result = await fixtureData.invoke('finding',finding(scope,change));
    assert.equal(result.isError,true);
  }
  fixtureData.change(scope.files[0].path,{revision:'updated'});
  const result = await fixtureData.invoke('finding',finding(scope));
  assert.equal(result.finding.revision,scope.files[0].revision);
});

test('anchoring exhausts mapped pages under one strong revision', async () => {
  const many = Array.from({length:1100},(_,index) => ({kind:'added',old_line:null,new_line:index+1,text:`changed_${index}();`}));
  const fixtureData = await fixture([file('src/main.js',{lines:many})]);
  const scope = await fixtureData.invoke('scope',{});
  const result = await fixtureData.invoke('finding',finding(scope,{snippet:'changed_1099();',line:1}));
  assert.equal(result.finding.line,1100);
  const pages = fixtureData.calls.filter(call => call.kind === 'read_git_change');
  assert.deepEqual(pages.map(page => page.data.offset),[0,500,1000]);
  assert.ok(pages.slice(1).every(page => page.data.expected_revision === scope.files[0].revision));
});

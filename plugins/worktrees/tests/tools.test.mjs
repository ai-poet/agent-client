import test from 'node:test';
import assert from 'node:assert/strict';
import {list,create,register,remove} from '../dev.sailry.platform/host/main.js';

test('creation keeps supplied revisions and defaults to a clean checkout', () => {
  const args = {branch:'feature/资料',expected_head:'head',expected_index:'index'};
  assert.deepEqual(create(args),{...args,include_changes:false});
  assert.deepEqual(create({...args,include_changes:true}),{...args,include_changes:true});
  assert.equal(create({...args,include_changes:null}).isError,true);
  assert.equal(create({branch:'feature'}).isError,true);
});

test('registration and removal keep frozen resource identity', () => {
  assert.deepEqual(list({}),{});
  assert.deepEqual(register({path:'/projects/资料'}),{path:'/projects/资料'});
  const args = {worktree:'registered-tree',expected_head:'head',expected_branch:'branch'};
  assert.deepEqual(remove(args),args);
  assert.equal(remove({worktree:'tree'}).isError,true);
});

test('a package cannot substitute the admitted project or source', () => {
  assert.equal(list({project:'other'}).isError,true);
  assert.equal(register({path:'/tree',project:'other'}).isError,true);
  assert.equal(create({branch:'branch',expected_head:'head',expected_index:'index',source:'other'}).isError,true);
  assert.equal(remove({worktree:'tree',expected_head:'head',expected_branch:'branch',force:true}).isError,true);
});

test('removal guidance preserves unintegrated work and history', async () => {
  const {readFile} = await import('node:fs/promises');
  const manifest = JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  const tools = manifest.extensions['dev.sailry.platform'].tools;
  const remove = tools.find(tool => tool.name === 'remove_worktree').description;
  assert.match(remove,/including ones with running or queued turns, cannot be removed/);
  assert.match(remove,/Sessions that used it keep their history but cannot run new turns there/);
  assert.match(remove,/stop and report conflicts to the user without resolving them automatically/);
  assert.match(remove,/never discard work or session history/);
  const create = tools.find(tool => tool.name === 'create_worktree').description;
  assert.match(create,/remove its worktree when it has no changes and no new commits/);
});

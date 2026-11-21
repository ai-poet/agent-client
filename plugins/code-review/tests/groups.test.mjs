import assert from 'node:assert/strict';
import test from 'node:test';
import {groups, peers} from '../dev.sailry.platform/host/groups.js';

const scoped = paths => paths.map(path => ({path}));
const sizes = result => result.map(group => group.files.length);

test('cap splits a large directory into bounded groups', () => {
  const paths = Array.from({length:25},(_,index) => `src/file-${String(index).padStart(2,'0')}.rs`);
  const result = groups(scoped(paths));
  assert.deepEqual(sizes(result),[peers,peers,5]);
  assert.deepEqual(result.flatMap(group => group.files),paths);
  assert.deepEqual(result.map(group => group.id),['group-1','group-2','group-3']);
});

test('small directories stay in one group', () => {
  const result = groups(scoped(['src/a.rs','src/b.rs','src/c.rs']));
  assert.deepEqual(result,[{id:'group-1',files:['src/a.rs','src/b.rs','src/c.rs']}]);
});

test('counterparts stay together beyond the cap', () => {
  const sources = Array.from({length:12},(_,index) => `src/item-${String(index).padStart(2,'0')}.js`);
  const tests = sources.map(path => path.replace('src/','tests/').replace('.js','.test.js'));
  const result = groups(scoped([...sources,...tests]));
  for (const [index,source] of sources.entries()) {
    const owner = result.find(group => group.files.includes(source));
    assert.ok(owner.files.includes(tests[index]),`${source} lost its test`);
  }
  assert.ok(result.length > 1);
});

test('locale sets stay together beyond the cap', () => {
  const paths = Array.from({length:14},(_,index) => `app/locales/lang-${String(index).padStart(2,'0')}.json`);
  assert.deepEqual(sizes(groups(scoped(paths))),[14]);
});

test('identical input yields identical groups', () => {
  const paths = [...Array.from({length:23},(_,index) => `lib/mod-${String(index).padStart(2,'0')}.py`),
    'lib/tests/test_mod-03.py','docs/guide.md'];
  assert.deepEqual(groups(scoped(paths)),groups(scoped(paths)));
  assert.ok(sizes(groups(scoped(paths))).every(size => size <= peers));
});

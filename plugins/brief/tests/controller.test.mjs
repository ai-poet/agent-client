import assert from 'node:assert/strict';
import test from 'node:test';
import {Controller} from '../dev.sailry.platform/desktop/controller.js';

function fixture(revision) {
  const calls = [],errors = [],resource = {id:'artifact',session:'session',...(revision === undefined ? {} : {revision})};
  const api = {async readArtifact(id,version) {calls.push(['read',id,version]);return {artifact:{id,title:`Version ${version ?? 2}`,revision:version ?? 2},html:'<p>Body</p>'};},
    async listArtifactRevisions(id,before) {calls.push(['revisions',id,before]);return {items:[{revision:2},{revision:1}],next_before:null};},
    async exportArtifact(value) {calls.push(['export',value]);return true;}};
  let notifications = 0;
  const state = new Controller(api,resource,() => notifications++,key => errors.push(key));
  return {state,api,calls,errors,notifications:() => notifications};
}

test('latest refresh follows changes but an explicit version stays pinned',async () => {
  const latest = fixture();await latest.state.refresh();await latest.state.refresh();
  assert.equal(latest.state.revision,2);assert(latest.calls.filter(call => call[0] === 'read').every(call => call[2] === undefined));
  await latest.state.select('1');await latest.state.refresh();
  assert.equal(latest.state.revision,1);assert.deepEqual(latest.calls.filter(call => call[0] === 'read').slice(-2).map(call => call[2]),[1,1]);
  const pinned = fixture(1);await pinned.state.refresh();assert.equal(pinned.state.revision,1);
});
test('selecting the current latest pins subsequent publications',async () => {
  const {state,api,calls} = fixture();await state.refresh();await state.select('2');
  assert.equal(state.follow,false);
  api.readArtifact = async (id,version) => {
    calls.push(['read',id,version]);return {artifact:{id,revision:version ?? 3},html:'<p>Body</p>'};
  };
  api.listArtifactRevisions = async () => ({items:[{revision:3},{revision:2},{revision:1}],next_before:null});
  await state.refresh();assert.equal(state.revision,2);assert.equal(state.artifact.revision,2);
  assert.equal(calls.filter(call => call[0] === 'read').at(-1)[2],2);
});
test('failed reads preserve content and revert unconfirmed selections',async () => {
  const {state,api,errors} = fixture();await state.refresh();const original = state.artifact;
  api.readArtifact = async () => {throw Error('Unavailable');};
  await state.select('1');assert.equal(state.artifact,original);assert.equal(state.revision,2);assert.equal(state.follow,true);
  assert.deepEqual(errors,['brief_load_failed']);assert.equal(state.loading,false);
});
test('concurrent invalidations coalesce into one subsequent read',async () => {
  const {state,api,calls} = fixture();let resolve;
  const read = api.readArtifact;api.readArtifact = (...args) => new Promise(done => {resolve = () => done(read(...args));});
  const pending = state.refresh();await state.refresh();await state.refresh();
  resolve();await new Promise(done => setImmediate(done));resolve();await pending;
  assert.equal(calls.filter(call => call[0] === 'read').length,2);assert.equal(state.loading,false);
});
test('rapid selections keep the committed version and apply only the latest read',async () => {
  const {state,api,calls} = fixture();await state.refresh();state.items.unshift({revision:3});
  const reads = new Map();
  api.readArtifact = (id,revision) => new Promise(resolve => reads.set(revision,artifact => resolve({artifact:{id,...artifact},html:'<p>Body</p>'})));
  const first = state.select('1');
  assert.equal(state.revision,2);assert.equal(state.artifact.revision,2);assert.equal(state.follow,true);assert.equal(state.loading,true);
  await state.export('html');
  assert.deepEqual(calls.filter(call => call[0] === 'export').at(-1),['export',{artifact:'artifact',revision:2,format:'html'}]);
  const latest = state.select('3');
  reads.get(1)({revision:1});await first;
  assert.equal(state.revision,2);assert.equal(state.artifact.revision,2);assert.equal(state.follow,true);assert.equal(state.loading,true);
  reads.get(3)({revision:3});await latest;
  assert.equal(state.revision,3);assert.equal(state.artifact.revision,3);assert.equal(state.follow,false);assert.equal(state.loading,false);
});
test('historic pages preserve sorted choices across selections and refreshes',async () => {
  const {state,api} = fixture(80);
  api.listArtifactRevisions = async (_id,before) => {
    assert.equal(before,undefined);
    return {items:Array.from({length:16},(_,index) => ({revision:80-index})),next_before:65};
  };
  await state.refresh();
  api.listArtifactRevisions = async (_id,before) => {
    if (before === undefined) return {items:[{revision:81},{revision:80}],next_before:80};
    assert.equal(before,65);
    return {items:Array.from({length:16},(_,index) => ({revision:65-index})),next_before:null};
  };
  await state.more();assert.equal(state.items.length,31);assert.equal(state.before,null);
  assert.deepEqual(state.items.map(item => item.revision),Array.from({length:31},(_,index) => 80-index));
  await state.select('50');assert.equal(state.revision,50);assert.equal(state.items.length,32);
  await state.select('80');assert.equal(state.revision,80);
  assert.deepEqual(state.versions().map(item => item.id),Array.from({length:32},(_,index) => String(81-index)));
});
test('export captures the visible version and prevents duplicate launches',async () => {
  const {state,api,calls} = fixture(1);await state.refresh();let resolve;
  api.exportArtifact = value => {calls.push(['export',value]);return new Promise(done => {resolve = done;});};
  const pending = state.export('png');await state.export('html');assert.equal(state.exporting,true);
  assert.deepEqual(calls.filter(call => call[0] === 'export'),[['export',{artifact:'artifact',revision:1,format:'png'}]]);
  resolve(false);await pending;assert.equal(state.exporting,false);
  state.connected = false;await state.export('html');assert.equal(calls.filter(call => call[0] === 'export').length,1);
});
test('release ignores late reads',async () => {
  const {state,api} = fixture();let resolve;
  api.readArtifact = () => new Promise(done => {resolve = done;});
  const pending = state.refresh();state.stop();resolve({artifact:{id:'artifact',revision:2}});await pending;
  assert.equal(state.artifact,null);assert.equal(state.closed,true);
});
test('preview export rejects PNG without launching',async () => {
  const {state,calls,errors} = fixture(1);await state.refresh();
  state.artifact.kind = 'preview';
  await state.export('png');
  assert.deepEqual(errors,['brief_export_unsupported']);
  assert(!calls.some(call => call[0] === 'export'));
  assert.equal(state.exporting,false);
  await state.export('html');
  assert.deepEqual(calls.filter(call => call[0] === 'export'),[
    ['export',{artifact:'artifact',revision:1,format:'html'}],
  ]);
});
test('form edits pin the mounted version before a newer refresh',async () => {
  const {state,calls} = fixture();await state.refresh();await state.freeze(2);await state.refresh();
  assert.equal(state.follow,false);assert.equal(state.revision,2);
  assert.equal(calls.filter(call => call[0] === 'read').at(-1)[2],2);
});
test('an edit invalidates a newer read already in flight',async () => {
  const {state,api,calls} = fixture();await state.refresh();let resolve;
  const read = api.readArtifact;
  api.readArtifact = (id,version) => version === undefined
    ? new Promise(done => {resolve = () => done({artifact:{id,revision:3}});}) : read(id,version);
  const pending = state.refresh();await state.freeze(2);resolve();await pending;
  assert.equal(state.artifact.revision,2);assert.equal(state.revision,2);assert.equal(state.follow,false);
  assert.equal(calls.filter(call => call[0] === 'read').at(-1)[2],2);
});

import {context, getValue, readSession} from 'sailry/sdk';
import {clone, fail, identifier, limits, size} from './policy.js';

export const key = (id, suffix) => `review/${identifier(id)}/${suffix}`;
export async function identity() {
  const captured = context();
  if (!captured.session || !captured.worktree || captured.package?.name !== 'code-review') {
    fail('Review requires a worktree conversation', 'permission_denied');
  }
  const session = await readSession();
  if (session.id !== captured.session || session.worktree !== captured.worktree) {
    fail('Review session scope changed', 'permission_denied');
  }
  return {session:captured.session, worktree:captured.worktree, package:captured.package,
    parent:session.delegation?.session ?? null};
}
export async function record(name) {
  const value = await getValue(name);
  if (value.key !== name || typeof value.revision !== 'string' || !/^(0|[1-9][0-9]*)$/.test(value.revision)) {
    fail('Invalid review storage record', 'internal');
  }
  return value;
}
export async function head(id, actor) {
  const current = await record(key(id, 'head'));
  if (!current.present) fail('Review does not exist', 'not_found');
  const value = current.value;
  if (!value || value.v !== 1 || value.kind !== 'head' || value.id !== id
      || !['open','complete'].includes(value.status) || !Array.isArray(value.pages)
      || !Array.isArray(value.findings) || !Array.isArray(value.reviewed_groups)
      || !Array.isArray(value.receipts)) fail('Review storage is incompatible', 'invalid_request');
  if (actor.worktree !== value.worktree
      || (actor.session !== value.creator && actor.parent !== value.creator)) {
    fail('Review belongs to another session or worktree', 'permission_denied');
  }
  if (actor.package.name !== value.package?.name) {
    fail('Review belongs to another package', 'permission_denied');
  }
  return {record:current, value:clone(value)};
}
export function open(value) {
  if (value.status !== 'open') fail('Review is already complete', 'conflict');
}
export async function manifest(value) {
  const rows = [];
  const pages=await Promise.all(value.pages.map(page=>record(key(value.id,`scope/${page}`))));
  for (const current of pages) {
    if (!current.present || current.value?.v !== 1 || current.value?.kind !== 'scope'
        || !Array.isArray(current.value.rows)) fail('Review scope is incomplete', 'internal');
    rows.push(...current.value.rows);
  }
  const files = rows.filter(row => row.kind === 'file').map(row => row.value);
  const groups = rows.filter(row => row.kind === 'group').map(row => row.value);
  const skipped = rows.filter(row => row.kind === 'skipped').map(row => row.value);
  if (files.length !== value.counts.files || groups.length !== value.counts.groups
      || skipped.length !== value.counts.skipped || rows.length !== files.length + groups.length + skipped.length) {
    fail('Review scope is incomplete', 'internal');
  }
  return {rows, files, groups, skipped};
}
export function write(name, value, expected = '0') {
  if (size(value) > limits.value) fail('Review record exceeds the bounded storage budget');
  return {kind:'write',data:{key:name,value,expected_revision:expected}};
}
export function shards(id, suffix, kind, rows) {
  const operations = [], pages = [];
  let pending = [];
  const append = () => {
    const page = pages.length;
    operations.push(write(key(id, `${suffix}/${page}`), {v:1,kind,rows:pending}));
    pages.push(page);
    pending = [];
  };
  for (const row of rows) {
    if (size({v:1,kind,rows:[row]}) > limits.shard) fail('Review record exceeds the bounded scope budget');
    if (size({v:1,kind,rows:[...pending,row]}) > limits.shard) append();
    pending.push(row);
  }
  if (pending.length) append();
  return {operations,pages};
}
export function transaction(operations, result) {
  if (size(result) > limits.output || !operations.length || operations.length > 64 || size(operations) + size(result) > limits.transaction) {
    fail('Review exceeds the bounded transaction budget');
  }
  return {call:{operation:'storage.transaction',arguments:{operations}}, state:{result,
    writes:operations.map(operation => ({key:operation.data.key,revision:operation.data.expected_revision}))}};
}
export function committed(outcome, state) {
  if (outcome?.isError) return outcome;
  if (outcome?.kind !== 'plugin_transaction' || !Array.isArray(outcome.data)
      || !state || !Array.isArray(state.writes) || outcome.data.length !== state.writes.length) {
    fail('Review transaction result is unavailable', 'outcome_unknown');
  }
  for (let index = 0; index < state.writes.length; index++) {
    const expected = state.writes[index], output = outcome.data[index];
    if (output.kind !== 'plugin_value' || output.data.key !== expected.key || !output.data.present
        || output.data.revision !== (BigInt(expected.revision)+1n).toString()) {
      fail('Review transaction result is unavailable', 'outcome_unknown');
    }
  }
  return state.result;
}

import {verify} from './content.js';
import {fail, fields, identifier, limits, path, revision, size} from './policy.js';
import {head, key, manifest, open, shards, transaction, write} from './records.js';
import {completion} from './locales.js';

export function display(value) {
  return {...value,sailry_result:{version:1,diagnostics:[],
    status:completion(value.reviewed_files)}};
}

function remaining(scope, covered) {
  const files = scope.files.filter(file => !covered.has(file.group))
    .map(file => ({path:file.path,revision:file.revision}));
  const groups = scope.groups.filter(group => !covered.has(group.id)).map(group => group.id);
  const visible = [];
  let used = 0;
  for (const file of files) {
    const count = size(file);
    if (visible.length === limits.page || used+count > limits.output/2) break;
    visible.push(file);
    used += count;
  }
  return {files:visible,groups:groups.slice(0,limits.page),
    counts:{files:files.length,groups:groups.length},more:visible.length < files.length || groups.length > limits.page};
}

function receipt(args, scope, value) {
  if (!Array.isArray(args.reviewed_files)
      || !Array.isArray(args.reviewed_groups)
      || typeof args.finalize !== 'boolean') fail('Invalid review receipt');
  const files = new Map(), groups = new Set();
  for (const file of args.reviewed_files) {
    fields(file,['path','revision'],['path','revision']);
    path(file.path);
    revision(file.revision);
    if (files.has(file.path)) fail('Duplicate reviewed file');
    const expected = scope.files.find(candidate => candidate.path === file.path);
    if (!expected) fail('Reviewed file is outside the scope');
    if (expected.revision !== file.revision) fail('Receipt uses a stale file revision','revision_conflict');
    files.set(file.path,file.revision);
  }
  for (const group of args.reviewed_groups) {
    if (typeof group !== 'string' || groups.has(group)) fail('Duplicate or invalid reviewed group');
    const expected = scope.groups.find(candidate => candidate.id === group);
    if (!expected) fail('Reviewed group is outside the scope');
    if (value.reviewed_groups.includes(group)) fail('Review group already has a receipt','conflict');
    if (expected.files.some(path => !files.has(path))) fail('Review receipt must cover whole groups');
    groups.add(group);
  }
  if (scope.files.some(file => files.has(file.path) && !groups.has(file.group))) {
    fail('Reviewed files require their complete group receipt');
  }
  if (!args.finalize && !groups.size) fail('Child or partial completion requires a whole group receipt');
  return {files,groups};
}
export async function prepare(input, actor, intent) {
  const args = fields(input,['review_id','reviewed_files','reviewed_groups','finalize'],
    ['review_id','reviewed_files','reviewed_groups','finalize']);
  identifier(args.review_id);
  const current = await head(args.review_id,actor);
  open(current.value);
  if (args.finalize && actor.session !== current.value.creator) fail('Only the review creator may finalize','permission_denied');
  const scope = await manifest(current.value);
  const incoming = receipt(args,scope,current.value);
  const covered = new Set([...current.value.reviewed_groups,...incoming.groups]);
  const pending = remaining(scope,covered);
  if (args.finalize && (pending.counts.files || pending.counts.groups)) {
    fail('Review coverage is incomplete','invalid_request',pending);
  }
  await Promise.all(scope.files.filter(file=>args.finalize||incoming.files.has(file.path)).map(file=>verify(current.value,file)));
  const operations = [];
  current.value.reviewed_groups = scope.groups.map(group => group.id).filter(id => covered.has(id));
  if (incoming.groups.size) {
    const id = identifier(intent);
    if (current.value.receipts.includes(id)) fail('Receipt was already recorded','conflict');
    const rows = [...args.reviewed_files.map(value => ({kind:'file',value})),
      ...args.reviewed_groups.map(value => ({kind:'group',value})),{kind:'reporter',value:actor.session}];
    const stored = shards(args.review_id,`receipt/${id}`,'receipt',rows);
    operations.push(...stored.operations);
    current.value.receipts.push(id);
  }
  if (args.finalize) current.value.status = 'complete';
  operations.unshift(write(key(args.review_id,'head'),current.value,current.record.revision));
  return transaction(operations,{review_id:args.review_id,status:current.value.status,
    reviewed_files:scope.files.filter(file => covered.has(file.group)).length,
    reviewed_groups:current.value.reviewed_groups,remaining:pending,
    findings:current.value.findings.length});
}

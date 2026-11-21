import {readTurnState, stageTurnState} from 'sailry/sdk';
import {clone, fail, fields, identifier, limits, revision, size, text} from './policy.js';
import {head, identity, manifest, open} from './records.js';
import {available} from './content.js';

const receiptTools = ['review_scope', 'report_finding', 'review_complete'];
const inputs = ['review_id', 'groups', 'record_receipt', 'role', 'title', 'background'];

function parameters(roles) {
  return {type:'object', additionalProperties:false,
    required:['review_id','groups','record_receipt'], properties:{
      review_id:{type:'string',pattern:'^[0-9a-fA-F]{8}(-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}$'},
      groups:{type:'array',minItems:1,uniqueItems:true,
        items:{type:'string',minLength:1}},
      record_receipt:{type:'boolean',description:'Require a formal receipt, or explicitly gather evidence only'},
      role:{type:['string','null'],enum:[...roles.map(role => role.key),null],default:null,
        description:'Omit for a new child inheriting the current turn; named roles retain their captured mode and model'},
      title:{type:'string',minLength:1},
      background:{type:'boolean'},
    }};
}

export function initialize({tools, roles, mode}) {
  const captured = roles.map(({key,read_only,tools}) => ({key,read_only,tools}));
  stageTurnState({mode,roles:captured,recording:receiptTools.every(name => tools.includes(name))});
  const roster = roles.map(role => `${role.key}${role.read_only ? ' [read only]' : ''}${role.tools === false ? ' [no tools]' : ''}: ${role.name}`).join('\n');
  return {tools, parameters:tools.includes('delegate_review')
    ? {delegate_review:parameters(captured)} : {},
    instruction:tools.includes('delegate_review')
      ? `Use delegate_review for complete frozen review groups. Select record_receipt explicitly. Omit role to inherit the current turn; read-only roles gather evidence only.${roster ? `\nCaptured roles:\n${roster}` : ''}` : ''};
}

function capability(args) {
  const current = readTurnState();
  const writable = ['code','flagship'].includes(current?.mode);
  if (!current || (!writable && current.mode !== 'plan') || !Array.isArray(current.roles)) {
    fail('Review delegation capture is unavailable; start a new turn', 'permission_denied');
  }
  if (typeof args.record_receipt !== 'boolean') fail('Select record_receipt explicitly');
  const role = args.role == null ? null : current.roles.find(role => role.key === args.role);
  if (args.role != null && !role) fail('Select a role from this turn\'s captured roster');
  if (role?.tools === false) fail('The selected role model cannot read frozen patches; choose a tool-capable role');
  if (args.record_receipt && (!writable || role?.read_only || !current.recording)) {
    fail('Formal receipts require an authorized Code or Flagship turn and a writable role; choose evidence-only explicitly instead', 'permission_denied');
  }
}

function capture(value, scope, selected, recording) {
  if (!Array.isArray(selected) || !selected.length
      || selected.some(group => typeof group !== 'string') || new Set(selected).size !== selected.length) {
    fail('Select distinct complete review groups');
  }
  const groups = scope.groups.filter(group => selected.includes(group.id));
  if (groups.length !== selected.length) fail('A selected group is outside the review');
  if (groups.some(group => value.reviewed_groups.includes(group.id))) fail('A selected group already has a receipt', 'conflict');
  const files = scope.files.filter(file => selected.includes(file.group));
  if (!files.length || groups.some(group => group.files.some(path => !files.some(file => file.path === path)))) {
    fail('The frozen review group is incomplete', 'internal');
  }
  if (!value.snapshot?.target) fail('Frozen snapshot metadata is incomplete', 'internal');
  revision(value.snapshot.metadata_revision);
  for (const file of files) {
    available(file);
    revision(file.metadata_revision);
    revision(file.revision);
    if (!file.rule?.path || !file.rule.sha256) fail('Frozen review rules are incomplete', 'internal');
  }
  return {review_id:value.id, record_receipt:recording, snapshot:clone(value.snapshot),
    groups:clone(groups), files:clone(files)};
}

function task(value) {
  const purpose = value.record_receipt
    ? 'Review every assigned group and submit report_finding for established issues, then review_complete with exact whole-group reviewed_files/reviewed_groups and finalize:false. Only the creator finalizes.'
    : 'Gather evidence only. Do not call review_scope, report_finding or review_complete, and do not claim a formal receipt. Return evidence with each assigned path/revision; the authorized parent independently checks all files and records findings and coverage.';
  const instructions = [
    'Review only the complete groups in this frozen capture. A review does not authorize edits, commits or publication.',
    'Load code-review:code-review. Load each file.rule.path with read_skill_resource and use its supplied rule SHA-256.',
    'Use read_review_change with review_id and each assigned path. Read every diff page and the saved old/new full context as needed, following next until null. These are the captured source bytes; do not verify them against live readFile or Git.',
    'Later repository edits do not invalidate this capture. Missing captured content is an error, never permission to substitute live source or widen access.',
    purpose,
    'Frozen review capture:',
    JSON.stringify(value),
  ].join('\n');
  if (size(instructions) > limits.output) fail('The complete group exceeds the task budget; use a smaller review target');
  return instructions;
}

export async function prepare(input) {
  const args = fields(input,inputs,['review_id','groups','record_receipt']);
  identifier(args.review_id);
  capability(args);
  if (args.title !== undefined) {
    text(args.title,'review title');
    if (/[\n\r]/.test(args.title)) fail('Invalid review title');
  }
  if (args.background !== undefined && typeof args.background !== 'boolean') fail('Invalid delegation background mode');
  const actor = await identity();
  const current = await head(args.review_id,actor);
  open(current.value);
  if (actor.session !== current.value.creator) fail('Only the review creator may delegate groups', 'permission_denied');
  const frozen = capture(current.value,await manifest(current.value),args.groups,args.record_receipt);
  return {task:task(frozen),title:args.title ?? 'Review captured changes',
    role:args.role ?? null,background:args.background ?? false};
}

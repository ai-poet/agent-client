import {context,inspectGit} from 'sailry/sdk';
import {labels} from './labels.js';

const clearsBadge = new Set(['revision_conflict','not_found','permission_denied','wrong_target']);

export async function badge() {
  const hidden = [{id:'changes',visible:false}];
  if (!context().worktree) return hidden;
  try {
    const status = await inspectGit();
    if (status.kind === 'directory') return hidden;
    let added = 0, removed = 0, partial = status.truncated || status.omitted_paths > 0;
    for (const entry of status.entries) {
      added += entry.diff.additions;
      removed += entry.diff.deletions;
      partial ||= entry.diff.truncated === true;
    }
    return [{id:'changes',visible:true,enabled:true,
      segments:[{label:{label:`+${added}`},tone:'success'},{label:{label:`-${removed}`},tone:'danger'}],
      details:[{label:labels.added,value:String(added)},{label:labels.removed,value:String(removed)},
        {label:labels.files,value:String(status.entries.length)},...(partial ? [{label:labels.partial,value:''}] : [])]}];
  } catch (error) {
    if (clearsBadge.has(error.code)) return hidden;
    throw error;
  }
}

// Argument policy from Sailry ef5e82ea's native Git tools.
function object(value, keys) {
  return value !== null && typeof value === 'object' && !Array.isArray(value) &&
    Object.keys(value).every(key => keys.includes(key));
}

function target(value) { return value === undefined || value === null || typeof value === 'string'; }
function invalid(message = 'Invalid Git arguments') {
  return {isError:true,error:{code:'invalid_request',message}};
}

export function status(args) {
  if (!object(args,['worktree']) || !target(args.worktree)) return invalid();
  return {worktree:args.worktree ?? null};
}

export function diff(args) {
  if (!object(args,['worktree','path','scope']) || !target(args.worktree) ||
      typeof args.path !== 'string') return invalid();
  const scope = args.scope ?? 'all';
  if (!['all','staged','unstaged'].includes(scope) || args.scope === null) return invalid();
  return {worktree:args.worktree ?? null,path:args.path,scope};
}

export function log(args) {
  if (!object(args,['worktree','limit','cursor','revision','path']) || !target(args.worktree) ||
      !target(args.revision) || !target(args.path)) return invalid();
  const {limit = 20,cursor = null} = args;
  if (!Number.isSafeInteger(limit) || limit < 1) return invalid('limit must be a positive safe integer');
  if (cursor !== null && (!object(cursor,['head','after','offset','path']) || !target(cursor.path) ||
      !Number.isSafeInteger(cursor.offset) || cursor.offset < 0 ||
      typeof cursor.head !== 'string' || !/^[0-9a-fA-F]{40}$/.test(cursor.head) ||
      typeof cursor.after !== 'string' || !/^[0-9a-fA-F]{40}$/.test(cursor.after))) return invalid();
  return {worktree:args.worktree ?? null,limit,cursor,revision:args.revision ?? null,path:args.path ?? null};
}

function page(args) {
  const {offset = 0, limit = 200} = args;
  return Number.isSafeInteger(offset) && offset >= 0 &&
    Number.isSafeInteger(limit) && limit >= 1 &&
    Number.isSafeInteger(offset + limit);
}

function pageError(args) {
  const {offset = 0, limit = 200} = args;
  if (!Number.isSafeInteger(offset) || offset < 0) {
    return invalid('offset must be a non-negative safe integer');
  }
  if (!Number.isSafeInteger(limit) || limit < 1) {
    return invalid('limit must be a positive safe integer');
  }
  return invalid('offset + limit must be a safe integer');
}

export function show(args) {
  if (!object(args,['worktree','revision','path','offset','limit']) || !target(args.worktree) ||
      (args.revision !== undefined && typeof args.revision !== 'string') ||
      (args.path !== undefined && args.path !== null && typeof args.path !== 'string')) return invalid();
  const {revision = 'HEAD', path = null, offset = 0, limit = 2000} = args;
  if (!page({offset,limit})) return pageError({offset,limit});
  if (path === null && offset !== 0) return invalid();
  return {worktree:args.worktree ?? null,revision,path,offset,limit};
}

export function blame(args) {
  if (!object(args,['worktree','revision','path','offset','limit']) || !target(args.worktree) ||
      !target(args.revision) || typeof args.path !== 'string') return invalid();
  if (!page(args)) return pageError(args);
  return {worktree:args.worktree ?? null,revision:args.revision ?? null,path:args.path,
    offset:args.offset ?? 0,limit:args.limit ?? 200};
}

export function changes(args) {
  if (!object(args,['worktree','target','cursor','limit']) || !target(args.worktree)) return invalid();
  if (!page({limit:args.limit})) return pageError({limit:args.limit});
  const cursor = args.cursor ?? null;
  const selected = args.target === undefined
    ? (cursor === null ? {kind:'worktree',data:{scope:'all'}} : null) : args.target;
  if ((selected === null) === (cursor === null) ||
      (selected !== null && !object(selected,['kind','data'])) ||
      (cursor !== null && (!object(cursor,['snapshot','after_path']) ||
        !object(cursor.snapshot,['target','metadata_revision']) || !target(cursor.after_path)))) return invalid();
  return {worktree:args.worktree ?? null,target:selected,cursor,limit:args.limit ?? 200};
}

export function change(args) {
  if (!object(args,['worktree','snapshot','path','expected_metadata_revision','expected_revision','offset','limit']) ||
      !target(args.worktree) || typeof args.path !== 'string' ||
      !object(args.snapshot,['target','metadata_revision']) ||
      typeof args.expected_metadata_revision !== 'string' || !target(args.expected_revision) ||
      ((args.offset ?? 0) > 0 && args.expected_revision == null)) return invalid();
  if (!page(args)) return pageError(args);
  return {worktree:args.worktree ?? null,snapshot:args.snapshot,path:args.path,
    expected_metadata_revision:args.expected_metadata_revision,expected_revision:args.expected_revision ?? null,
    offset:args.offset ?? 0,limit:args.limit ?? 200};
}

import {prepareRequest, completeRequest, forgetRequest} from 'sailry/sdk';
import {equal, fail, path, revision} from './policy.js';

async function read(kind, data, expected) {
  const request = prepareRequest({kind, data});
  try {
    const outcome = await completeRequest(request);
    if (outcome?.Err) fail(outcome.Err.message, outcome.Err.code);
    if (outcome?.Ok?.kind !== expected) fail('Unexpected Git result', 'internal');
    return outcome.Ok.data;
  } catch (error) {
    // These commands only read captured Git state. Keep the durable capture
    // available for an explicit continuation after a transient read failure.
    if (['busy','unavailable','expired'].includes(error.code)) error.retryableRead=true;
    throw error;
  } finally { forgetRequest(request); }
}
export async function inventory(worktree, target, snapshot = null) {
  const entries = [], seen = new Set();
  let cursor = snapshot ? {snapshot, after_path:null} : null;
  let frozen = snapshot;
  for (;;) {
    const page = await inventoryPage(worktree,target,cursor);
    if (frozen && !equal(frozen, page.snapshot)) fail('Review target changed', 'revision_conflict');
    frozen = page.snapshot;
    for (const entry of page.entries) {
      path(entry.path);
      if (entry.old_path != null) path(entry.old_path);
      revision(entry.metadata_revision);
      if (seen.has(entry.path)) fail('Git inventory contains duplicate paths', 'internal');
      seen.add(entry.path);
      entries.push(entry);
    }
    if (page.exhausted) {
      if (page.next != null) fail('Invalid Git inventory cursor', 'internal');
      return {snapshot:frozen, entries};
    }
    if (!page.entries.length || !page.next || !equal(page.next.snapshot, frozen)
        || page.next.after_path !== entries.at(-1).path) fail('Git inventory is incomplete');
    cursor = page.next;
  }
}
export async function inventoryPage(worktree,target,cursor = null) {
  const page=await read('list_git_changes',{worktree,target:cursor ? null : target,cursor,limit:200},'git_changes');
  if (!page.snapshot || !Array.isArray(page.entries) || page.omitted_paths !== 0
      || typeof page.exhausted !== 'boolean') fail('Git inventory is incomplete');
  if (cursor && !equal(cursor.snapshot,page.snapshot)) fail('Review target changed','revision_conflict');
  const seen=new Set(cursor?.after_path ? [cursor.after_path] : []);
  for (const entry of page.entries) {
    path(entry.path);
    if (entry.old_path != null) path(entry.old_path);
    revision(entry.metadata_revision);
    if (seen.has(entry.path)) fail('Git inventory contains duplicate paths','internal');
    seen.add(entry.path);
  }
  if (page.exhausted ? page.next != null : !page.entries.length || !page.next
      || !equal(page.next.snapshot,page.snapshot) || page.next.after_path !== page.entries.at(-1)?.path) {
    fail('Git inventory is incomplete');
  }
  return page;
}

export async function changePage(worktree,snapshot,file,offset = 0) {
  const page=await read('read_git_change',{worktree,snapshot,path:file.path,
    expected_metadata_revision:file.metadata_revision,expected_revision:file.revision ?? null,
    offset,limit:500},'git_change');
  revision(page.revision);
  if (!equal(page.snapshot,snapshot) || page.path !== file.path
      || page.metadata_revision !== file.metadata_revision || page.offset !== offset
      || (file.revision && file.revision !== page.revision) || !Array.isArray(page.lines)) {
    fail('Reviewed file changed','revision_conflict');
  }
  if (page.binary || page.skip_reason) {
    if (page.lines.length || page.next != null) fail('Skipped Git file returned content','internal');
    return page;
  }
  for (const line of page.lines) {
    if (!['added','deleted','context'].includes(line.kind) || typeof line.text !== 'string'
        || line.text.includes('\n')
        || (line.kind !== 'deleted' && (!Number.isSafeInteger(line.new_line) || line.new_line < 1))
        || (line.kind !== 'added' && (!Number.isSafeInteger(line.old_line) || line.old_line < 1))) {
      fail('Invalid mapped Git line','internal');
    }
  }
  if (page.next != null && (!Number.isSafeInteger(page.next)
      || page.next !== offset+page.lines.length || page.next <= offset)) fail('Invalid Git diff cursor','internal');
  return page;
}

export async function change(worktree, snapshot, file, all = false, visit = null) {
  let offset = 0, frozen = file.revision ?? null;
  const lines = [];
  for (;;) {
    const page = await changePage(worktree,snapshot,{...file,revision:frozen},offset);
    frozen = page.revision;
    if (page.binary || page.skip_reason) {
      if (page.lines.length) fail('Skipped Git file returned content', 'internal');
      return {...page, lines:[], revision:frozen};
    }
    if (!all) return {...page, revision:frozen};
    if (!visit) lines.push(...page.lines);
    if (visit) await visit(page);
    if (page.next == null) return {...page, lines, revision:frozen};
    if (!Number.isSafeInteger(page.next) || page.next !== offset + page.lines.length || page.next <= offset) {
      fail('Invalid Git diff cursor', 'internal');
    }
    offset = page.next;
  }
}

export async function contextPage(worktree,snapshot,file,side,offset = 0) {
  const page=await read('read_git_context',{worktree,snapshot,path:file.path,
    expected_metadata_revision:file.metadata_revision,expected_revision:file.revision,
    side,offset,limit:500},'git_context');
    if (!equal(page.snapshot,snapshot) || page.path !== file.path || page.side !== side
        || page.metadata_revision !== file.metadata_revision || page.revision !== file.revision
        || page.offset !== offset || typeof page.text !== 'string') fail('Reviewed context changed','revision_conflict');
    if (page.binary || page.skip_reason) fail('Complete review context is unavailable');
    const count=(page.text.match(/[^\n]*\n|[^\n]+$/g) ?? []).length;
    if (page.next != null && (!Number.isSafeInteger(page.next) || page.next !== offset+count || page.next <= offset)) {
      fail('Invalid Git context cursor','internal');
    }
  return page;
}
export async function context(worktree,snapshot,file,side,visit) {
  let offset=0;
  for (;;) {
    const page=await contextPage(worktree,snapshot,file,side,offset);
    await visit(page);
    if (page.next == null) return;
    offset=page.next;
  }
}

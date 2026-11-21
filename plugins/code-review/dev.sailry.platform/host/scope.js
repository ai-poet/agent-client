import {callId} from 'sailry/sdk';
import {begin,discard,finish,flush,recover,resume} from './capture.js';
import {advance,initialize,pending} from './capture/scope.js';
import {fail,fields,identifier,integer,limits,size} from './policy.js';
import {head,key,manifest,record,write} from './records.js';

export async function page(value, offset = 0, limit = limits.page, content = null) {
  integer(offset, 'review offset', 0, Number.MAX_SAFE_INTEGER);
  integer(limit, 'review page limit', 1, Number.MAX_SAFE_INTEGER);
  const scope = content ?? await manifest(value);
  const total = scope.rows.length + value.findings.length;
  if (offset > total) fail('Review offset exceeds the scope');
  const result = {review_id:value.id, snapshot:value.snapshot, status:value.status,
    creator:value.creator, counts:{...value.counts,findings:value.findings.length},
    reviewed_groups:value.reviewed_groups, offset, next:null, files:[],groups:[],skipped:[],findings:[]};
  let next = offset;
  for (; next < total && next-offset < Math.min(limit,limits.page); next++) {
    let row = scope.rows[next];
    if (!row) {
      const current = await record(key(value.id, `finding/${value.findings[next-scope.rows.length]}`));
      if (!current.present || current.value?.kind !== 'finding') fail('Review finding is unavailable', 'internal');
      row = {kind:'finding',value:current.value.finding};
    }
    const target = {file:'files',group:'groups',skipped:'skipped',finding:'findings'}[row.kind];
    result[target].push(row.value);
    if (size(result) > limits.output) {
      result[target].pop();
      if (next === offset) fail('Review page item exceeds the bounded output budget');
      break;
    }
  }
  result.next = next < total ? next : null;
  return result;
}
export async function prepare(input, actor) {
  const args = fields(input, ['target','review_id','offset','limit']);
  const limit = integer(args.limit === undefined ? limits.page : args.limit, 'review page limit', 1, Number.MAX_SAFE_INTEGER);
  let owner;
  if (args.review_id !== undefined) {
    identifier(args.review_id);
    if (args.target !== undefined) fail('Existing reviews have an immutable target');
    if ((await record(key(args.review_id,'head'))).present) {
      const current = await head(args.review_id, actor);
      return await page(current.value,args.offset,limit);
    }
    if (args.offset !== undefined) fail('Capture continuation does not accept an offset');
    owner=await resume(args.review_id,actor);
  } else {
    if (args.offset !== undefined) fail('offset requires an existing review_id');
    const id = identifier(callId());
    if ((await record(key(id,'head'))).present) fail('Review scope already exists', 'conflict');
    await recover(actor);
    owner=await begin(id,actor);
    initialize(owner,args.target ?? {kind:'worktree',data:{scope:'all'}});
  }
  try {
    await flush(owner,true);
    const value=await advance(owner);
    if (!value) return pending(owner);
    const output=await page(value,0,limit);
    await finish(owner,[write(key(owner.id,'head'),value)]);
    return output;
  } catch (error) {
    if (error.retryableRead) {
      error.details={...error.details,review_id:owner.id};
      throw error;
    }
    if (error.details?.kind !== 'plugin_value_conflict') await discard(owner.id,actor);
    throw error;
  }
}

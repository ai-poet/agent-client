import {context,readTurn,listKeys,prepareTransaction,completeRequest,forgetRequest} from 'sailry/sdk';
import {clone,fail,limits,size} from './policy.js';
import {key,record,write} from './records.js';

const terminal = ['completed','continued','cancelled','interrupted','failed'];
const owned = (value,actor) => value?.v === 1 && value.kind === 'capture'
  && value.creator === actor.session && value.worktree === actor.worktree
  && value.package?.name === actor.package.name;

async function execute(operations) {
  const request = prepareTransaction(operations);
  const outcome = await completeRequest(request);
  if (outcome?.Err) {
    if (outcome.Err.code !== 'outcome_unknown') forgetRequest(request);
    fail(outcome.Err.message,outcome.Err.code,outcome.Err.details);
  }
  if (outcome?.Ok?.kind !== 'plugin_transaction' || outcome.Ok.data?.length !== operations.length) {
    fail('Review capture transaction is uncertain','outcome_unknown');
  }
  for (let index=0; index<operations.length; index++) {
    const operation=operations[index], output=outcome.Ok.data[index];
    if (output.kind !== 'plugin_value' || output.data.key !== operation.data.key
        || output.data.revision !== (BigInt(operation.data.expected_revision)+1n).toString()
        || output.data.present !== (operation.kind === 'write')) {
      fail('Review capture transaction is uncertain','outcome_unknown');
    }
  }
  forgetRequest(request);
}

function counts(value) {
  // These counters describe live keys in the existing 4096-key namespace quota.
  for (const count of [value.chunks,value.pages]) {
    if (!Number.isSafeInteger(count) || count < 0 || count > 4096) fail('Invalid capture manifest','internal');
  }
}

export async function discard(id,actor,fallback=null) {
  let current=await record(key(id,'capture'));
  if (!current.present && !fallback) return;
  let value=current.present ? current.value : fallback;
  if (!owned(value,actor)) fail('Capture belongs to another owner','permission_denied');
  if (value.id !== id || !['capturing','published','aborted'].includes(value.status)) {
    fail('Invalid capture manifest','internal');
  }
  if (value.status === 'published') return;
  counts(value);
  if (value.status !== 'aborted') {
    value={...value,status:'aborted'};
    // This fence competes atomically with every batch and the final publication.
    await execute([write(key(id,'capture'),value,current.revision)]);
    current=await record(key(id,'capture'));
  }
  for (const [suffix,count] of [['data',value.chunks],['scope',value.pages]]) {
    let operations=[];
    for (let index=0;index<count;index++) {
      const stored=await record(key(id,`${suffix}/${index}`));
      if (!stored.present) continue;
      if (stored.revision !== '1' || stored.value?.capture !== id
          || !['content','scope'].includes(stored.value.kind)) fail('Capture cleanup ownership changed','internal');
      operations.push({kind:'remove',data:{key:stored.key,expected_revision:stored.revision}});
      if (operations.length === 64) { await execute(operations); operations=[]; }
    }
    if (operations.length) await execute(operations);
  }
  await execute([{kind:'remove',data:{key:current.key,expected_revision:current.revision}}]);
}

export async function recover(actor) {
  let after=null;
  do {
    const page=await listKeys('review/',after,100);
    for (const name of page.keys) {
      if (!name.endsWith('/capture')) continue;
      const current=await record(name), value=current.value;
      if (!current.present || !owned(value,actor) || value.status === 'published') continue;
      const history=await readTurn(value.turn,null,1);
      if (history.run?.turn !== value.turn || history.run.session !== actor.session
          || history.run.worktree !== actor.worktree) fail('Capture history owner changed','internal');
      if (terminal.includes(history.run.status)) await discard(value.id,actor);
    }
    if (page.after !== null && (typeof page.after !== 'string' || page.after === after)) {
      fail('Invalid capture cleanup cursor','internal');
    }
    after=page.after;
  } while (after !== null);
}

export async function begin(id,actor) {
  const value={v:1,kind:'capture',id,creator:actor.session,worktree:actor.worktree,
    package:clone(actor.package),turn:context().turn,status:'capturing',chunks:0,pages:0};
  const owner={id,actor,value,revision:'0',pending:[]};
  try {
    await execute([write(key(id,'capture'),value)]);
    owner.revision='1';
    return owner;
  } catch (error) {
    await discard(id,actor,value);
    throw error;
  }
}

export async function resume(id,actor) {
  const current=await record(key(id,'capture'));
  if (!current.present) fail('Review does not exist','not_found');
  if (!owned(current.value,actor)) fail('Capture belongs to another owner','permission_denied');
  if (current.value.id !== id || current.value.status !== 'capturing') fail('Review capture is unavailable','conflict');
  counts(current.value);
  const value=clone(current.value);
  value.turn=context().turn;
  return {id,actor,value,revision:current.revision,pending:[]};
}

export async function flush(owner,force = false) {
  if (!owner.pending.length && !force) return;
  const value=clone(owner.value);
  await execute([write(key(owner.id,'capture'),value,owner.revision),...owner.pending]);
  owner.revision=(BigInt(owner.revision)+1n).toString();
  owner.pending=[];
}

export async function append(owner,suffix,value) {
  const counter=suffix === 'data' ? 'chunks' : 'pages';
  const index=owner.value[counter];
  const operation=write(key(owner.id,`${suffix}/${index}`),{...value,capture:owner.id});
  const next={...owner.value,[counter]:index+1};
  if (owner.pending.length && (owner.pending.length === 63
      || size([write(key(owner.id,'capture'),next,owner.revision),...owner.pending,operation]) > limits.transaction)) {
    await flush(owner);
  }
  owner.value[counter]++;
  owner.pending.push(operation);
  return index;
}

export function publish(owner) {
  if (owner.pending.length) fail('Capture has uncommitted resources','internal');
  return write(key(owner.id,'capture'),{...owner.value,status:'published'},owner.revision);
}

export async function finish(owner,operations) {
  await execute([publish(owner),...operations]);
}

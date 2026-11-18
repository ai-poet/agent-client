import {readTurnState,stageTurnState,newId,isValueConflict} from 'sailry/sdk';
import {readMemorySettings} from './settings.js';
import {readMemory,head,unchanged,stable,snapshotChanged} from './storage.js';
import {searchMemories} from './retrieval.js';
import {terms} from './tokenize.js';
import {near} from './review.js';
import {putOperations,removeOperations,mergeOperations,memoryOutput,RecordChanged,EquivalentMemory} from './mutations.js';
import {argumentsFor,requireRead,retrieve,fail} from './policy.js';

export async function enabled(write) {
  const state=readTurnState(), current=await readMemorySettings();
  if (!state.settings.enabled || !current.enabled || (write && (!state.settings.auto_write || !current.auto_write))) fail('memory access is disabled','permission_denied');
  return state;
}
function capture(prepared,entry,action,turn) {
  const id=entry.summary.id.toLowerCase();
  const changed=prepared.operations?.find(operation=>operation.data?.key===`memory/entry/${id}`);
  if (!changed) return null;
  // Display snapshots share the existing UTF-8 budget without granting read authority.
  const visible=retrieve([entry],{remaining:turn.settings.context_bytes})[0];
  const snapshot=visible ? JSON.parse(visible.parts[0].text) : null;
  return {id,revision:String(BigInt(changed.data.expected_revision)+1n),
    summary_revision:entry.summary.revision+1,action,
    title:entry.summary.title,project:entry.summary.project,kind:entry.summary.kind,
    body:snapshot?.body ?? '',body_truncated:snapshot?.body_truncated ?? true};
}
function receipt(output,snapshot) {
  if (!snapshot) return null;
  // Only the matching confirmed CAS outcome can expose the captured snapshot.
  const changed=output.data?.find(value=>value.kind==='plugin_value'
    && value.data?.key===`memory/entry/${snapshot.id}`
    && value.data.revision===snapshot.revision)?.data;
  if (!changed || changed.present!==(snapshot.action!=='deleted')) return null;
  if (changed.present) {
    const summary=changed.value?.summary;
    if (summary?.id!==snapshot.id || summary.revision!==snapshot.summary_revision
      || summary.title!==snapshot.title || summary.project!==snapshot.project
      || summary.kind!==snapshot.kind) return null;
  }
  const {action,title,project,kind,body,body_truncated}=snapshot;
  return {action,title,project,kind,body,body_truncated};
}
export function result({output},snapshot) {
  if (output.isError) return output;
  const outcome=memoryOutput(output);
  const value=outcome.kind==='memory_removed' ? {removed:outcome.data} : {updated:outcome.data};
  const confirmed=receipt(output,snapshot);
  return confirmed ? {...value,receipt:confirmed} : value;
}
async function latest(ids,turn) {
  const captured=await stable(async()=>{
    const current=await head(), entries=[], deleted=[];
    for (const id of new Set(ids)) {
      try { entries.push(await readMemory(id,turn.project)); }
      catch (error) { if (error.code!=='not_found') throw error; deleted.push(id); }
    }
    await unchanged(current.record.revision);
    return {entries,deleted};
  });
  const output={status:captured.entries.length ? 'changed' : 'deleted',saved:false,
    latest:retrieve(captured.entries,turn),deleted:captured.deleted};
  stageTurnState(turn);
  return output;
}
const finish=(value,failed)=>({result:failed ? {context:value} : value});
function writer(prepare) {
  return async ({arguments:input,step,state,outcome})=>{
    const retry=step>0 && isValueConflict(outcome);
    if (step>0 && !retry) return {result:result({output:outcome},state?.receipt),output:step-1};
    try {
      const turn=await enabled(true);
      if (step===8) return finish({status:'pending',saved:false},retry);
      try {
        const prepared=await prepare(input,turn,state);
        if (prepared.done) return finish(prepared.done,retry);
        return {call:{operation:'storage.transaction',arguments:prepared.arguments},state:prepared.state};
      } catch (error) {
        if (error instanceof RecordChanged || error instanceof EquivalentMemory) return finish(await latest(error.ids,turn),retry);
        throw error;
      }
    } catch (error) {
      if (snapshotChanged(error)) return finish({status:'pending',saved:false},retry);
      return {result:{isError:true,error:{code:error.code ?? 'internal',message:error.message}}};
    }
  };
}
// Creation checks the same scope first, so saving needs no prior search.
async function neighbors(entry,turn) {
  let captured=[];
  try { captured=await searchMemories(turn.project,Array.from(`${entry.summary.title}\n${entry.body}`).slice(0,1024).join(''),{keyword:true}); }
  catch (error) { if (snapshotChanged(error)) throw error; }
  const added=new Set(terms(entry.body));
  const candidates=captured.filter(current=>current.summary.project===entry.summary.project);
  const covers=(current,other)=>[...other].every(term=>current.has(term));
  const known=added.size ? candidates.filter(current=>covers(new Set(terms(current.body)),added)) : [];
  if (known.length) return {similar:known};
  // Overlapping words suggest related facts, but never authorize replacing one.
  const related=candidates.filter(current=>{
    const existing=new Set(terms(current.body));
    return (existing.size>0 && existing.size<added.size && existing.size*2>=added.size && covers(added,existing))
      || near(current.body,entry.body);
  });
  return related.length ? {similar:related} : {};
}
export const save=writer(async(input,turn,state)=>{
  const value=argumentsFor(input,['id','expected_revision','global','archived','kind','title','body'],['kind','title','body']);
  let project=value.global ? null : turn.project;
  if (value.id!=null) {
    if (!('expected_revision' in value)) fail('pass expected_revision from search_memory to update this memory');
    requireRead(turn,value.id,value.expected_revision);
    let current;
    try { current=await readMemory(value.id,turn.project); }
    catch (error) { if (error.code==='not_found') throw new RecordChanged([value.id]); throw error; }
    if (current.summary.revision!==value.expected_revision) throw new RecordChanged([value.id]);
    project=current.summary.project;
  } else if ((value.expected_revision ?? 0)!==0) fail('omit expected_revision to create a memory, or pass id to update one');
  if (project===null && ((!value.global && value.id==null) || !['user','feedback'].includes(value.kind))) fail('global memory is only for cross-project user preferences or feedback; save project facts without global, inside a project');
  const id=value.id ?? state?.id ?? newId(), revision=value.expected_revision ?? 0;
  const entry={summary:{id,project,title:value.title,kind:value.kind,revision,updated_at_ms:0,archived:value.archived ?? false},body:value.body};
  if (value.id==null && !state) {
    const found=await neighbors(entry,turn);
    if (found.similar) {
      const output={status:'similar',saved:false,existing:retrieve(found.similar,turn),
        hint:'Similar memories exist. Update one with its id and revision, or save a distinct fact'};
      stageTurnState(turn);
      return {done:output};
    }
  }
  const arguments_=await putOperations(entry,revision,turn.project);
  return {arguments:arguments_,state:{id,receipt:capture(arguments_,entry,revision===0 ? 'created' : 'updated',turn)}};
});
export const forget=writer(async(input,turn)=>{
  const value=argumentsFor(input,['id','expected_revision'],['id','expected_revision']);
  // Deletion retains its exact target, including if another writer changed its body.
  for (let attempt=0; ; attempt++) {
    try {
      const current=await readMemory(value.id,turn.project);
      const arguments_=await removeOperations(value.id,current.summary.revision,turn.project);
      return {arguments:arguments_,state:{receipt:capture(arguments_,current,'deleted',turn)}};
    } catch (error) {
      if (error.code==='not_found') return {done:{status:'deleted',saved:false,deleted:[value.id]}};
      if (!(error instanceof RecordChanged) || attempt===2) throw error;
    }
  }
});
export const consolidate=writer(async(input,turn)=>{
  const value=argumentsFor(input,['id','expected_revision','title','body','sources'],['id','expected_revision','title','body','sources']);
  if (!Array.isArray(value.sources) || value.sources.length<1) fail('invalid consolidation arguments');
  const expected=[{id:value.id,revision:value.expected_revision},...value.sources];
  for (const source of expected) {
    if (!source || Object.keys(source).some(key=>!['id','revision'].includes(key)) || typeof source.id!=='string' || !Number.isSafeInteger(source.revision) || source.revision<1) fail('invalid consolidation arguments');
    requireRead(turn,source.id,source.revision);
  }
  const current=[];
  for (const source of expected) {
    try { current.push(await readMemory(source.id,turn.project)); }
    catch (error) { if (error.code==='not_found') throw new RecordChanged(expected.map(value=>value.id)); throw error; }
  }
  if (current.some((entry,index)=>entry.summary.revision!==expected[index].revision || entry.summary.archived)) throw new RecordChanged(expected.map(value=>value.id));
  const entry=current[0]; entry.summary.title=value.title; entry.body=value.body;
  try {
    const arguments_=await mergeOperations(entry,value.sources,turn.project);
    return {arguments:arguments_,state:{receipt:capture(arguments_,entry,'merged',turn)}};
  } catch (error) { if (error instanceof RecordChanged) throw new RecordChanged(expected.map(value=>value.id)); throw error; }
});

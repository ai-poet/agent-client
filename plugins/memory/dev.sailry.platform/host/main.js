import {readTurnState,stageTurnState} from "sailry/sdk";
import {bytes,turnParts,values,reads,argumentsFor,retrieve} from "./policy.js";
import {readMemorySettings} from './settings.js';
import {searchMemories,reviewMemories} from './retrieval.js';
import {snapshotChanged} from './storage.js';
import {enabled} from './workflow.js';
export {save,forget,consolidate,result} from './workflow.js';

// Recall is best effort: a failed lookup adds no context instead of failing the turn.
// Keyword matching keeps unrelated entries out when semantic ranking is available.
async function relevant(project,message) {
  if (typeof message !== 'string' || !message.trim()) return [];
  try { return await searchMemories(project,message,{keyword:true}); }
  catch { return []; }
}

// The fixed policy is the manifest's instructions. A turn returns matched bodies and the save
// cue only; with neither it returns no parts, so memory adds nothing to the conversation.
export async function initialize({tools,project,resources,message,mode}) {
  const settings = await readMemorySettings();
  const state = {settings,project,mode,remaining:settings.context_bytes,reads:[]};
  let parts = [];
  if (settings.enabled && resources) {
    // Planning turns reject writes, so they never get the save cue.
    const write = settings.auto_write && mode !== 'plan';
    parts = turnParts({relevant:await relevant(project,message),message,settings,write});
    state.remaining = Math.max(0,state.remaining-parts.reduce((size,part)=>size+bytes(part),0));
  }
  stageTurnState(state);
  return {instruction:"",parts,tools:tools.filter(name => settings.enabled && (settings.auto_write || ["search_memory","review_memories"].includes(name)))};
}

export async function beforeModel({results}) {
  const state = readTurnState();
  state.reads = reads(results);
  stageTurnState(state);
  return null;
}

const protect = handler => async input => {
  try { return await handler(input); }
  catch (error) { return {error:{code:error.code ?? "internal",message:error.message},isError:true}; }
};

export const search = protect(async input => {
  const value = argumentsFor(input,["query"],["query"]);
  const state = await enabled(false);
  let captured;
  try { captured=await searchMemories(state.project,value.query); }
  catch (error) { if (!snapshotChanged(error)) throw error; return {status:'pending',budget_remaining_bytes:state.remaining,budget_limited:true}; }
  const entries = retrieve(captured,state);
  stageTurnState(state);
  return {entries,budget_remaining_bytes:state.remaining,budget_limited:true};
});
export const review = protect(async input => {
  argumentsFor(input,[],[]);
  const state = await enabled(false);
  let captured;
  try { captured=await reviewMemories(await readMemorySettings(),{project:state.project,all_projects:false}); }
  catch (error) { if (!snapshotChanged(error)) throw error; return {status:'pending'}; }
  const hints = values(captured,state);
  stageTurnState(state);
  return {candidates:hints};
});

// Dialog admission and retries follow the original create/rename/trash forms.
import {prepareFileAction, completeRequest, forgetRequest, faultCode} from 'sailry/sdk';
import {parent, join} from './explorer.js';

export function failure(kind,code) {
  const prefix = kind === 'rename' ? 'files_rename' : kind === 'trash' ? 'files_trash' : 'files_create';
  if (code === 'outcome_unknown') return `${prefix}_unknown`;
  if (code === 'permission_denied') return `${prefix}_denied`;
  if (code === 'conflict' || code === 'revision_conflict') return kind === 'create' ? 'files_entry_exists' : `${prefix}_conflict`;
  if (code === 'invalid_request' && kind === 'create') return 'files_name_invalid';
  return `${prefix}_failed`;
}

export class Mutation {
  constructor(kind,paths,directory = false) {
    this.kind = kind; this.paths = [...paths]; this.directory = directory;
    this.pending = null; this.busy = false; this.error = null; this.done = false;
    this.completed = []; this.closed = false;
    this.batch = null;
  }

  arguments(name) {
    if (this.kind === 'trash') return this.paths.map(path => ({kind:'trash',path}));
    if (!name || name.includes('/')) { this.error = 'files_name_required'; return null; }
    if (this.kind === 'rename') return [{kind:'rename',from:this.paths[0],to:join(parent(this.paths[0]),name)}];
    const path = join(this.paths[0] ?? '',name);
    return [this.directory ? {kind:'create_directory',path} : {kind:'write',path,text:'',revision:null}];
  }

  async submit(name,cx) {
    if (this.busy || this.closed || this.done) return;
    if (this.kind === 'trash') return this.trash(cx);
    const actions = this.pending ? this.pending.actions : this.arguments(name);
    if (!actions) { cx.notify(); return; }
    this.busy = true; this.error = null; cx.notify();
    try {
      for (let index = this.completed.length; index < actions.length; index++) {
        if (!this.pending) this.pending = {id:prepareFileAction(actions[index]),actions};
        const result = await completeRequest(this.pending.id);
        if (result.Err) {
          this.error = failure(this.kind,result.Err.code);
          if (result.Err.code !== 'outcome_unknown') {
            forgetRequest(this.pending.id); this.pending = null;
          }
          return;
        }
        this.completed.push(actions[index]);
        forgetRequest(this.pending.id); this.pending = null;
        if (this.closed) return;
      }
      this.done = true;
    } catch (error) { this.error = failure(this.kind,this.pending ? 'outcome_unknown' : faultCode(error.message ?? String(error))); }
    finally { this.busy = false; cx.notify(); }
  }

  async trash(cx) {
    if (!this.batch) {
      const actions = this.pending ? this.pending.actions : this.arguments('');
      this.batch = actions.map((action,index) => ({action,id:index === 0 ? this.pending?.id ?? null : null,done:false,error:null}));
    }
    this.busy = true; this.error = null; cx.notify();
    try {
      await Promise.all(this.batch.filter(entry => !entry.done).map(async entry => {
        entry.error = null;
        try {
          if (!entry.id) entry.id = prepareFileAction(entry.action);
          const result = await completeRequest(entry.id);
          if (result.Err) {
            entry.error = result.Err.code;
            if (entry.error === 'outcome_unknown') return;
          } else entry.done = true;
          forgetRequest(entry.id); entry.id = null;
        } catch (error) {
          entry.error = entry.id ? 'outcome_unknown' : faultCode(error.message ?? String(error));
        }
      }));
      this.completed = this.batch.filter(entry => entry.done).map(entry => entry.action);
      const unresolved = this.batch.find(entry => entry.id);
      this.pending = unresolved ? {id:unresolved.id,actions:this.batch.map(entry => entry.action)} : null;
      const error = this.batch.find(entry => entry.error === 'outcome_unknown') ?? this.batch.find(entry => entry.error);
      this.error = error ? failure(this.kind,error.error) : null;
      this.done = this.batch.every(entry => entry.done);
    } finally { this.busy = false; cx.notify(); }
  }
}

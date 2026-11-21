// Clipboard selection and transfer dialogs retain the original Files workflow.
import {captureClipboard, readClipboard, preparePaste, prepareMove, selectUpload, startDownload,
  readTransfers, nextTransferChange, transferAction} from 'sailry/file-transfers';
import {join, within} from './explorer.js';

export class Transfers {
  constructor(report,changed = () => {},text = {}) {
    this.report = report;
    this.text = text;
    this.changed = changed;
    this.state = readTransfers();
    this.clipboard = readClipboard();
    this.current = null; this.queue = null; this.busy = false; this.advancing = null; this.advancingQueue = null; this.capturing = null;
  }

  value() { return this.state.transfers.find(value => value.id === this.current); }

  accept(state,cx) {
    if (BigInt(state.cursor) < BigInt(this.state.cursor)) return;
    for(const value of state.transfers) {
      const previous=this.state.transfers.find(entry=>entry.id===value.id);
      if(previous?.stage===value.stage&&JSON.stringify(previous.error)===JSON.stringify(value.error))continue;
      if(['failed','uncertain','done','cancelled'].includes(value.stage)) {
        const key=status(value,this.text);if(key)this.report(key,['done','cancelled'].includes(value.stage)?'info':'error');
      }
    }
    this.state = state; this.changed(this.value()); cx?.notify();
  }

  refresh(cx) { this.accept(readTransfers(),cx); }

  observe(cx) {
    cx.spawn(async cx => {
      let cursor = this.state.cursor;
      try { while (true) {
        const state = await nextTransferChange(cursor); cursor = state.cursor;
        this.accept(state,cx);
        if (this.value()?.stage === 'done' && this.queue?.paths.length) {
          try { await this.next(cx); }
          catch (_) { this.report('files_paste_failed'); }
        }
      } } catch (_) { /* The core retains admitted transfers after this view closes. */ }
    });
  }

  async capture(paths,cut,cx) {
    const pending = captureClipboard(paths,cut);
    this.capturing = pending;
    try { this.clipboard = await pending; cx.notify(); }
    finally { if (this.capturing === pending) this.capturing = null; }
  }

  canPaste() { this.clipboard = readClipboard(); return !!this.clipboard?.available; }

  async paste(directory,cx) {
    if (this.capturing) await this.capturing;
    if (!this.canPaste()) { this.report('files_paste_unavailable'); return; }
    this.queue = {clipboard:this.clipboard.id,paths:[...this.clipboard.paths],directory,prepared:new Map(),started:new Set()};
    const destinations = [];
    for (const entry of this.queue.paths) {
      const destination = join(directory,entry.path.split('/').at(-1));
      if (destinations.every(path => !within(destination,path) && !within(path,destination))) this.prepare(this.queue,entry);
      destinations.push(destination);
    }
    await this.next(cx);
  }

  prepare(queue,entry) {
    if (!queue.prepared.has(entry.path)) {
      queue.prepared.set(entry.path,preparePaste(queue.clipboard,entry.path,join(queue.directory,entry.path.split('/').at(-1)))
        .then(value => ({value}),error => ({error})));
    }
    return queue.prepared.get(entry.path);
  }

  async next(cx) {
    const queue = this.queue, entry = queue?.paths[0];
    if (this.advancing && this.advancingQueue === queue) return this.advancing;
    if (!entry) return;
    const advancing = (async () => {
      const prepared = await this.prepare(queue,entry);
      if (Object.hasOwn(prepared,'error')) { queue.prepared.delete(entry.path); throw prepared.error; }
      if (this.queue !== queue) return;
      queue.paths.shift(); queue.prepared.delete(entry.path);
      this.current = prepared.value.id; this.refresh(cx);
    })();
    this.advancing = advancing; this.advancingQueue = queue;
    try { await advancing; }
    finally { if (this.advancing === advancing) { this.advancing = null; this.advancingQueue = null; } }
  }

  independentNext() {
    const queue = this.queue, entry = queue?.paths[0];
    if (!entry) return false;
    const destination = join(queue.directory,entry.path.split('/').at(-1));
    return this.state.transfers.every(value => !queue.started.has(value.id)
      || !['preparing','transferring','publishing','recycling'].includes(value.stage)
      || !within(destination,value.path) && !within(value.path,destination));
  }

  async upload(directory,cx) {
    const transfer = await selectUpload(directory);
    if (!transfer) return;
    this.queue = null; this.current = transfer.id; this.refresh(cx);
    if (transfer.can_start) await this.action('start',undefined,cx);
  }

  async move(source,path,cx) {
    const transfer = await prepareMove(source,path);
    this.queue = null; this.current = transfer.id; this.refresh(cx);
  }

  async download(path,cx) {
    const transfer = await startDownload(path);
    if (!transfer) return;
    this.queue = null; this.current = transfer.id; this.refresh(cx);
  }

  async action(kind,path,cx) {
    const id = this.current;
    if (!id || this.busy) return;
    this.busy = true; cx.notify();
    try {
      await transferAction(id,{kind,...(path === undefined ? {} : {path})});
      this.refresh(cx);
      if (this.current === id && this.queue?.paths.length) {
        const stage = this.value()?.stage;
        if (['start','replace'].includes(kind) && ['preparing','transferring','publishing','recycling','done'].includes(stage)) this.queue.started.add(id);
        if (stage === 'done' || this.queue.started.has(id) && this.independentNext()) await this.next(cx);
      }
    } finally { this.busy = false; cx.notify(); }
  }

  async close(cx) {
    const value = this.value();
    if (!value) return;
    this.queue = null;
    await transferAction(value.id,{kind:value.can_cancel ? 'cancel' : 'dismiss'});
    if (this.current === value.id) this.current = null;
    this.refresh(cx);
  }
}

export function status(value,text = {}) {
  const prefix = value.kind === 'upload' ? 'files_upload' : value.kind === 'download' ? 'files_download' : 'files_paste';
  if (value.kind === 'move' && value.stage === 'recycling') return 'files_move_recycling';
  if (value.kind === 'move' && value.published && ['failed','uncertain'].includes(value.stage)) return 'files_move_partial';
  if (value.error?.message && Object.hasOwn(text,value.error.message)) return value.error.message;
  if (value.stage === 'uncertain') return value.kind === 'move' ? 'files_move_unknown' : `${prefix}_unknown`;
  if (value.stage === 'exists') return value.kind === 'upload' ? 'files_upload_exists' : 'files_paste_conflict';
  if (value.stage === 'transferring') return value.kind === 'upload' ? 'files_upload_sending'
    : value.kind === 'download' ? 'files_download_receiving' : 'files_paste_copying';
  if (value.stage === 'ready') return null;
  if (value.stage === 'done' && ['copy','move'].includes(value.kind)) return null;
  if (value.stage === 'failed') {
    if (value.error?.code === 'busy') return 'files_operation_busy';
    if (['conflict','revision_conflict'].includes(value.error?.code)) return `${prefix}_conflict`;
    if (value.error?.code === 'permission_denied') return value.kind === 'download' ? 'files_download_destination' : `${prefix}_denied`;
  }
  return `${prefix}_${value.stage}`;
}

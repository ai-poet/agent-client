import {View} from 'gpui-kit';
import {context,releaseTerminal} from 'sailry';
import {listTerminals, listTerminalTools, prepareTerminal, prepareOpenTerminal, prepareCloseTerminal,
  completeRequest, forgetRequest, nextChange} from 'sailry/sdk';
import {nextMenuEvent, pane, openPane, updatePane, closePane, nextPaneEvent,toast} from 'sailry/ui';
import {messages} from './locales.js';
import {render} from './view.js';
import {title, directory} from './labels.js';
import {selection} from './access.js';
import * as exit from './exit.js';

export default class Commands extends View {
  init(_props, cx) {
    this.text = messages(JSON.parse(context()).locale);
    this.resource = pane()?.resource;
    this.items = []; this.tools = []; this.selected = this.resource?.id ?? null; this.reveal = null;
    this.loaded = false; this.loading = false; this.dirty = false; this.busy = false; this.pending = null; this.error = null;
    this.openAttempts = new Map(); this.openRetry = null;
    this.openIntent = null;
    this.visible = true; this.connected = true; this.lastRead = 0;
    this.toolsRevision = null; this.loadedToolsRevision = null;
    this.closingTerminals = new Set();
    this.ended = new Set(); this.dismissed = new Set();
    if (this.resource) cx.spawn(async cx => {
      try { while (true) {
        const event = await nextPaneEvent();
        if (event.kind === 'close') this.perform('close', this.resource.id, cx);
      } } catch (_) { /* View release ends pane observation. */ }
    });
    cx.spawn(async cx => {
      try { while (true) {
        const event = await nextMenuEvent();
        if (event.menu === 'terminal-tools') this.perform('create', event.id, cx);
        if (event.menu === 'terminal-access') this.perform('create', `shell:${event.id}`, cx);
      } } catch (_) { /* View release ends menu observation. */ }
    });
    cx.spawn(async cx => {
      let cursor = '', entry = '0';
      try { while (true) {
        const change = await nextChange(cursor,'package'); cursor = change.cursor;
        this.toolsRevision = change.terminal_tools;
        this.visible = change.visible !== false; this.connected = change.connected;
        if (change.entry !== entry) {
          entry = change.entry;
          if (entry !== '0') this.openIntent = {target:this.selected};
        }
        if (this.connected && this.visible) this.refresh(cx);
        else this.dirty = true;
      } } catch (_) { /* The captured view has been released. */ }
    });
  }
  refresh(cx, explicit = false) {
    if (!this.visible || !this.connected) { this.dirty = true; return; }
    if (this.loading) { this.dirty = true; return; }
    this.loading = true;
    cx.spawn(async cx => {
      try { do {
        const delay = 750 - (Date.now() - this.lastRead);
        if (!explicit && delay > 0) await cx.sleep(delay);
        if (!this.visible || !this.connected) { this.dirty = true; break; }
        this.lastRead = Date.now();
        this.dirty = false;
        const selected = this.selected;
        const toolsRevision = this.toolsRevision;
        const [items, tools] = await Promise.all([listTerminals(),
          this.loaded && this.loadedToolsRevision === toolsRevision ? this.tools : listTerminalTools()]);
        this.loadedToolsRevision = toolsRevision;
        this.items = items.filter(item => item.status.kind !== 'closed'); this.tools = tools; this.loaded = true;
        for (const id of this.closingTerminals) if (!this.items.some(item => item.id === id)) this.closingTerminals.delete(id);
        this.endExited();
        if (this.resource) {
          const index = this.items.findIndex(item => item.id === this.resource.id);
          if (index < 0) { closePane(); return; }
          updatePane({title:this.label(this.items[index], index)});
        }
        if (selected === this.selected && !this.items.some(item => item.id === selected))
          this.selected = this.items[0]?.id ?? null;
        if (this.reveal) {
          const index = this.items.findIndex(item => item.id === this.reveal);
          if (index >= 0 && openPane({resource:{kind:'terminal',id:this.reveal},title:this.label(this.items[index], index)}))
            this.reveal = null;
        }
        if (this.error === 'loadFailed') this.error = null;
        cx.notify();
      } while (this.dirty && this.visible && this.connected); }
      catch (_) { if (this.visible && (explicit || !this.loaded)) this.fail('loadFailed'); }
      finally { this.loading = false; this.openSelected(cx); this.closeEnded(cx); cx.notify(); }
    });
  }
  // A pane owns its own terminal; the page owns the inventory. Exit 0 is silent.
  endExited() {
    for (const item of this.items) {
      if ((this.resource && item.id !== this.resource.id) || item.ssh || this.ended.has(item.id)) continue;
      const ending = exit.action(item.status);
      if (!ending.close) continue;
      this.ended.add(item.id);
      if (ending.toast) toast({id:`terminal-exit-${item.id}`, message:exit.message(item.status, this.text), kind:'error'});
    }
  }
  closeEnded(cx) {
    if (this.busy || this.pending) return;
    const item = this.items.find(item => this.ended.has(item.id) && !this.dismissed.has(item.id)
      && !this.closingTerminals.has(item.id));
    if (!item) return;
    this.dismissed.add(item.id);
    // An automatic close must not clear another terminal's retry state.
    const {error, openRetry} = this;
    this.perform('close', item.id, cx);
    this.error = error; this.openRetry = openRetry;
  }
  fail(key) { if(this.error!==key)toast({id:'terminal-error',message:this.text[key],kind:'error'});this.error=key; }
  select(index, cx) {
    this.selected = this.items[index]?.id ?? null;
    this.openIntent = this.selected === null ? null : {target:this.selected};
    if (!this.pending && this.error === 'failed' && this.openRetry && this.selected !== this.openRetry) {
      this.error = null; this.openRetry = null;
    }
    this.openSelected(cx); cx.notify();
  }
  openSelected(cx) {
    if (!this.openIntent || this.loading || this.busy || this.pending) return;
    const intent = this.openIntent; this.openIntent = null;
    if (intent.target !== null && intent.target !== this.selected) return;
    const item = this.items.find(item => item.id === this.selected);
    if (item && this.closingTerminals.has(item.id)) return;
    if (!item || item.status.kind !== 'stopped' || item.tool || item.ssh) return;
    if (this.error) {
      const failed = this.openAttempts.get(this.openRetry);
      if (this.error !== 'failed' || !this.openRetry
        || (this.openRetry === item.id && failed?.revision === item.revision)) return;
      this.error = null; this.openRetry = null;
    }
    const attempted = this.openAttempts.get(item.id);
    if (attempted && attempted.revision === item.revision) {
      if (attempted.failed) { this.error = 'failed'; this.openRetry = item.id; cx.notify(); }
      return;
    }
    this.perform('open', item.id, cx);
  }
  openFailed(target) {
    this.openRetry = target;
    const attempted = this.openAttempts.get(target);
    if (attempted) attempted.failed = true;
  }
  retry(cx) {
    if (this.pending) this.perform(null, null, cx);
    else if (this.openRetry) this.perform('open', this.openRetry, cx);
    else this.refresh(cx,true);
  }
  perform(action, target, cx) {
    if (this.busy || (this.pending ? action !== null : action === null)) return;
    this.busy = true; this.error = null; this.openRetry = null; cx.notify();
    if (action === 'open') this.openAttempts.set(target, {
      revision:this.items.find(item => item.id === target)?.revision, failed:false,
    });
    if (this.resource) updatePane({busy:true});
    cx.spawn(async cx => {
      let closed = false;
      try {
        if (!this.pending) {
          const id = action === 'close' ? prepareCloseTerminal(target)
            : action === 'open' ? prepareOpenTerminal(target) : (() => {
              const chosen = target === null ? {tool:null,access:'unrestricted'} : selection(target);
              return prepareTerminal(chosen.tool, chosen.access);
            })();
          if (action === 'close') {
            try { releaseTerminal(target); }
            catch (error) { forgetRequest(id); throw error; }
            this.closingTerminals.add(target);
          }
          this.pending = {id, action, target};
        }
        const result = await completeRequest(this.pending.id);
        const done = this.pending.action, opened = this.pending.target;
        if (result.Err?.code === 'outcome_unknown') { this.fail('unknown'); return; }
        if (result.Err) {
          if (done === 'close') this.closingTerminals.delete(opened);
          // Another controller already removed the terminal.
          if (done !== 'close' || result.Err.code !== 'not_found') this.fail('failed');
          if (done === 'open') this.openFailed(opened);
        }
        else if (done !== 'close') {
          if (done !== 'open' || this.selected === opened) this.selected = result.Ok.data.id;
          if (!this.resource && done !== 'open') this.reveal = this.selected;
        }
        forgetRequest(this.pending.id); this.pending = null;
        if (!result.Err && done === 'close' && this.resource) { closed = true; return; }
        this.refresh(cx);
      } catch (_) {
        this.fail(this.pending ? 'unknown' : 'failed');
        if (!this.pending && action === 'open') this.openFailed(target);
      }
      finally {
        this.busy = false;
        try { if (this.resource) updatePane({busy:!!this.pending}); } catch (_) { /* The renderer may have been released. */ }
        this.openSelected(cx); if (!closed) this.closeEnded(cx); cx.notify();
        if (closed) closePane();
      }
    });
  }
  label(item, index) {
    return title(item.title) ?? this.text.tools[item.tool] ?? directory(item.directory)
      ?? `${this.text.terminal} ${index + 1}`;
  }
  render(cx) { return render(this); }
}

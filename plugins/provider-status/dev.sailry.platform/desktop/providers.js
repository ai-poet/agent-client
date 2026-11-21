import {observeVisibility} from './visibility.js';
import {publishContributions,requestProviderHttp} from 'sailry/sdk';
import {readProviderContexts,nextProviderContexts} from 'sailry/ui';
import * as claude from '../adapters/claude.js';
import * as codex from '../adapters/codex.js';
import {metric} from './format.js';

const adapters={chat_gpt:codex,chat_gpt_oauth:codex,claude_oauth:claude};
function measurement(response,provider,text) {
  const value=metric(adapters[provider.authentication].decode(response),text);
  return {...value,details:value.details.filter(row=>row.progress != null || row.label.label===text.credits)};
}

// One captured page host batches every declared provider; Node owns HTTP coalescing.
export default class Providers {
  constructor(cx,text) {
    this.text=text;this.entries=new Map();
    observeVisibility(this,cx);
    this.accept(readProviderContexts(),cx);
    cx.spawn(async cx=>{
      try {while(true) this.accept(await nextProviderContexts(this.state.cursor),cx);}
      catch (_) { /* Closing the page releases its context observation. */ }
    });

  }

  accept(state,cx) {
    const next=new Map();
    if(state.connected) for(const provider of state.providers) {
      const key=JSON.stringify(provider.scope);
      next.set(key,this.entries.get(key) ?? {provider,measurement:null,refreshAt:0});
    }
    this.entries=next;this.state=state;this.publish();this.wake();
  }

  publish() {
    for(const entry of this.entries.values()) {
      const measurement=entry.measurement;
      publishContributions([{id:'provider_usage',scope:entry.provider.scope,
        visible:measurement?.value != null,...(measurement ?? {value:null,details:[]})}]);
    }
  }

  refresh(cx,deferred=false) {
    if (!this.visible || !this.state.connected) return;
    if (!deferred) for (const entry of this.entries.values()) if (!entry.pending) entry.requested=true;
    const pending=[...this.entries.entries()].filter(([,entry])=>entry.requested&&!entry.pending&&Date.now()>=entry.refreshAt);
    for(const [,entry] of pending) {entry.pending=true;entry.requested=false;}
    const delayed=[...this.entries.values()].filter(entry=>entry.requested&&!entry.pending);
    if(delayed.length) this.deferRefresh(Math.min(...delayed.map(entry=>entry.refreshAt)));
    if(!pending.length) return;
    cx.spawn(async cx=>{
      await Promise.all(pending.map(async([key,entry])=>{
        try {
          if(this.entries.get(key)!==entry) return;
          try {
            const response=await requestProviderHttp(entry.provider.id,entry.provider.revision,'usage',{});
            if(this.entries.get(key)!==entry) return;
            entry.measurement=measurement(response,entry.provider,this.text);
            entry.refreshAt=Date.now()+180000;
          } catch (_) {
            if(this.entries.get(key)!==entry) return;
            entry.measurement=null;entry.refreshAt=Date.now()+60000;
          }
          this.publish();cx.notify();
        } finally {entry.pending=false;}
      }));
    });
  }
}

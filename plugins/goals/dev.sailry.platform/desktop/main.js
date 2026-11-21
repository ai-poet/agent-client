import {View} from 'gpui-kit';
import {context} from 'sailry';
import {nextChange,nextConversation,prepareRequest,completeRequest,forgetRequest,faultCode,publishContributions,nextContributionEvent,closeContribution} from 'sailry/sdk';
import {readComposer,nextComposerChange,toast} from 'sailry/ui';
import {messages} from './locales.js';
import {render} from './view.js';

export default class Goals extends View {
  init(_props,cx) {
    this.scope=JSON.parse(context()); this.text=messages(this.scope.locale);
    this.state=readComposer(); this.goal=null; this.history=[]; this.open=false;
    this.busy=false; this.loading=false; this.pending=null; this.readFailed=false; this.readError=null;
    this.packageCursor=null;this.runsCursor=null;this.readKey=null;
    this.publish(); this.refresh(cx);
    cx.spawn(async cx=>{
      try {while(true) {
        const previous=this.state;
        this.accept(await nextComposerChange(previous.cursor));
        if (this.state.session!==previous.session || this.state.connected!==previous.connected) {this.refresh(cx);this.observeConversation(cx);}
        this.publish(); cx.notify();
      }} catch (_) {}
    });
    cx.spawn(async cx=>{
      let cursor='';
      try {while(true) {const change=await nextChange(cursor,'package');cursor=change.cursor;this.packageCursor=change.revision;this.refresh(cx);this.observeConversation(cx);}}
      catch (_) {}
    });
    cx.spawn(async cx=>{
      try {while(true) {
        const event=await nextContributionEvent();
        if (event.id==='status') {
          this.open=event.kind==='invoke' ? true : event.value===true; this.refresh(cx);
        }
        this.publish(event);cx.notify();
      }} catch (_) {}
    });
    this.observeConversation(cx);
  }
  current() {return !!this.scope.session && this.scope.session===this.state.session && this.state.connected;}
  observeConversation(cx) {
    const active=()=>this.current() && this.goal?.status==='active';
    if (this.watching || !active()) return;
    this.watching=true;
    cx.spawn(async cx=>{
      let cursor='';
      try {while(active()) {
        const change=await nextConversation(cursor,'runs');cursor=change.cursor;
        if (!active()) break;
        const page=change.view.snapshot?.page;
        if (!page || page.session!==this.scope.session || page.session!==this.state.session) continue;
        const next=JSON.stringify([page.revision,page.runs.map(run=>[run.turn,run.sequence,run.status,run.error?.code])]);
        if (next!==this.runsCursor) {this.runsCursor=next;this.refresh(cx);}
      }} catch (_) {}
      finally {this.watching=false;}
    });
  }
  accept(state) {
    const left=state.cursor,right=this.state.cursor;
    if (left.length>right.length || left.length===right.length && left>=right) this.state=state;
  }
  enabled() {return this.state.connected && !this.state.busy && !this.state.readonly && !this.busy && !this.pending;}
  draft() {return this.state.command?.name==='goal' && this.state.command.package.name===this.scope.package.name ? this.state.command.arguments : null;}
  publish(event) {
    publishContributions([
      {id:'goal',enabled:this.enabled(),...(event?.id==='goal'?{reply_to:event.sequence}:{})},
    ]);
  }
  fail(key) {toast({id:'goal-error',kind:'error',message:this.text[key]});}
  refreshKey() {return JSON.stringify([this.state.session,this.state.connected,this.packageCursor,this.runsCursor]);}
  refresh(cx,force=false) {
    if (!this.current()) {this.goal=null;this.history=[];this.readKey=null;return;}
    if (force) this.readKey=null;
    if (this.readKey===this.refreshKey() && !this.readFailed) return;
    if (this.loading) return;
    this.loading=true;
    this.publish();cx.notify();
    cx.spawn(async cx=>{
      try {do {
        const key=this.refreshKey();
        this.readKey=key;
        const id=prepareRequest({kind:'read_plugin_callback',data:{handler:'inspect',input:{}}});
        let result;
        try {result=await completeRequest(id);}
        finally {forgetRequest(id);}
        if (result.Err) {const error=new Error(result.Err.message);error.code=result.Err.code;throw error;}
        if (!this.current()) {this.goal=null;this.history=[];break;}
        this.goal=result.Ok.data.goal;this.history=result.Ok.data.history ?? [];
        this.readFailed=false;
        this.readError=null;
      } while(this.current() && this.readKey!==this.refreshKey());}
      catch (error) {
        this.readError={code:error.code ?? faultCode(error.message ?? String(error)),message:error.message ?? String(error)};
        if (!this.readFailed) this.fail('readFailed');this.readFailed=true;
      }
      finally {this.loading=false;this.observeConversation(cx);this.publish();cx.notify();}
    });
  }
  perform(kind,cx) {
    if (this.busy || this.loading || this.pending || !this.state.connected || this.state.readonly || !this.goal) return;
    this.pending={id:prepareRequest({kind:'call_plugin',data:{handler:'control',input:{kind,goal_id:this.goal.id}}}),kind};
    this.execute(cx);
  }
  execute(cx) {
    if (this.busy || !this.pending) return;
    this.busy=true;this.publish();cx.notify();
    cx.spawn(async cx=>{
      const pending=this.pending;
      try {
        const result=await completeRequest(pending.id);
        if (result.Err) {const error=new Error(result.Err.message);error.code=result.Err.code;throw error;}
        forgetRequest(pending.id);this.pending=null;
        if (pending.kind==='end') {this.open=false;closeContribution('status');}
        toast({id:'goal-action',kind:'info',message:this.text[{pause:'paused',resume:'resumed',end:'ended'}[pending.kind]]});
      } catch(error) {
        const code=error.code ?? faultCode(error.message ?? String(error));
        const uncertain=['outcome_unknown','unavailable','busy'].includes(code);
        this.fail(uncertain?'uncertain':'error');
        if (!uncertain) {forgetRequest(pending.id);this.pending=null;}
      } finally {this.busy=false;this.refresh(cx,true);this.publish();cx.notify();}
    });
  }
  render() {return render(this);}
}

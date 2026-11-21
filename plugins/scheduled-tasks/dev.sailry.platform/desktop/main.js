import {View} from "gpui-kit";
import {context} from "sailry";
import {context as sdkContext, prepareHostCall, completeRequest, forgetRequest, nextChange, readResourceCatalog, listModels, listTeams, resolveSessionModel} from "sailry/sdk";
import {modal_closed,nextControlEvent,openSession,showWorkspaceDetails,toast,writeClipboard} from "sailry/ui";
import {list, history, states} from "../host/management.js";
import {messages} from "./locales.js";
import {editor, release, value, effortKey, setTiming, lead} from "./editor.js";
import {render} from "./view.js";
import {sameNode, hostCall, snapshot} from "../host/records.js";

const uncertain = code => ["unavailable", "busy", "outcome_unknown", "internal", "cancelled"].includes(code);
function unwrap(result) { if (result.Err) throw result.Err; return result.Ok; }
export default class ScheduledTasks extends View {
  init(_props, cx) {
    this.text = messages(JSON.parse(context()).locale);
    this.items = []; this.history = []; this.pages = 1; this.more = false; this.projectFilter = "all";
    this.panel = null; this.historyTask = null; this.historyLoaded = false;
    this.catalog = {projects:[],worktrees:[]}; this.loading = false; this.loaded = false; this.dirty = false;
    this.executionNode = sdkContext().execution_node; this.hosts = [];
    this.models = []; this.modelsError = false;
    this.teams = []; this.teamsError = false;
    this.busy = false; this.pending = null; this.error = null;
    this.editing = null; this.deleting = null; this.dialogId = 0;
    cx.spawn(async cx => {
      let seen = "";
      try { while (true) {
        const change = await nextChange(seen); seen = change.cursor;
        if (change.connected) this.refresh(cx);
      } } catch (_) { /* View release ends observation. */ }
    });
    cx.spawn(async cx => {
      try { while (true) this.control(await nextControlEvent(),cx); }
      catch (_) { /* View release ends its native controls. */ }
    });
    cx.spawn(async cx => {
      try { while (true) {
        if (await modal_closed() === `task-dialog-${this.dialogId}`) this.close(cx);
      } } catch (_) { /* View release ends observation. */ }
    });
  }
  refresh(cx) {
    if (this.loading) { this.dirty = true; return; }
    this.loading = true; cx.notify();
    cx.spawn(async cx => {
      try { do {
        this.dirty = false;
        // Records belong to the task open in the details panel only.
        const task = this.panel?.task ?? null,pages=this.pages;
        const [items,records,status,directory,models,teams] = await Promise.all([
          (async()=>{const items=[];let after=null;do {
            const page=unwrap(await list({after}));items.push(...page.items);after=page.after;
          }while(after!==null);return items;})(),
          (async()=>{const runs=[];let before=null;if(task!==null)for(let index=0;index<pages;index++){
            const selected=this.items.find(item=>item.id===task);
            const input={before,handler:task};
            const page=selected?.execution_node ? await hostCall(selected.execution_node,"history",input,true)
              : unwrap(await history(input));
            runs.push(...page.items);before=page.next_before;
            if(before===null)break;
          }return {runs,before};})().catch(()=>({runs:[],before:null,unavailable:true})),
          states(),readResourceCatalog(),
          listModels().then(catalog=>catalog.models.filter(model=>model.kind==='provider')).catch(()=>null),
          listTeams().catch(()=>null),
        ]);
        this.hosts = directory.sources;
        this.catalog = {projects:directory.sources.flatMap(source=>source.projects.map(project=>({...project,node:source.node}))),
          worktrees:directory.sources.flatMap(source=>source.worktrees.map(tree=>({...tree,node:source.node})))};
        this.items = items.map(item => ({...item,status:status[item.id]?.status ?? null,started_ms:status[item.id]?.started_ms ?? null}));
        const remoteNodes = new Map(this.items.filter(item => item.execution_node && !sameNode(item.execution_node,this.executionNode))
          .map(item=>[JSON.stringify(item.execution_node),item.execution_node]));
        const projections = await Promise.all([...remoteNodes.values()].map(async target=>{
          try{return {target,value:await hostCall(target,"runtime",{},true)};}
          catch(_){return {target,value:null};}
        }));
        this.items = this.items.map(item=>{
          const remote=projections.find(value=>sameNode(value.target,item.execution_node));
          if(!remote)return item;
          return {...item,next_ms:remote.value?.schedules.find(schedule=>schedule.id===item.id)?.next_ms ?? null,
            status:remote.value?.status[item.id]?.status ?? null,started_ms:remote.value?.status[item.id]?.started_ms ?? null,
            unavailable:item.unavailable || remote.value===null};
        });
        if (this.panel && !this.items.some(item => item.id === this.panel.task)) this.panel = null;
        if (task === (this.panel?.task ?? null)) {
          this.history = records.runs; this.historyTask = task; this.historyLoaded = task !== null && !records.unavailable; this.more = records.before !== null;
          if(records.unavailable)this.report("executionUnavailable");
        }
        this.loaded = true;
        if (this.error === "loadFailed") this.error = null;
        this.modelsError = models === null;
        if(models===null)this.report("modelsFailed");else this.models=models;
        this.teamsError = teams === null;
        const remoteEditing=this.editing?.draft.execution_node && !sameNode(this.editing.draft.execution_node,this.executionNode);
        if(teams===null)this.report("teamsFailed");else if(!remoteEditing) { this.teams=teams; this.syncLead(cx); }
        if(remoteEditing) this.loadOptions(cx);
        cx.notify();
      } while (this.dirty); }
      catch (_) { this.report("loadFailed"); }
      finally { this.loading = false; cx.notify(); }
    });
  }
  projectName(id) {
    return id ? this.catalog.projects.find(project => project.id === id)?.name ?? this.text.unavailable : this.text.noProject;
  }
  control(event,cx) {
    if (this.panelControl(event,cx)) return;
    const item = this.items.find(item => ["enabled","run","details","records","edit","delete"].some(action => event.id === `task-${action}-${item.id}`));
    if (item) {
      if (event.id === `task-details-${item.id}`) return this.inspect(item,"details",cx);
      if (event.id === `task-records-${item.id}`) return this.inspect(item,"records",cx);
      if (this.busy || this.pending || item.unavailable) return;
      if (event.id === `task-enabled-${item.id}` && typeof event.value === "boolean") this.perform("save", {...item,enabled:event.value},cx);
      else if (event.id === `task-run-${item.id}`) this.perform("run", {id:item.id},cx);
      else if (event.id === `task-edit-${item.id}`) this.edit(item,cx);
      else if (event.id === `task-delete-${item.id}`) { this.deleting = item; this.error = null; this.dialogId++; cx.notify(); }
      return;
    }
    const editing = this.editing;
    if (!editing || this.busy || this.pending) return;
    if (event.id === "task-timing" && ["once","every","weekly"].includes(event.value)) setTiming(editing,event.value,this.text);
    else if (/^task-weekday-[1-7]$/.test(event.id) && typeof event.value === "boolean" && editing.kind === "weekly") {
      const day = Number(event.id.slice(-1));
      editing.weekdays = editing.weekdays.filter(value => value !== day);
      if (event.value) editing.weekdays.push(day);
      editing.weekdays.sort((left,right) => left-right);
    }
    else if (event.id === "task-host" && !editing.draft.id) {
      const selected=this.hosts.find(source=>JSON.stringify(source.node)===event.value);
      if(!selected || selected.fault)return;
      editing.draft.execution_node=selected.node;
      editing.draft.project=null;editing.draft.worktree=null;editing.draft.config=null;
      editing.draft.team=null;editing.draft.models={};editing.model=null;
      this.loadOptions(cx);
    }
    else if (event.id === "task-project") {
      const project = event.value === "none" ? null : this.catalog.projects.find(project => project.id === event.value &&
        sameNode(project.node,editing.draft.execution_node))?.id;
      if (project === undefined) return;
      editing.draft.project = project; editing.draft.worktree = null;
    } else if (event.id === "task-worktree") {
      const worktree = event.value === "main" ? null : this.catalog.worktrees
        .find(tree => tree.id === event.value && tree.project === editing.draft.project && sameNode(tree.node,editing.draft.execution_node))?.id;
      if (worktree === undefined || !editing.draft.project) return;
      editing.draft.worktree = worktree;
    } else if (event.id === "task-model") {
      if (!this.modelsError && !lead(this)) return this.selectModel(event.value?.model,event.value?.effort,cx);
    }
    else if (event.id === "task-execution") {
      if (!["model","team"].includes(event.value) || event.value === editing.draft.execution) return;
      editing.draft.execution = event.value; editing.draft.team = null; editing.draft.models = {};
      this.syncLead(cx);
    } else if (event.id === "task-team") {
      if (editing.draft.execution !== "team") return;
      const team = event.value === "default" ? null : this.teams.find(team => team.id === event.value);
      if (team === undefined || (team?.id ?? null) === (editing.draft.team?.id ?? null)) return;
      editing.draft.team = team === null ? null : {id:team.id,revision:null}; editing.draft.models = {};
      this.syncLead(cx);
    }
    else return;
    cx.notify();
  }
  // Returns true when the event belongs to the list filter or the details panel.
  panelControl(event,cx) {
    if (event.id === "tasks-project-filter") {
      if (event.value === "all") this.projectFilter = "all";
      else if (event.value === "none") this.projectFilter = null;
      else if (this.catalog.projects.some(project => project.id === event.value)
          || this.items.some(item => item.project === event.value)) this.projectFilter = event.value;
      cx.notify(); return true;
    }
    const panel = this.panel, task = panel && this.items.find(item => item.id === panel.task);
    if (!task) return false;
    if (event.id === "task-panel-tabs") {
      if (["details","records"].includes(event.value)) { panel.tab = event.value; cx.notify(); }
    } else if (event.id === "task-panel-close") { this.panel = null; cx.notify(); }
    else if (event.id === "task-records-more") {
      if (event.value === "load" && this.more && !this.loading) { this.pages++; this.refresh(cx); }
    } else if (event.id === "task-prompt-copy") {
      writeClipboard(task.prompt); toast({id:"scheduled-tasks-copy",message:this.text.copied,kind:"info"});
    } else {
      const run = this.history.find(run => event.id === `task-open-${run.job.id}` || event.id === `task-stop-${run.job.id}`);
      if (!run) return false;
      if (event.id === `task-open-${run.job.id}`) { if (run.session) openSession(run.session,run.execution_node ?? task.execution_node); }
      else if (!this.busy && !this.pending && (run.job.status === "queued" || (run.job.status === "running" && run.turn))) {
        this.perform(run.job.status === "queued" ? "cancel" : "stop",
          {id:run.job.id,execution_node:run.execution_node ?? task.execution_node},cx);
      }
    }
    return true;
  }
  inspect(item,tab,cx) {
    if (this.panel?.task !== item.id) {
      this.history = []; this.historyTask = null; this.historyLoaded = false; this.pages = 1; this.more = false;
      this.panel = {task:item.id,tab}; this.refresh(cx);
    } else this.panel.tab = tab;
    showWorkspaceDetails(); cx.notify();
  }
  selectModel(model,effort,cx) {
    const editing = this.editing;
    if (!editing || this.busy || this.pending) return;
    const fixed = lead(this);
    if (fixed ? model !== `${fixed.provider}/${fixed.model}` || effortKey(effort ?? "default") !== effortKey(fixed.effort ?? "default")
      : !this.models.some(choice => choice.id === model)) return;
    const version = ++editing.modelVersion;
    editing.modelLoading = true; editing.leadError = false; cx.notify();
    cx.spawn(async cx => {
      try {
        const resolve = (model,effort,config) => sameNode(editing.draft.execution_node,this.executionNode)
          ? resolveSessionModel(model,effort,config)
          : hostCall(editing.draft.execution_node,"model",{model,effort,config},true);
        const defaults = fixed && fixed.effort == null ? await resolve(model,null,null) : null;
        if (this.editing !== editing || editing.modelVersion !== version) return;
        const config = defaults && !editing.draft.config ? defaults
          : await resolve(model,defaults?.effort ?? effort,editing.draft.config);
        if (this.editing !== editing || editing.modelVersion !== version) return;
        editing.draft.config = config; editing.model = `${config.provider}/${config.model}`;
      } catch (_) {
        if (this.editing !== editing || editing.modelVersion !== version) return;
        if (fixed) { editing.leadKey = null; editing.leadError = true; }
        this.report("modelFailed");
      } finally {
        if (this.editing === editing && editing.modelVersion === version) { editing.modelLoading = false; cx.notify(); }
      }
    });
  }
  loadOptions(cx) {
    const editing=this.editing;if(!editing)return;
    const target=editing.draft.execution_node,version=++editing.modelVersion;
    editing.modelLoading=true;this.modelsError=true;this.teamsError=true;cx.notify();
    cx.spawn(async cx=>{
      try {
        const options=sameNode(target,this.executionNode)
          ? await Promise.all([listModels(),listTeams()]).then(([models,teams])=>({models:models.models.filter(model=>model.kind==="provider"),teams}))
          : await hostCall(target,"options",{},true);
        if(this.editing!==editing || editing.modelVersion!==version)return;
        this.models=options.models;this.teams=options.teams;this.modelsError=false;this.teamsError=false;
        editing.modelLoading=false;this.syncLead(cx);
      } catch (_) {
        if(this.editing===editing && editing.modelVersion===version) {
          this.models=[];this.teams=[];this.modelsError=true;this.teamsError=true;this.report("executionUnavailable");
        }
      } finally {if(this.editing===editing && editing.modelVersion===version){editing.modelLoading=false;cx.notify();}}
    });
  }
  syncLead(cx) {
    const editing = this.editing;
    if (!editing || this.busy || this.pending) return;
    const fixed = lead(this);
    const key = fixed ? JSON.stringify([editing.draft.team.id,fixed.provider,fixed.model,fixed.effort ?? "default"]) : null;
    if (key === (editing.leadKey ?? null) && (!editing.leadError || fixed)) return;
    editing.leadKey = key; editing.leadError = false;
    editing.modelVersion++; editing.modelLoading = false;
    if (fixed) this.selectModel(`${fixed.provider}/${fixed.model}`,fixed.effort ?? null,cx);
  }
  report(key) { this.error=key;toast({id:'scheduled-tasks-error',message:this.text[key],kind:'error'}); }
  edit(item, cx) {
    if (this.busy || this.pending) return;
    this.close(cx); this.editing = editor(item, this.text); this.dialogId++; this.syncLead(cx); cx.notify();
    this.editing.draft.execution_node = item?.execution_node ?? this.executionNode;
    this.loadOptions(cx);
  }
  close(cx) {
    if (this.busy) return;
    if (this.pending) { forgetRequest(this.pending); this.pending = null; this.refresh(cx); }
    if (this.editing) release(this.editing);
    this.editing = null; this.deleting = null; this.error = null; cx.notify();
  }
  save(cx) {
    if (this.pending) return this.perform(null, null, cx);
    if (lead(this) && (this.editing.modelLoading || this.editing.leadError)) return;
    try {
      const editing = this.editing, input = value(editing);
      editing.modelVersion++; editing.modelLoading = false;
      this.perform("save", input, cx);
    }
    catch (error) { this.report(["required", "modelRequired", "invalidTime", "invalidInterval", "invalidClock", "timezoneRequired", "weekdaysRequired"].includes(error.message) ? error.message : "failed"); cx.notify(); }
  }
  perform(handler, input, cx) {
    if (this.busy) return;
    if (!this.pending && input?.id && ["save", "remove", "run"].includes(handler)) {
      const previous = handler === "save" && this.editing?.draft.id === input.id && this.editing.original ? this.editing.original
        : handler === "remove" ? this.deleting : this.items.find(item => item.id === input.id);
      if (previous) {
        const expected = snapshot(previous);
        input = {...input, revision:input.revision ?? previous.revision, expected_value:expected};
      }
    }
    this.busy = true; this.error = null; cx.notify();
    cx.spawn(async cx => {
      try {
        if (!this.pending) {
          const item=input?.id && this.items.find(item=>item.id===input.id);
          const target=input?.expected_value?.execution_node ?? input?.execution_node ?? item?.execution_node ?? this.executionNode;
          if(item?.unavailable){this.error="executionUnavailable";return;}
          if(this.hosts.find(source=>sameNode(source.node,target))?.fault){this.error="executionUnavailable";return;}
          this.pending = prepareHostCall(target,handler,input);
        }
        const receipt = await completeRequest(this.pending);
        const fault = receipt.Err ?? receipt.Ok?.data?.Err;
        if (fault && uncertain(fault.code)) {
          this.error = fault.code==="unavailable"?"executionUnavailable":"unknown";
          this.refresh(cx); return;
        }
        if (fault) this.error = fault.code === "revision_conflict" ? "conflict"
          : fault.code === "not_configured" ? "notConfigured" : fault.code==="unavailable"?"executionUnavailable":"failed";
        forgetRequest(this.pending); this.pending = null; this.busy = false;
        if (!this.error) this.close(cx);
        this.refresh(cx);
      } catch (_) { this.error = this.pending ? "unknown" : "failed"; }
      finally { this.busy = false;if(this.error)this.report(this.error);cx.notify(); }
    });
  }
  render(cx) { return render(this,cx); }
}

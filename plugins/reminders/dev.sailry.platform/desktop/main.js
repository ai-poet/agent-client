import { View } from "gpui-kit";
import { context } from "sailry";
import { context as sdkContext, prepareHostCall, completeRequest, forgetRequest, nextChange, readResourceCatalog } from "sailry/sdk";
import { modal_closed,nextControlEvent,showWorkspaceDetails,toast } from "sailry/ui";
import { messages } from "./locales.js";
import { editor, release, value } from "./editor.js";
import { render } from "./view.js";
import {page, sameNode, snapshot} from "../host/records.js";
import {history} from "../host/schedules.js";

const uncertain = code => ["unavailable", "busy", "outcome_unknown", "internal", "cancelled"].includes(code);
function unwrap(result) { if (result.Err) throw result.Err; return result.Ok; }
export default class Reminders extends View {
  init(_props, cx) {
    this.text = messages(JSON.parse(context()).locale);
    this.items = []; this.showCompleted = true; this.loading = false; this.loaded = false; this.dirty = false;
    this.catalog = {projects:[],worktrees:[]}; this.projectFilter = "all";
    this.executionNode = sdkContext().execution_node; this.hosts = [];
    this.panel = null; this.history = []; this.historyId = null; this.historyLoaded = false;
    this.busy = false; this.pending = null; this.error = null;
    this.editing = null; this.deleting = null; this.dialogId = 0;
    cx.spawn(async cx => {
      let seen = "";
      try {
        while (true) {
          const change = await nextChange(seen);
          seen = change.cursor;
          if (change.connected) this.refresh(cx);
        }
      } catch (_) { /* View release ends observation. */ }
    });
    cx.spawn(async cx => {
      try { while (true) {
        const id = await modal_closed();
        if (id === `reminder-dialog-${this.dialogId}`) this.close(cx);
      } } catch (_) { /* View release ends observation. */ }
    });
    cx.spawn(async cx => {
      try { while (true) this.control(await nextControlEvent(), cx); }
      catch (_) { /* View release ends its native controls. */ }
    });
  }
  refresh(cx) {
    if (this.loading) { this.dirty = true; return; }
    this.loading = true; cx.notify();
    cx.spawn(async cx => {
      try {
        do {
          this.dirty = false;
          // Deliveries belong to the reminder open in the details panel only.
          const reminder = this.panel?.reminder ?? null;
          const [items,directory] = await Promise.all([
            (async()=>{const items=[];let after=null;do {
              const batch=await page({after});items.push(...batch.items);after=batch.after;
            }while(after!==null);return items;})(),
            readResourceCatalog(),
          ]);
          this.hosts = directory.sources;
          this.catalog = {projects:directory.sources.flatMap(source => source.projects.map(project => ({...project,node:source.node}))),
            worktrees:directory.sources.flatMap(source => source.worktrees)};
          this.items = items; this.loaded = true;
          const selected = items.find(item => item.id === reminder);
          let jobs = [], historyFailure = false;
          if (selected) {
            try {
              if (selected.execution_node == null) jobs = unwrap(await history({handler:reminder})).items;
              else {
                const id = prepareHostCall(selected.execution_node, "history", {handler:reminder}, true);
                try { jobs = unwrap(unwrap(await completeRequest(id)).data).items; }
                finally { forgetRequest(id); }
              }
            } catch (_) { historyFailure = true; this.report("executionUnavailable"); }
          }
          if (this.panel && !items.some(item => item.id === this.panel.reminder)) this.panel = null;
          if (reminder === (this.panel?.reminder ?? null)) {
            this.history = jobs; this.historyId = reminder; this.historyLoaded = reminder !== null && !historyFailure;
          }
          if (this.error === "loadFailed") this.error = null;
          cx.notify();
        } while (this.dirty);
      } catch (_) { this.report("loadFailed"); }
      finally { this.loading = false; cx.notify(); }
    });
  }
  projectName(id) {
    return id === null ? this.text.noProject : this.catalog.projects.find(project => project.id === id)?.name ?? this.text.unavailableProject;
  }
  control(event, cx) {
    if (this.panel && this.panelControl(event, cx)) return;
    const inspected = this.items.find(item => event.id === `reminder-details-${item.id}` || event.id === `reminder-records-${item.id}`);
    if (inspected) return this.inspect(inspected, event.id.startsWith("reminder-details-") ? "details" : "records", cx);
    const item = this.items.find(item => event.id === `reminder-complete-${item.id}`);
    if (item) {
      if (!this.busy && !this.pending && !item.unavailable && typeof event.value === "boolean") this.perform("save", {...item, completed:event.value}, cx);
      return;
    }
    const editing = this.items.find(item => event.id === `reminder-edit-${item.id}`);
    if (editing) { if (!editing.unavailable) this.edit(editing, cx); return; }
    const deleting = this.items.find(item => event.id === `reminder-delete-${item.id}`);
    if (deleting) {
      if (!this.busy && !this.pending && !deleting.unavailable) {
        this.deleting = deleting; this.error = null; this.dialogId++; cx.notify();
      }
      return;
    }
    if (event.id === "reminders-project-filter") {
      if (event.value === "all") this.projectFilter = "all";
      else if (event.value === "none") this.projectFilter = null;
      else if (this.catalog.projects.some(project => project.id === event.value)
          || this.items.some(item => item.project === event.value)) this.projectFilter = event.value;
      else return;
    } else if (event.id === "reminder-host" && this.editing && !this.editing.draft.id && !this.busy && !this.pending) {
      const host = this.hosts.find(source => JSON.stringify(source.node) === event.value);
      if (!host || host.fault) return;
      this.editing.draft.execution_node = host.node;
      if (!host.projects.some(project => project.id === this.editing.draft.project)) this.editing.draft.project = null;
    } else if (event.id === "reminder-project" && this.editing && !this.busy && !this.pending) {
      if (event.value === "none") this.editing.draft.project = null;
      else if (this.catalog.projects.some(project => project.id === event.value &&
          sameNode(project.node, this.editing.draft.execution_node))) this.editing.draft.project = event.value;
      else return;
    } else return;
    cx.notify();
  }
  // Returns true when the event belongs to the open details panel.
  panelControl(event, cx) {
    if (event.id === "reminder-panel-tabs") {
      if (["details","records"].includes(event.value)) { this.panel.tab = event.value; cx.notify(); }
    } else if (event.id === "reminder-panel-close") { this.panel = null; cx.notify(); }
    else return false;
    return true;
  }
  inspect(item, tab, cx) {
    if (this.panel?.reminder !== item.id) {
      this.history = []; this.historyId = null; this.historyLoaded = false;
      this.panel = {reminder:item.id, tab}; this.refresh(cx);
    } else this.panel.tab = tab;
    showWorkspaceDetails(); cx.notify();
  }
  report(key) { this.error=key;toast({id:'reminders-error',message:this.text[key],kind:'error'}); }
  edit(item, cx) {
    if (this.busy || this.pending) return;
    this.close(cx); this.editing = editor(item, this.text); this.dialogId++; cx.notify();
    if (!item) this.editing.draft.execution_node = this.executionNode;
  }
  close(cx) {
    if (this.busy) return;
    // Closing an uncertain draft releases only its UI handle, never its Node work.
    if (this.pending) { forgetRequest(this.pending); this.pending = null; this.refresh(cx); }
    if (this.editing) release(this.editing);
    this.editing = null; this.deleting = null; this.error = null; cx.notify();
  }
  save(cx) {
    if (this.pending) return this.perform(null, null, cx);
    try { this.perform("save", value(this.editing), cx); }
    catch (error) { this.report(["required", "invalidTime"].includes(error.message) ? error.message : "failed"); cx.notify(); }
  }
  perform(handler, input, cx) {
    if (this.busy) return;
    if (!this.pending && input?.id) {
      const previous = input.expected_value ?? (handler === "remove" ? this.deleting : this.items.find(item => item.id === input.id));
      if (previous) input = {...input, expected_value:snapshot(previous)};
    }
    this.busy = true; this.error = null; cx.notify();
    cx.spawn(async cx => {
      try {
        if (!this.pending) {
          const item = input?.id && this.items.find(item => item.id === input.id);
          const target = input?.expected_value?.execution_node ?? input?.execution_node ?? item?.execution_node ?? this.executionNode;
          if (item?.unavailable) { this.error = "executionUnavailable"; return; }
          if (target == null) { this.error = "executionUnknown"; return; }
          if (this.hosts.find(source => sameNode(source.node, target))?.fault) {
            this.error = "executionUnavailable"; return;
          }
          this.pending = prepareHostCall(target, handler, input);
        }
        const receipt = await completeRequest(this.pending);
        const fault = receipt.Err ?? receipt.Ok?.data?.Err;
        if (fault && uncertain(fault.code)) {
          this.error = fault.code === "unavailable" ? "executionUnavailable" : "unknown";
          this.refresh(cx); return;
        }
        if (fault) this.error = fault.code === "revision_conflict" ? "conflict"
          : fault.code === "not_found" ? "projectUnavailable" : "failed";
        forgetRequest(this.pending); this.pending = null;
        this.busy = false;
        if (!this.error) this.close(cx);
        this.refresh(cx);
      } catch (_) { this.error = this.pending ? "unknown" : "failed"; }
      finally { this.busy = false;if(this.error)this.report(this.error);cx.notify(); }
    });
  }
  render(cx) { return render(this, cx); }
}

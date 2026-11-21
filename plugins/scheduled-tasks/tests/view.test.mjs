import test from "node:test";
import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";
import vm from "node:vm";
import {messages} from "../dev.sailry.platform/desktop/locales.js";

const plain = value => JSON.parse(JSON.stringify(value));
const read = async name => (await readFile(new URL(`../dev.sailry.platform/desktop/${name}`, import.meta.url), "utf8"))
  .replace(/^import[\s\S]*?;\n/gm, "").replace(/^export default /gm, "").replace(/^export /gm, "");
const source = [await read("schedule.js"), await read("details.js"), await read("view.js")].join("\n");
const controller = await read("main.js");
const schedule = vm.runInNewContext(`${await read("schedule.js")}\n({upcoming,streak,ribbon,weeklyOffset,clock,dayLabel,when,relative,interval})`);
const effortKey = value => typeof value === "string" ? value : String(value.budget);
const effortLabel = vm.runInNewContext(`${await read("details.js")}\neffortLabel`, {effortKey});
const task = (id, extra = {}) => ({id, name:`Task ${id}`, prompt:"Keep this task", revision:"2", project:"project", worktree:null,
  queue:"default", enabled:true, next_ms:null, status:null, started_ms:null,
  config:{provider:"provider", model:"explicit", effort:"high"},
  timing:{kind:"weekly", data:{timezone:"Asia/Shanghai", weekdays:[1,3], hour:9, minute:0}}, ...extra});
const run = (id, handler, status, extra = {}) => ({name:`Task ${handler}`, session:null, turn:null,
  job:{id, handler, status, created_ms:Date.now() - 60000, started_ms:Date.now() - 60000, ...extra}});

test("model effort labels preserve raw identifiers and budgets", () => {
  for (const [effort, label] of [["high","high"], ["xhigh","xhigh"], [{budget:2048},"2048"],
    ["default",""], [null,""], [undefined,""]]) {
    assert.equal(effortLabel(effort),label);
  }
});

function fixture() {
  const nodes = new Map(), calls = [], cx = {notify() {}, theme:() => ({typography:{mono:"mono"}})};
  function element(kind = "div", id = null, props = {}) {
    const value = {kind, selector:id, props, items:[], style:{}};
    const proxy = new Proxy(value, {get(target, key) {
      if (key === "id") return id => { target.selector = id; nodes.set(id, proxy); return proxy; };
      if (key === "child") return value => { target.items.push(value); return proxy; };
      if (key === "children") return values => { target.items.push(...values); return proxy; };
      if (key in target) return target[key];
      if (key === "on_click" || key === "on_change") return callback => { target[key] = callback; return proxy; };
      return (...args) => { target.style[key] = args; return proxy; };
    }});
    if (id) nodes.set(id, proxy);
    return proxy;
  }
  const constructors = Object.fromEntries(["Button", "Icon", "Tag"].map(kind => [kind, function(id) { return element(kind, id); }]));
  const adapters = Object.fromEntries(["CardColumn", "CardList", "CardRow", "CardSummary", "ClampedText", "EmptyState", "IconButton",
    "LoadMore", "Loading", "Modal", "PanelHeader", "SegmentedTabs", "SelectField", "SettingsPage", "Toggle", "Tooltip", "Workspace"]
    .map(kind => [kind, {new:(id, props) => element(kind, id, props)}]));
  const globals = {
    div:() => element(), ...constructors, ...adapters, effortKey,
    theme:() => ({colors:{foreground:"#f0f0f0ff", muted_foreground:"#909090ff", muted:"#202020ff", background:"#101010ff",
      border:"#303030ff", ring:"#4080f0ff", primary:"#4080f0ff", primary_foreground:"#ffffffff", destructive:"#f07050ff",
      success:"#50d080ff", info:"#4080f0ff", warning:"#e0a050ff"}}),
    editor:() => [element("editor"), element("footer")],
  };
  const {render} = vm.runInNewContext(`${source}\n({render})`, globals);
  const Controller = vm.runInNewContext(`${controller}\nScheduledTasks`, {View:class {}, lead:() => null,
    openSession:session => calls.push(["open", session]), showWorkspaceDetails:() => calls.push(["show"]),
    writeClipboard:text => calls.push(["copy", text]), toast:notice => calls.push(["toast", notice.message])});
  const view = Object.assign(Object.create(Controller.prototype), {text:messages("en"), items:[], history:[], pages:1, more:false,
    loading:false, loaded:true, busy:false, pending:null, error:null, editing:null, deleting:null, dialogId:0, projectFilter:"all",
    panel:null, historyTask:null, historyLoaded:false, teams:[], catalog:{projects:[{id:"project", name:"Project"}, {id:"other", name:"Other"}], worktrees:[]},
    refresh:() => calls.push(["refresh"]), edit:item => calls.push(["edit", item]),
    perform:(handler, input) => calls.push([handler, input]), close:() => calls.push(["close"])});
  return {view, nodes, calls, cx, render() {
    nodes.clear(); const body = render(view, cx);
    for (const list of [...nodes.values()].filter(node => node.kind === "CardList" && !node.items.length)) {
      const {loaded, empty_icon, empty_label} = list.props;
      list.child(loaded === false ? element("Loading", `${list.selector}-loading`, {})
        : element("EmptyState", `empty-${list.selector}-empty`, {variant:"list", icon:empty_icon, label:empty_label}));
    }
    return body;
  }, open(id, tab) { view.control({id:`task-${tab}-${id}`}, cx); view.historyTask = id; view.historyLoaded = true; }};
}
function texts(node) {
  if (typeof node === "string") return [node];
  return node?.items ? node.items.flatMap(texts) : [];
}

test("page shows the project filter and task list without a tab bar", () => {
  const setup = fixture();
  setup.render();
  const {nodes} = setup, body = nodes.get("tasks");
  assert.equal(body.kind, "SettingsPage");
  assert.deepEqual(plain(body.props), {title:"Scheduled tasks", description:"Run tasks on a schedule"});
  assert.equal([...nodes.values()].some(node => ["TabBar", "Tab"].includes(node.kind)), false);
  const toolbar = nodes.get("tasks-controls");
  assert.deepEqual(toolbar.items.map(child => child.items[0]?.kind ?? child.kind), ["SelectField", "Button"]);
  assert.equal(toolbar.items[1].selector, "task-create");
  const filter = nodes.get("tasks-project-filter");
  assert.equal(filter.props.label, "Project");
  assert.equal(filter.props.selected, "all");
  assert.deepEqual(plain(filter.props.items.map(item => item.id)), ["all", "none", "project", "other"]);
  assert.deepEqual(plain(nodes.get("tasks-items").props), {loaded:true, empty_icon:"calendar", empty_label:"No tasks"});
  assert.equal(nodes.get("empty-tasks-items-empty").kind, "EmptyState");
  assert.equal(nodes.get("tasks-workspace").props.has_details, false);
  assert.equal(nodes.has("task-panel"), false);
});

test("project filter narrows cards and ignores unknown projects", () => {
  const setup = fixture();
  setup.view.items = [task("a"), task("b", {project:null}), task("c", {project:"other"})];
  const visible = () => { setup.render(); return ["a", "b", "c"].filter(id => setup.nodes.has(`task-card-${id}`)); };
  assert.deepEqual(visible(), ["a", "b", "c"]);
  setup.view.control({id:"tasks-project-filter", value:"none"}, setup.cx);
  assert.deepEqual(visible(), ["b"]);
  setup.view.control({id:"tasks-project-filter", value:"other"}, setup.cx);
  assert.deepEqual(visible(), ["c"]);
  setup.view.control({id:"tasks-project-filter", value:"removed"}, setup.cx);
  assert.equal(setup.view.projectFilter, "other");
  setup.view.control({id:"tasks-project-filter", value:"all"}, setup.cx);
  assert.deepEqual(visible(), ["a", "b", "c"]);
});

test("unconfirmed reads reserve card rows and pending actions remain recoverable", () => {
  const setup = fixture(); setup.view.loaded = false;
  for (const loading of [false, true]) {
    setup.view.loading = loading; setup.render();
    assert.equal(setup.nodes.has("empty-tasks-items-empty"), false);
    assert.equal(setup.nodes.get("tasks-items-loading").kind, "Loading");
  }
  setup.view.error = "unknown"; setup.view.pending = "original-request"; setup.render();
  assert.ok(setup.nodes.has("task-retry"));
  assert.ok(setup.nodes.has("task-dismiss"));
  setup.view.busy = true; setup.render();
  assert.equal(setup.nodes.has("task-retry"), false);
  assert.deepEqual(setup.calls, []);
});

test("refresh keeps existing cards and the open editor", () => {
  const setup = fixture();
  setup.view.items = [task("saved", {enabled:false})]; setup.view.editing = {draft:{name:"Retained draft"}};
  for (const error of [null, "loadFailed"]) {
    setup.view.loading = error === null; setup.view.error = error; setup.render();
    assert.ok(setup.nodes.has("task-card-saved"));
    assert.deepEqual(setup.nodes.get("task-dialog-0").items.map(item => item.kind), ["editor", "footer"]);
  }
});

test("task cards place time and status tag before the toggle and actions", () => {
  const setup = fixture(), item = task("row", {name:"Review 中文 🙂", prompt:"Keep this task in the editor"});
  setup.view.items = [item]; setup.render();
  const row = setup.nodes.get("task-card-row");
  assert.deepEqual(plain(row.props), {row_id:"task-row-row"});
  assert.deepEqual(row.items.map(child => child.selector), ["task-summary-row", "task-status-row", "task-leading-row", "task-actions-row"]);
  assert.deepEqual(row.items.slice(1).map(child => child.props.variant), ["wide_metadata", "control", "actions"]);
  assert.deepEqual(plain(row.items[0].props), {title:item.name, subtitle:"Project", icon:"icons/folder.svg"});
  assert.deepEqual(plain(setup.nodes.get("task-enabled-row").props), {label:"Enabled: Review 中文 🙂", checked:true, disabled:false});
  const actions = setup.nodes.get("task-actions-row");
  assert.deepEqual(actions.items.map(child => child.selector), ["task-run-row", "task-details-row", "task-records-row", "task-edit-row", "task-delete-row"]);
  assert.ok(actions.items.every(child => child.kind === "IconButton"));
  assert.deepEqual(actions.items.map(child => child.props.label), ["Run now", "Details", "Records", "Edit", "Delete"]);
  assert.equal(JSON.stringify(plain(row.items)).includes(item.prompt), false);
  setup.view.control({id:"task-edit-row"}, setup.cx);
  assert.equal(setup.calls[0][1].prompt, item.prompt);
  setup.view.control({id:"task-delete-row"}, setup.cx);
  assert.equal(setup.view.deleting, item);
  setup.render();
  setup.nodes.get("task-delete-confirm").on_click({}, setup.cx);
  assert.deepEqual(plain(setup.calls[1]), ["remove", {id:"row", revision:"2"}]);
});

test("status tags map native state to tone beside a muted time", () => {
  const setup = fixture(), next = Date.now() + 3 * 3600000;
  const cases = [
    [{next_ms:next}, "Pending", "secondary", true],
    [{status:"failed", next_ms:next}, "Failed", "danger", true],
    [{status:"running", started_ms:Date.now() - 5 * 60000}, "Running", "info", "Running 5 min"],
    [{status:"queued"}, "Queued", "info", false],
    [{enabled:false, next_ms:next}, "Paused", "secondary", false],
    [{status:"completed"}, "Completed", "success", false],
  ];
  for (const [extra, label, variant, time] of cases) {
    setup.view.items = [task("row", extra)]; setup.render();
    const tag = setup.nodes.get("task-state-row").items[0];
    assert.equal(tag.kind, "Tag");
    assert.deepEqual(texts(tag), [label]);
    assert.deepEqual(tag.style.variant, [variant]);
    assert.deepEqual(tag.style.size, ["medium"]);
    assert.equal("outline" in tag.style, label === "Paused");
    assert.equal(setup.nodes.has("task-time-row"), !!time);
    if (typeof time === "string") assert.deepEqual(texts(setup.nodes.get("task-time-row")), [time]);
  }
});

test("native row controls reject stale identities, invalid values and pending mutations", () => {
  const setup = fixture(); setup.view.items = [task("row")];
  setup.view.control({id:"task-enabled-row", value:"false"}, setup.cx);
  setup.view.control({id:"task-run-removed"}, setup.cx);
  for (const state of [{busy:true, pending:null}, {busy:false, pending:"original-request"}]) {
    Object.assign(setup.view, state); setup.render();
    for (const action of ["enabled", "run", "edit", "delete"]) {
      assert.equal(setup.nodes.get(`task-${action}-row`).props.disabled, true);
      setup.view.control({id:`task-${action}-row`, value:false}, setup.cx);
    }
  }
  assert.deepEqual(setup.calls, []); assert.equal(setup.view.deleting, null);
});

test("details and records buttons open the side panel on that tab", () => {
  const setup = fixture(); setup.view.items = [task("a"), task("b")];
  setup.view.control({id:"task-details-a"}, setup.cx);
  assert.deepEqual(plain(setup.view.panel), {task:"a", tab:"details"});
  assert.deepEqual(setup.calls, [["refresh"], ["show"]]);
  setup.view.historyTask = "a"; setup.view.historyLoaded = true; setup.render();
  const workspace = setup.nodes.get("tasks-workspace");
  assert.deepEqual(plain(workspace.props), {has_details:true, details_label:"Details", default_details_width:380, min_details_width:320, navigation:"none"});
  assert.deepEqual(workspace.items.map(item => item.selector), ["tasks", "task-panel"]);
  assert.equal(setup.nodes.get("task-panel-tabs").props.selected, "details");
  assert.deepEqual(plain(setup.nodes.get("task-panel-tabs").props.items), [{id:"details", label:"Details"}, {id:"records", label:"Records 0"}]);
  assert.equal(setup.nodes.get("task-details-a").props.selected, true);
  assert.equal(setup.nodes.get("task-records-a").props.selected, false);
  assert.deepEqual(setup.nodes.get("task-card-a").style.border_color, ["#4080f0ff"]);
  assert.deepEqual(setup.nodes.get("task-card-b").style.border_color, ["#303030ff"]);
  assert.ok(setup.nodes.has("task-overview")); assert.equal(setup.nodes.has("task-records"), false);
  setup.view.control({id:"task-panel-tabs", value:"records"}, setup.cx); setup.render();
  assert.equal(setup.view.panel.tab, "records");
  assert.ok(setup.nodes.has("task-records")); assert.equal(setup.nodes.has("task-overview"), false);
  setup.view.control({id:"task-panel-tabs", value:"other"}, setup.cx);
  assert.equal(setup.view.panel.tab, "records");
  setup.view.control({id:"task-details-a"}, setup.cx);
  assert.equal(setup.view.panel.tab, "details");
  assert.equal(setup.calls.filter(call => call[0] === "refresh").length, 1);
  setup.view.control({id:"task-records-b"}, setup.cx);
  assert.deepEqual(plain(setup.view.panel), {task:"b", tab:"records"});
  assert.equal(setup.view.historyLoaded, false);
  setup.render();
  assert.ok(setup.nodes.has("task-records-loading"));
  setup.view.control({id:"task-panel-close"}, setup.cx); setup.render();
  assert.equal(setup.view.panel, null);
  assert.equal(setup.nodes.get("tasks-workspace").props.has_details, false);
});

test("records show only this task's runs with session and stop routes", () => {
  const setup = fixture(); setup.view.items = [task("a"), task("b")];
  setup.open("a", "records");
  setup.view.history = [
    run("queued", "a", "queued"), run("running", "a", "running"),
    run("other", "b", "completed"),
    run("failed", "a", "failed", {created_ms:Date.now() - 86400000, started_ms:Date.now() - 86400000, error:{message:"Provider unavailable"}}),
  ];
  setup.view.history[1].turn = "turn"; setup.view.history[1].session = "running-session";
  setup.view.history[3].session = "failed-session";
  setup.render();
  const rail = setup.nodes.get("task-records");
  assert.deepEqual(rail.items.map(item => item.selector), ["task-day-0", "task-run-queued", "task-run-running", "task-day-2", "task-run-failed"]);
  assert.deepEqual(texts(setup.nodes.get("task-day-0")), ["Today"]);
  assert.deepEqual(texts(setup.nodes.get("task-day-2")), ["Yesterday"]);
  assert.equal(setup.nodes.has("task-run-other"), false);
  assert.deepEqual(plain(setup.nodes.get("task-panel-tabs").props.items[1]), {id:"records", label:"Records 3"});
  assert.deepEqual(texts(setup.nodes.get("task-run-error-failed")), ["Provider unavailable"]);
  assert.ok(texts(setup.nodes.get("task-run-card-failed")).includes("Failed"));
  assert.equal(setup.nodes.get("task-stop-queued").kind, "IconButton");
  assert.equal(setup.nodes.has("task-open-queued"), false);
  assert.equal(setup.nodes.has("task-stop-failed"), false);
  for (const id of ["task-open-failed", "task-stop-queued", "task-stop-running", "task-open-running", "task-stop-other"]) {
    setup.view.control({id}, setup.cx);
  }
  assert.deepEqual(plain(setup.calls.slice(2)), [["open", "failed-session"], ["cancel", {id:"queued"}], ["stop", {id:"running"}], ["open", "running-session"]]);
  setup.view.pending = "original-request"; setup.view.control({id:"task-stop-queued"}, setup.cx);
  assert.equal(setup.calls.length, 6);
});

test("records load the next page when the sentinel becomes visible", () => {
  const setup = fixture(); setup.view.items = [task("a")];
  setup.open("a", "records"); setup.view.history = [run("done", "a", "completed")];
  setup.render();
  assert.equal(setup.nodes.has("task-records-more"), false);
  setup.view.more = true; setup.render();
  assert.deepEqual(plain(setup.nodes.get("task-records-more").props), {loading:false, label:"Loading"});
  assert.deepEqual(plain(setup.nodes.get("task-panel-tabs").props.items[1]), {id:"records", label:"Records 1+"});
  setup.calls.length = 0;
  setup.view.control({id:"task-records-more", value:"load"}, setup.cx);
  assert.equal(setup.view.pages, 2); assert.deepEqual(setup.calls, [["refresh"]]);
  setup.view.loading = true; setup.render();
  assert.equal(setup.nodes.get("task-records-more").props.loading, true);
  setup.view.control({id:"task-records-more", value:"load"}, setup.cx);
  setup.view.loading = false; setup.view.more = false;
  setup.view.control({id:"task-records-more", value:"load"}, setup.cx);
  setup.view.more = true;
  setup.view.control({id:"task-records-more", value:"other"}, setup.cx);
  assert.equal(setup.view.pages, 2); assert.equal(setup.calls.length, 1);
});

test("details show the failure streak, facts, prompt copy and recent runs", () => {
  const setup = fixture(), item = task("a", {status:"failed", next_ms:Date.UTC(2030, 0, 7, 1)});
  setup.view.items = [item];
  setup.open("a", "details");
  setup.view.history = [
    run("f1", "a", "failed", {error:{message:"Timed out"}}), run("f2", "a", "failed"), run("ok", "a", "completed"),
    run("stop", "a", "cancelled"), run("other", "b", "failed"),
  ];
  setup.view.history[0].session = "failed-session";
  setup.render();
  const failure = texts(setup.nodes.get("task-failure"));
  assert.ok(failure.includes("Last run failed")); assert.ok(failure.includes("Timed out"));
  assert.ok(failure.some(line => line.endsWith("· 2 failures in a row")));
  assert.equal(setup.nodes.has("task-failure-open"), false);
  setup.view.control({id:"task-failure-open"}, setup.cx);
  assert.equal(setup.calls.some(call => call[0] === "open"), false);
  assert.deepEqual(texts(setup.nodes.get("task-plan-hero")), ["09:00"]);
  assert.equal(setup.nodes.get("task-plan-days").items.length, 7);
  assert.deepEqual(texts(setup.nodes.get("task-plan-timezone")), ["Timezone", "Asia/Shanghai"]);
  assert.deepEqual(texts(setup.nodes.get("task-fact-model")), ["Model", "explicit high"]);
  assert.equal(setup.nodes.get("task-panel-project").items[0].kind, "Icon");
  assert.deepEqual(texts(setup.nodes.get("task-panel-project")), ["Project"]);
  assert.deepEqual(texts(setup.nodes.get("task-fact-worktree")), ["Worktree", "Main worktree"]);
  assert.deepEqual(texts(setup.nodes.get("task-fact-status")), ["Status", "Enabled"]);
  assert.ok(texts(setup.nodes.get("task-fact-last")).includes("Failed"));
  assert.deepEqual(plain(setup.nodes.get("task-prompt-text").props), {text:"Keep this task", lines:5});
  setup.view.control({id:"task-prompt-copy"}, setup.cx);
  assert.deepEqual(plain(setup.calls.slice(-2)), [["copy", "Keep this task"], ["toast", "Copied"]]);
  assert.deepEqual(setup.nodes.get("task-recent-strip").items.map(item => item.selector),
    ["task-recent-stop", "task-recent-ok", "task-recent-f2", "task-recent-f1"]);
  assert.ok(texts(setup.nodes.get("task-recent")).includes("1 succeeded · 2 failed · 1 stopped"));
  assert.equal(setup.nodes.get("task-upcoming").items.length, 6);
  setup.view.history = [run("ok", "a", "completed"), run("f1", "a", "failed")];
  setup.render();
  assert.equal(setup.nodes.has("task-failure"), false);
});

test("paused and interval tasks show their own schedule details", () => {
  const setup = fixture(), anchor = Date.now() - 3600000;
  setup.view.items = [task("a", {enabled:false, timing:{kind:"every", data:{anchor_ms:anchor, interval_ms:7200000}}})];
  setup.open("a", "details"); setup.render();
  assert.deepEqual(texts(setup.nodes.get("task-plan-hero")), ["Every 2 h"]);
  assert.ok(setup.nodes.has("task-plan-ribbon")); assert.ok(setup.nodes.has("task-plan-now"));
  assert.deepEqual(texts(setup.nodes.get("task-plan-next")), ["Next", "Paused"]);
  assert.deepEqual(texts(setup.nodes.get("task-upcoming-paused")), ["Runs resume when enabled"]);
  assert.deepEqual(texts(setup.nodes.get("task-fact-status")), ["Status", "Paused"]);
});

test("schedules compute the next runs for each timing kind", () => {
  const hour = 3600000, day = 24 * hour;
  // Asia/Shanghai Monday and Wednesday 09:00 is 01:00 UTC on the same days.
  const monday = Date.UTC(2026, 9, 19, 1);
  const weekly = {enabled:true, next_ms:monday, timing:{kind:"weekly", data:{timezone:"Asia/Shanghai", weekdays:[1,3], hour:9, minute:0}}};
  assert.deepEqual(plain(schedule.upcoming(weekly, monday - hour)), [monday, monday + 2 * day, monday + 7 * day, monday + 9 * day, monday + 14 * day]);
  // Los Angeles Friday 20:00 PDT is Saturday 03:00 UTC; local weekdays follow the task zone.
  const friday = Date.UTC(2026, 9, 24, 3);
  const western = {enabled:true, next_ms:friday, timing:{kind:"weekly", data:{timezone:"America/Los_Angeles", weekdays:[5,6], hour:20, minute:0}}};
  assert.equal(schedule.weeklyOffset(western.timing.data, friday), -420);
  assert.deepEqual(plain(schedule.upcoming(western, friday - hour, 3)), [friday, friday + day, friday + 7 * day]);
  const now = Date.UTC(2026, 9, 18, 12), anchor = now - 90 * 60000;
  const every = {enabled:true, next_ms:null, timing:{kind:"every", data:{anchor_ms:anchor, interval_ms:hour}}};
  assert.deepEqual(plain(schedule.upcoming(every, now, 3)), [anchor + 2 * hour, anchor + 3 * hour, anchor + 4 * hour]);
  assert.deepEqual(plain(schedule.upcoming({...every, next_ms:now + 5 * 60000}, now, 2)), [now + 5 * 60000, now + 65 * 60000]);
  const once = {enabled:true, next_ms:null, timing:{kind:"once", data:{at_ms:now + hour}}};
  assert.deepEqual(plain(schedule.upcoming(once, now)), [now + hour]);
  assert.deepEqual(plain(schedule.upcoming(once, now + 2 * hour)), []);
  for (const item of [weekly, every, once]) assert.deepEqual(plain(schedule.upcoming({...item, enabled:false}, now)), []);
  assert.deepEqual(plain(schedule.upcoming({...weekly, next_ms:null}, now)), []);
  assert.equal(schedule.ribbon({anchor_ms:anchor, interval_ms:1000}, now).ticks, null);
  assert.equal(schedule.ribbon({anchor_ms:anchor - 10 * day, interval_ms:2 * hour}, now).ticks.length, 12);
  assert.ok(schedule.ribbon({anchor_ms:anchor, interval_ms:2 * hour}, now).ticks.every(at => at >= 0 && at < 1));
});

test("consecutive failures count only the newest failed runs", () => {
  const runs = statuses => statuses.map(status => ({job:{status}}));
  assert.equal(schedule.streak(runs(["failed", "failed", "completed", "failed"])), 2);
  assert.equal(schedule.streak(runs(["running", "failed"])), 0);
  assert.equal(schedule.streak(runs([])), 0);
});

test("a data refresh loads only the open task's records", async () => {
  const requests = [];
  const ScheduledTasks = vm.runInNewContext(`${controller}\nScheduledTasks`, {
    View:class {}, lead:() => null, context:() => JSON.stringify({locale:"en"}),sdkContext:() => ({execution_node:[1,...Array(31).fill(0)]}), messages,
    list:async() => ({Ok:{items:[{id:"fresh"}], after:null}}),
    history:async input => { requests.push(input); return {Ok:{items:[{job:{id:"run", handler:"fresh"}}], next_before:null}}; },
    states:async() => ({fresh:{status:"running", started_ms:5}}),
    readResourceCatalog:async() => ({sources:[{node:[1,...Array(31).fill(0)],projects:[],worktrees:[]}]}),
    listModels:async() => ({models:[]}), listTeams:async() => [], showWorkspaceDetails() {},
  });
  const view = new ScheduledTasks(), jobs = [], cx = {notify() {}, spawn:job => jobs.push(job)};
  view.init(null, cx); jobs.length = 0;
  view.refresh(cx); await jobs.shift()(cx);
  assert.deepEqual(requests, []);
  assert.equal(view.items[0].status, "running"); assert.equal(view.items[0].started_ms, 5);
  view.control({id:"task-records-fresh"}, cx); await jobs.shift()(cx);
  assert.deepEqual(plain(requests), [{before:null, handler:"fresh"}]);
  assert.equal(view.historyTask, "fresh"); assert.equal(view.historyLoaded, true);
  assert.equal(view.history[0].job.id, "run"); assert.equal(view.panel.tab, "records");
});

test("manifest enables the details column and localized panel copy", async () => {
  const manifest = JSON.parse(await readFile(new URL("../plugin.json", import.meta.url), "utf8"));
  const extension = manifest.extensions["dev.sailry.platform"];
  assert.equal(manifest.version, "0.1.23");
  assert.equal(extension.min_sailry_version, "0.2.0-alpha.2");
  assert.equal(extension.desktop.navigation_options.details, true);
  for (const file of ["details.js", "schedule.js"]) assert.ok(extension.desktop.resources.includes(`dev.sailry.platform/desktop/${file}`));
  assert.deepEqual(extension.actions, ["storage.read", "storage.write", "dispatch.manage", "sessions.start", "projects.read", "models.read", "teams.read", "conversation.read"]);
  for (const name of ["project", "worktree", "config", "model"]) assert.equal(name in extension.tools[0].handler.parameters.properties, false);
  for (const key of ["tasks", "runs", "more"]) assert.equal(messages("en")[key], undefined);
  assert.equal(messages("zh-CN").records, "记录");
  assert.equal(messages("zh-CN").allProjects, "全部项目");
  assert.equal(messages("zh-CN").failedStreak(2), "连续 2 次失败");
  assert.equal(messages("zh-CN").weeklyDays(5), "每周 5 天");
});

test("the editor uses the shared modal title, body, footer and close guard", () => {
  const setup = fixture(); setup.view.editing = {draft:{}}; setup.view.busy = true; setup.render();
  const dialog = setup.nodes.get("task-dialog-0");
  assert.deepEqual(plain(dialog.props), {open:true, dismissable:false, form:true, title:"New task", width:560});
  assert.deepEqual(dialog.items.map(item => item.kind), ["editor", "footer"]);
  setup.view.busy = false; setup.view.editing = null; setup.view.deleting = {id:"retained", revision:"2", name:"Retained task"}; setup.render();
  const confirmation = setup.nodes.get("task-dialog-0");
  assert.deepEqual(plain(confirmation.props), {open:true, dismissable:true, form:true, title:setup.view.text.deleteTitle, width:528});
  assert.equal(confirmation.items[0].items[0].items[0], "Retained task");
});

test("unrepresentable dates remain readable without changing schedule values",()=>{
  const now=Date.UTC(2026,9,18), next=now+Number.MAX_SAFE_INTEGER;
  const item={enabled:true,next_ms:next,timing:{kind:"every",data:{anchor_ms:now,interval_ms:Number.MAX_SAFE_INTEGER}}};
  assert.equal(schedule.upcoming(item,now,1)[0],next);
  const text=messages("en");
  assert.equal(schedule.clock(next),"—");assert.equal(schedule.dayLabel(next,now,text),"—");
  assert.equal(schedule.when(next,now,text),"—");assert.equal(schedule.relative(next,now,text),"—");
  assert.equal(schedule.interval(1,text),"Every 0.001 s");
  assert.notEqual(schedule.when(now,now,text),"—");
});


test("offline task copies keep details and disable mutations",()=>{
  const setup=fixture();setup.view.items=[task("offline",{revision:null,unavailable:true})];setup.render();
  for(const action of ["enabled","run","edit","delete"]){
    assert.equal(setup.nodes.get(`task-${action}-offline`).props.disabled,true);
    setup.view.control({id:`task-${action}-offline`,value:true},setup.cx);
  }
  assert.deepEqual(setup.calls,[]);
  assert.deepEqual(texts(setup.nodes.get("task-state-offline")),[setup.view.text.executionUnavailable]);
  setup.view.control({id:"task-details-offline"},setup.cx);
  assert.equal(setup.view.panel.task,"offline");
});

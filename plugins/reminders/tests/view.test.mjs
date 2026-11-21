import test from "node:test";
import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";
import vm from "node:vm";
import {messages} from "../dev.sailry.platform/desktop/locales.js";

const plain = value => JSON.parse(JSON.stringify(value));
const read = async name => (await readFile(new URL(`../dev.sailry.platform/desktop/${name}`, import.meta.url), "utf8"))
  .replace(/^import .*;\n/gm, "").replace(/^export default /gm, "").replace(/^export /gm, "");
const source = [await read("time.js"), await read("details.js"), await read("view.js")].join("\n");
const controller = await read("main.js");
const HOUR = 3600000;

function fixture(items = []) {
  const nodes = new Map(), calls = [], cx = {notify() {}, theme:() => ({typography:{mono:"mono"}})};
  function element(kind = "div", id = null, props = {}) {
    const value = {kind, selector:id, props, items:[], style:{}};
    const proxy = new Proxy(value, {get(target, key) {
      if (key === "id") return id => { target.selector = id; nodes.set(id, proxy); return proxy; };
      if (key === "child") return value => { target.items.push(value); return proxy; };
      if (key === "children") return values => { target.items.push(...values); return proxy; };
      if (key === "item") return (label, callback) => { target.items.push({label, callback}); return proxy; };
      if (key in target) return target[key];
      if (key === "on_click" || key === "on_change") return callback => { target[key] = callback; return proxy; };
      return (...args) => { target.style[key] = args; return proxy; };
    }});
    if (id) nodes.set(id, proxy);
    return proxy;
  }
  const constructors = Object.fromEntries(["Button", "Checkbox", "Icon", "Tag"]
    .map(kind => [kind, function(id, label) { return element(kind, id, {label}); }]));
  const adapters = Object.fromEntries(["CardColumn", "CardList", "CardRow", "CardSummary", "EmptyState", "IconButton", "Modal",
    "PanelHeader", "SegmentedTabs", "SelectField", "SettingsPage", "Loading", "Toggle", "Workspace"]
    .map(kind => [kind, {new:(id, props) => element(kind, id, props)}]));
  const {render} = vm.runInNewContext(`${source}\n({render})`, {
    div:() => element(), ...constructors, ...adapters,
    theme:() => ({colors:{foreground:"#f0f0f0ff", muted_foreground:"#909090ff", muted:"#202020ff", background:"#101010ff",
      border:"#303030ff", ring:"#4080f0ff", primary:"#4080f0ff", primary_foreground:"#ffffffff", destructive:"#f07050ff",
      success:"#50d080ff", info:"#4080f0ff", warning:"#e0a050ff"}}),
    editor:() => [element("editor"),element("footer")],
  });
  const Controller = vm.runInNewContext(`${controller}\nReminders`, {View:class {}, showWorkspaceDetails:() => calls.push(["show"])});
  const view = Object.assign(Object.create(Controller.prototype), {text:messages("en"), items, showCompleted:true, loading:false, loaded:true,
    panel:null, history:[], historyId:null, historyLoaded:false,
    projectFilter:"all", catalog:{projects:[{id:"project", name:"Current"}],worktrees:[]},
    busy:false, pending:null, error:null, editing:null, deleting:null, dialogId:0,
    refresh:() => calls.push(["refresh"]), edit:item => calls.push(["edit", item]),
    perform:(handler, input) => calls.push([handler, input]), close:() => calls.push(["close"]),
  });
  return {view, nodes, calls, cx, render() {
    nodes.clear();const body=render(view, cx);
    for(const list of [...nodes.values()].filter(node=>node.kind==='CardList'&&!node.items.length)) {
      const {loaded,empty_icon,empty_label}=list.props;
      list.child(loaded===false
        ? element('Loading',`${list.selector}-loading`,{})
        : element('EmptyState',`empty-${list.selector}-empty`,{variant:'list',icon:empty_icon,label:empty_label}));
    }
    return body;
  }, open(id, tab) { view.control({id:`reminder-${tab}-${id}`}, cx); view.historyId = id; view.historyLoaded = true; }};
}
function texts(node) {
  if (typeof node === "string") return [node];
  return node?.items ? node.items.flatMap(texts) : [];
}

test("page uses the centered settings layout and native toolbar controls", () => {
  const setup = fixture();
  setup.render();
  const {nodes} = setup, body = nodes.get("reminders");
  assert.equal(body.kind, "SettingsPage");
  assert.deepEqual(plain(body.props), {title:"Reminders", description:"To-dos and scheduled reminders"});
  assert.equal(nodes.get("reminders-items").kind, "CardList");
  const toolbar = nodes.get("reminders-toolbar");
  assert.deepEqual(plain(toolbar.style.items_center), []);
  assert.equal([...nodes.values()].some(node => ["TabBar", "Tab", "SegmentedTabs"].includes(node.kind)), false);
  assert.deepEqual(toolbar.items.map(child => child.items[0]?.kind ?? child.kind), ["SelectField", "Button"]);
  const filter = toolbar.items[0].items[0];
  assert.equal(filter.selector, "reminders-project-filter");
  assert.equal(filter.props.label, "Project");
  assert.equal(texts(toolbar).includes("Project"), false);
  assert.equal(toolbar.items[1].selector, "reminder-create");
  assert.equal(nodes.get("reminders-workspace").props.has_details, false);
  assert.equal(nodes.has("reminder-panel"), false);
  assert.equal(nodes.has("reminders-refresh"), false);
  assert.equal(nodes.has("reminders-reload"), false);
  assert.equal(nodes.get("empty-reminders-items-empty").kind, "EmptyState");
  assert.deepEqual(plain(nodes.get("reminders-items").props), {loaded:true,empty_icon:"bell",empty_label:"No reminders"});
  nodes.get("reminder-create").on_click({}, setup.cx);
  assert.deepEqual(setup.calls, [["edit", null]]);
});

test("load failure keeps the page without a retry bar", () => {
  const setup = fixture(); setup.view.error = "loadFailed"; setup.render();
  assert.equal(setup.nodes.has("reminders-refresh"), false);
  assert.equal(setup.nodes.has("reminders-reload"), false);
  assert.ok(setup.nodes.has("reminders-items"));
  assert.deepEqual(setup.calls, []);
});

test("forms delegate heading, close guard, body and footer to shared modals", () => {
  const setup = fixture(); setup.view.editing = {draft:{}}; setup.render();
  const dialog = setup.nodes.get("reminder-dialog-0");
  assert.deepEqual(plain(dialog.props), {open:true,dismissable:true,form:true,title:"New reminder",width:480});
  assert.deepEqual(dialog.items.map(item=>item.kind), ["editor","footer"]);
  setup.view.editing.draft.id="saved"; setup.view.busy=true; setup.render();
  assert.equal(setup.nodes.get("reminder-dialog-0").props.title,"Edit");
  assert.equal(setup.nodes.get("reminder-dialog-0").props.dismissable,false);
  setup.view.busy=false; setup.view.pending="request"; setup.render();
  assert.equal(setup.nodes.get("reminder-dialog-0").props.dismissable,true);
  setup.view.pending=null;setup.view.editing=null;setup.view.deleting={id:"retained",revision:"2",title:"Retained reminder"};setup.render();
  const confirmation=setup.nodes.get("reminder-dialog-0");
  assert.deepEqual(plain(confirmation.props),{open:true,dismissable:true,form:true,title:setup.view.text.deleteTitle,width:528});
  assert.deepEqual(confirmation.items.map(item=>item.selector),["reminder-delete-confirmation",null]);
  assert.equal(confirmation.items[0].items[0].items[0],"Retained reminder");
});

test("unconfirmed reads reserve the reminder card row", () => {
  const setup=fixture();setup.view.loaded=false;
  for(const loading of [false,true]) {
    setup.view.loading=loading;setup.render();
    assert.equal(setup.nodes.has("empty-reminders-items-empty"),false);
    const row=setup.nodes.get('reminders-items-loading');
    assert.equal(row.kind,'Loading');
    assert.equal(setup.nodes.get('reminders-toolbar').items.length,2);
  }
  setup.view.error='loadFailed';setup.render();
  assert.equal(setup.nodes.has('empty-reminders-items-empty'),false);
  assert.ok(setup.nodes.has('reminders-items-loading'));
});

test("background refresh preserves the loaded empty card", () => {
  const setup=fixture();setup.view.loaded=true;setup.view.loading=true;setup.render();
  assert.ok(setup.nodes.has("empty-reminders-items-empty"));
  assert.equal(setup.nodes.has('reminders-items-loading'),false);
  assert.ok(!setup.nodes.get("reminders-items").items.some(item=>item.items.includes(setup.view.text.loading)));
});

test("refresh keeps existing cards and the open editor", () => {
  const item={id:'saved',title:'Reminder',project:null,due_ms:null,notified_ms:null,completed:false},setup=fixture([item]);
  setup.view.editing={draft:{title:'Retained draft'}};
  for(const error of [null,'loadFailed']) {
    setup.view.loading=error===null;setup.view.error=error;setup.render();
    assert.ok(setup.nodes.has('reminder-card-saved'));
    assert.equal(setup.nodes.has('reminders-items-loading'),false);
    assert.equal(setup.nodes.has('empty-reminders-items-empty'),false);
    assert.deepEqual(setup.nodes.get('reminder-dialog-0').items.map(item=>item.kind),['editor','footer']);
  }
});

test("admitted actions do not insert recovery rows while still pending", () => {
  const setup=fixture();setup.view.busy=true;setup.view.pending="in-flight";setup.render();
  assert.equal(setup.nodes.has("reminder-retry"),false);
  assert.equal(setup.nodes.has("reminder-dismiss"),false);
  assert.ok(setup.nodes.has("reminders-toolbar") && setup.nodes.has("reminders-items"));
});

test("completion toggles and row actions route through the controller", () => {
  const pending = {id:"pending", project:null, revision:"4", title:"Pending item", message:"Keep this note", due_ms:null, notified_ms:null, completed:false};
  const completed = {...pending, id:"completed", title:"Completed item", completed:true};
  const setup = fixture([pending, completed]);
  setup.render();
  assert.equal(setup.nodes.get("reminder-card-pending").kind, "CardRow");
  assert.deepEqual(plain(setup.nodes.get("reminder-card-pending").props), {row_id:"reminder-row-pending"});
  setup.view.control({id:"reminder-complete-pending",value:true}, setup.cx);
  assert.deepEqual(plain(setup.calls[0]), ["save", {...pending, completed:true}]);
  setup.view.control({id:"reminder-edit-completed"}, setup.cx);
  assert.deepEqual(setup.calls[1], ["edit", completed]);
  setup.view.control({id:"reminder-delete-completed"}, setup.cx);
  assert.equal(setup.view.deleting, completed);
  setup.render();
  setup.nodes.get("reminder-delete-confirm").on_click({}, setup.cx);
  assert.deepEqual(plain(setup.calls[2]), ["remove", {id:"completed", revision:"4"}]);
});

test("open reminders sort by due time and completed ones collapse below", () => {
  const now = Date.now(), base = {project:null, revision:"1", message:"", notified_ms:null, completed:false};
  const items = [
    {...base, id:"later", title:"Later", due_ms:now + 5 * HOUR},
    {...base, id:"untimed", title:"Untimed", due_ms:null},
    {...base, id:"late", title:"Late", due_ms:now - 2 * HOUR},
    {...base, id:"soon", title:"Soon", due_ms:now + HOUR},
    {...base, id:"done", title:"Done", due_ms:now - HOUR, completed:true},
    {...base, id:"finished", title:"Finished", due_ms:null, completed:true},
  ];
  const setup = fixture(items); setup.render();
  assert.deepEqual(setup.nodes.get("reminders-items").items.map(card => card.selector),
    ["reminder-card-late", "reminder-card-soon", "reminder-card-later", "reminder-card-untimed"]);
  assert.deepEqual(setup.nodes.get("reminders-done").items.map(card => card.selector), ["reminder-card-done", "reminder-card-finished"]);
  const toggle = setup.nodes.get("reminders-completed-toggle");
  assert.deepEqual(plain(toggle.style.ghost), []);
  assert.deepEqual(texts(toggle), ["Completed", "2"]);
  assert.equal(toggle.items[0].items[0].kind, "Icon");
  assert.equal(toggle.items[0].items[0].selector, "icons/chevron-down.svg");
  assert.deepEqual(plain(setup.nodes.get("reminder-card-done").style.line_through), []);
  toggle.on_click({}, setup.cx); setup.render();
  assert.equal(setup.view.showCompleted, false);
  assert.equal(setup.nodes.has("reminders-done"), false);
  assert.equal(setup.nodes.has("reminder-card-done"), false);
  assert.equal(setup.nodes.get("reminders-completed-toggle").items[0].items[0].selector, "icons/chevron-right.svg");
  setup.nodes.get("reminders-completed-toggle").on_click({}, setup.cx); setup.render();
  assert.ok(setup.nodes.has("reminder-card-done"));
});

test("completed-only lists hide the open list; empty pages keep one placeholder", () => {
  const setup = fixture([{id:"done", project:null, revision:"1", title:"Done", message:"", due_ms:null, notified_ms:null, completed:true}]);
  setup.render();
  assert.equal(setup.nodes.has("reminders-items"), false);
  assert.equal(setup.nodes.has("empty-reminders-items-empty"), false);
  assert.ok(setup.nodes.has("reminder-card-done"));
  setup.view.items = []; setup.render();
  assert.ok(setup.nodes.has("empty-reminders-items-empty"));
  assert.equal(setup.nodes.has("reminders-completed"), false);
});

test("cards show due text, an overdue tag and panel actions", () => {
  const now = Date.now(), base = {project:null, revision:"1", message:"", notified_ms:null, completed:false};
  const setup = fixture([{...base, id:"late", title:"Late", due_ms:now - HOUR}, {...base, id:"soon", title:"Soon", due_ms:now + HOUR},
    {...base, id:"untimed", title:"Untimed", due_ms:null}, {...base, id:"done", title:"Done", due_ms:now - HOUR, completed:true}]);
  setup.render();
  const late = setup.nodes.get("reminder-when-late");
  assert.deepEqual(plain(late.style.text_color), ["#f07050ff"]);
  assert.equal(setup.nodes.get("reminder-state-late").items[0].items[0], "Overdue");
  assert.deepEqual(plain(setup.nodes.get("reminder-state-late").items[0].style.variant), ["danger"]);
  assert.deepEqual(plain(setup.nodes.get("reminder-when-soon").style.text_color), ["#909090ff"]);
  assert.equal(setup.nodes.has("reminder-state-soon"), false);
  assert.deepEqual(setup.nodes.get("reminder-when-untimed").items, ["No time"]);
  assert.equal(setup.nodes.has("reminder-state-done"), false);
  assert.deepEqual(plain(setup.nodes.get("reminder-when-done").style.text_color), ["#909090ff"]);
});

test("cards delegate ordered column roles and preserve native actions", () => {
  const item = {id:"row", project:"project", revision:"1", title:"Long reminder 中文 🙂", message:"Preserve this note in the editor",
    due_ms:null, notified_ms:null, completed:false};
  const setup = fixture([item]); setup.render();
  const row = setup.nodes.get("reminder-card-row");
  assert.equal(row.kind, "CardRow");
  assert.deepEqual(plain(row.props), {row_id:"reminder-row-row"});
  assert.deepEqual(row.items.map(child => child.kind), ["CardSummary","CardColumn","CardColumn"]);
  assert.deepEqual(row.items.map(child => child.selector), ["reminder-summary-row","reminder-time-row","reminder-actions-row"]);
  assert.deepEqual(row.items.slice(1).map(child => child.props.variant), ["wide_metadata","actions"]);
  assert.deepEqual(plain(row.items[0].props), {title:item.title,subtitle:"Current",icon:"icons/folder.svg"});
  const complete = setup.nodes.get("reminder-complete-row");
  assert.equal(complete.kind, "Toggle");
  assert.deepEqual(plain(complete.props), {variant:"checkbox",label:"Complete: Long reminder 中文 🙂",checked:false,disabled:false});
  assert.equal(row.items[0].items[0], complete);
  assert.deepEqual(texts(setup.nodes.get("reminder-time-row")), ["No time"]);
  const actions = setup.nodes.get("reminder-actions-row");
  assert.deepEqual(plain(actions.props), {variant:"actions"});
  assert.deepEqual(actions.items.map(child => child.selector),
    ["reminder-details-row","reminder-records-row","reminder-edit-row","reminder-delete-row"]);
  assert.ok(actions.items.every(child => child.kind === "IconButton"));
  assert.deepEqual(plain(actions.items.map(child => child.props)), [
    {icon:"info",label:"Details",selected:false},
    {icon:"reicon:time/clock",label:"Records",selected:false},
    {icon:"reicon:newicons/edit",label:"Edit",disabled:false},
    {icon:"reicon:ui/trash2",label:"Delete",disabled:false},
  ]);
  assert.ok(actions.items.every(child => !Object.hasOwn(child,"on_click")));
  assert.equal(JSON.stringify(plain(row.items)).includes(item.message), false);
  setup.view.control({id:"reminder-edit-row"}, setup.cx);
  assert.equal(setup.calls[0][1].message, item.message);
});

test("completed cards inherit a whole-row strike without changing native controls", () => {
  const item = {id:"done", project:"project", revision:"1", title:"Finished", message:"Original note", due_ms:null, notified_ms:0, completed:true};
  const setup = fixture([item, {...item,id:"pending",completed:false}]); setup.render();
  const row = setup.nodes.get("reminder-card-done");
  assert.deepEqual(plain(row.style.line_through), []);
  assert.equal(setup.nodes.get("reminder-card-pending").style.line_through, undefined);
  assert.equal(row.items[0].kind, "CardSummary");
  assert.deepEqual(row.items.slice(1).map(child => child.props.variant), ["wide_metadata","actions"]);
  assert.equal(setup.nodes.get("reminder-complete-done").props.checked, true);
  assert.equal(setup.nodes.get("reminder-edit-done").props.label, "Edit");
  setup.view.control({id:"reminders-project-filter",value:"project"},setup.cx); setup.render();
  assert.ok(setup.nodes.has("reminder-card-done") && setup.nodes.has("reminder-card-pending"));
  assert.equal(setup.view.projectFilter,"project");
});

test("time labels use local day names in both locales", () => {
  const now = Date.now(), item = {id:"row",project:null,revision:"1",title:"Reminder",message:"",notified_ms:null,completed:false};
  const setup = fixture([]);
  for (const [locale, today, tomorrow] of [["en", "Today", "Tomorrow"], ["zh", "今天", "明天"]]) {
    setup.view.text = messages(locale);
    const soon = new Date(now); soon.setHours(23, 59, 0, 0);
    const next = new Date(now); next.setDate(next.getDate() + 1); next.setHours(9, 5, 0, 0);
    setup.view.items = [{...item, id:"today", due_ms:soon.getTime()}, {...item, id:"next", due_ms:next.getTime()}]; setup.render();
    assert.deepEqual(setup.nodes.get("reminder-when-today").items, [`${today} 23:59`]);
    assert.deepEqual(setup.nodes.get("reminder-when-next").items, [`${tomorrow} 09:05`]);
  }
});

test("uncertain actions retain retry and dismiss controls without enabling writes", () => {
  const item = {id:"item", project:null, revision:"3", title:"Saved", message:"", due_ms:null, notified_ms:null, completed:false};
  const setup = fixture([item]);
  setup.view.pending = "original-request";
  setup.view.error = "unknown";
  setup.render();
  assert.deepEqual(plain(setup.nodes.get("reminder-create").style.disabled), [true]);
  for (const id of ["reminder-edit-item", "reminder-delete-item"])
    assert.equal(setup.nodes.get(id).props.disabled, true);
  assert.equal(setup.nodes.get("reminder-complete-item").props.disabled, true);
  setup.view.control({id:"reminder-complete-item",value:true},setup.cx);
  setup.view.control({id:"reminder-delete-item"},setup.cx);
  assert.deepEqual(setup.calls, []);
  assert.equal(setup.view.deleting,null);
  setup.nodes.get("reminder-retry").on_click({}, setup.cx);
  setup.nodes.get("reminder-dismiss").on_click({}, setup.cx);
  assert.deepEqual(setup.calls, [[null, null], ["close"]]);
  assert.equal(setup.view.pending, "original-request");
});

test("desktop and conversation tools share the host package", async () => {
  const manifest = JSON.parse(await readFile(new URL("../plugin.json", import.meta.url), "utf8"));
  const extension = manifest.extensions["dev.sailry.platform"];
  assert.equal(extension.scope, undefined);
  assert.equal(extension.desktop.navigation_options.target, undefined);
  assert.equal(extension.api_version, "v1");
  assert.equal(manifest.version, "0.1.20");
  assert.equal(extension.desktop.navigation_options.details, true);
  assert.ok(extension.host.handlers.includes("history"));
  assert.deepEqual(extension.actions, ["storage.read", "storage.write", "dispatch.manage", "notifications.publish", "projects.read"]);
  assert.equal(extension.tools.length, 1);
  assert.deepEqual(extension.tools[0].contexts, ["workspace", "plugin"]);
  assert.deepEqual(extension.tools[0].handler.flow.operations, ["storage.transaction"]);
  assert.equal("project" in extension.tools[0].handler.parameters.properties, false);
  assert.equal(extension.host.turn.initialize, "initialize");
  assert.equal(messages("zh-CN").pageTitle, "待办提醒");
  assert.equal(messages("zh-CN").description, "管理待办事项与定时提醒");
});

test("captured tool displays exclude localized action and target prompts from payloads", async () => {
  const manifest = JSON.parse(await readFile(new URL("../plugin.json", import.meta.url), "utf8"));
  const tool = manifest.extensions["dev.sailry.platform"].tools[0];
  assert.equal(tool.presentation,"summary");
  assert.deepEqual(tool.display.input,{summary:{path:"/title"}});
  assert.equal(tool.display.output,undefined);
  assert.deepEqual(tool.display.approval.map(prompt=>prompt.message.label),[
    "Create reminder %{title}","Update reminder %{title}","Update reminder %{id}","Delete reminder %{id}",
  ]);
  assert.deepEqual(tool.display.approval.map(prompt=>prompt.message.locales["zh-CN"]),[
    "创建提醒 %{title}","更新提醒 %{title}","更新提醒 %{id}","删除提醒 %{id}",
  ]);
  assert.deepEqual(tool.display.approval.map(prompt=>prompt.values),[
    {title:{source:"arguments",path:"/title"}},{title:{source:"arguments",path:"/title"}},
    {id:{source:"arguments",path:"/id"}},{id:{source:"arguments",path:"/id"}},
  ]);
  const matches = (prompt,input) => prompt.when.all.every(condition => {
    const value = input[condition.value.path.slice(1)] ?? null;
    return (value === condition.equals) !== !!condition.negate;
  });
  for (const [input,index] of [[{action:"create",title:"New"},0],[{action:"update",title:"Changed"},1],
    [{action:"update",id:"saved"},2],[{action:"delete",id:"saved"},3]]) {
    assert.deepEqual(tool.display.approval.map((prompt,index)=>matches(prompt,input) ? index : null).filter(index=>index!==null),[index]);
  }
  assert.ok(tool.display.approval.every(prompt=>!matches(prompt,{action:"list"})));
});

test("project filter keeps all, unassigned and unavailable associations distinct", () => {
  const item = {id:"current", project:"project", revision:"1", title:"Current", message:"", due_ms:null, notified_ms:null, completed:false};
  const setup = fixture([item, {...item,id:"global",project:null}, {...item,id:"missing",project:"removed"}]);
  setup.render();
  assert.equal(setup.nodes.get("reminders-project-filter").kind, "SelectField");
  assert.equal(setup.nodes.get("reminders-project-filter").props.label, "Project");
  assert.deepEqual(plain(setup.nodes.get("reminders-project-filter").props.items).map(item => item.label),
    ["All projects", "No project", "Current", "Unavailable project"]);
  assert.equal(setup.nodes.get("reminder-summary-current").props.subtitle, "Current");
  assert.equal(setup.nodes.get("reminder-summary-missing").props.subtitle, "Unavailable project");
  setup.view.control({id:"reminders-project-filter",value:"none"}, setup.cx);
  setup.render();
  assert.ok(setup.nodes.has("reminder-card-global"));
  assert.equal(setup.nodes.has("reminder-card-current"), false);
  setup.view.control({id:"reminders-project-filter",value:"project"}, setup.cx);
  setup.render();
  assert.ok(setup.nodes.has("reminder-card-current"));
  assert.equal(setup.nodes.has("reminder-card-global"), false);
  setup.view.control({id:"reminders-project-filter",value:"removed"}, setup.cx);
  setup.render();
  assert.ok(setup.nodes.has("reminder-card-missing"));
  setup.nodes.get("reminder-create").on_click({}, setup.cx);
  assert.deepEqual(setup.calls, [["edit", null]]);
  assert.equal(setup.view.projectFilter, "removed");
  setup.view.control({id:"reminders-project-filter",value:"not-a-choice"}, setup.cx);
  assert.equal(setup.view.projectFilter, "removed");
  setup.view.control({id:"reminders-project-filter",value:"all"}, setup.cx); setup.render();
  assert.equal(setup.nodes.get("reminders-project-filter").props.selected, "all");
  assert.ok(setup.nodes.has("reminder-card-current") && setup.nodes.has("reminder-card-global") && setup.nodes.has("reminder-card-missing"));
});

const reminder = (id, extra = {}) => ({id, project:"project", revision:"1", title:`Reminder ${id}`, message:"",
  due_ms:null, notified_ms:null, completed:false, ...extra});
const job = (id, handler, status, extra = {}) => ({id, handler, status, created_ms:Date.now() - 60000,
  started_ms:Date.now() - 60000, finished_ms:Date.now() - 59000, error:null, ...extra});

test("details and records buttons open the side panel on that tab", () => {
  const setup = fixture([reminder("a"), reminder("b")]);
  setup.render();
  assert.equal(setup.nodes.has("reminder-panel"), false);
  setup.view.control({id:"reminder-details-a"}, setup.cx);
  assert.deepEqual(plain(setup.view.panel), {reminder:"a", tab:"details"});
  assert.deepEqual(setup.calls, [["refresh"], ["show"]]);
  setup.render();
  assert.equal(setup.nodes.get("reminders-workspace").props.has_details, true);
  assert.equal(setup.nodes.get("reminders-workspace").props.default_details_width, 380);
  assert.equal(setup.nodes.get("reminder-panel-tabs").props.selected, "details");
  assert.ok(setup.nodes.has("reminder-overview"));
  assert.deepEqual(plain(setup.nodes.get("reminder-card-a").style.border_color), ["#4080f0ff"]);
  assert.deepEqual(plain(setup.nodes.get("reminder-card-b").style.border_color), ["#303030ff"]);
  assert.equal(setup.nodes.get("reminder-details-a").props.selected, true);
  setup.view.control({id:"reminder-panel-tabs", value:"records"}, setup.cx); setup.render();
  assert.equal(setup.view.panel.tab, "records");
  assert.ok(setup.nodes.has("reminder-records-loading"));
  assert.equal(setup.nodes.get("reminder-records-a").props.selected, true);
  setup.view.control({id:"reminder-panel-tabs", value:"other"}, setup.cx);
  assert.equal(setup.view.panel.tab, "records");
  setup.view.control({id:"reminder-records-a"}, setup.cx);
  assert.deepEqual(setup.calls.filter(call => call[0] === "refresh").length, 1);
  setup.view.control({id:"reminder-details-b"}, setup.cx);
  assert.deepEqual(plain(setup.view.panel), {reminder:"b", tab:"details"});
  assert.equal(setup.view.historyLoaded, false);
  setup.view.control({id:"reminder-panel-close"}, setup.cx); setup.render();
  assert.equal(setup.view.panel, null);
  assert.equal(setup.nodes.has("reminder-panel"), false);
});

test("panel headers place the status tag after the title without actions", () => {
  const setup = fixture([reminder("a", {due_ms:Date.now() - HOUR, message:"Reply to review"})]);
  setup.open("a", "details"); setup.render();
  const summary = setup.nodes.get("reminder-panel-summary");
  assert.deepEqual(summary.items[0].items.map(child => child.selector ?? texts(child)[0]), ["Reminder a", "reminder-panel-status"]);
  assert.equal(summary.items[0].style.justify_between, undefined);
  assert.equal(setup.nodes.get("reminder-panel-status").items[0].items[0], "Overdue");
  assert.deepEqual(plain(setup.nodes.get("reminder-panel-status").items[0].style.size), ["medium"]);
  assert.deepEqual(texts(setup.nodes.get("reminder-panel-project")), ["Current"]);
  assert.equal(setup.nodes.get("reminder-panel-project").items[0].kind, "Icon");
  assert.equal(setup.nodes.get("reminder-fact-project").items[1].items[0].kind, "Icon");
  const nodes = node => node?.items ? [node, ...node.items.flatMap(nodes)] : [];
  assert.deepEqual(nodes(setup.nodes.get("reminder-panel")).filter(node => ["IconButton", "Button"].includes(node.kind))
    .map(node => node.selector), ["reminder-panel-close"]);
  assert.deepEqual(texts(setup.nodes.get("reminder-note")), ["Reply to review"]);
});

test("details show overdue, upcoming, completed and untimed states", () => {
  const now = Date.now(), setup = fixture([
    reminder("late", {due_ms:now - 80 * 60000, notified_ms:now - 80 * 60000}),
    reminder("soon", {due_ms:now + 17 * HOUR}),
    reminder("done", {due_ms:now - HOUR, completed:true}),
    reminder("none"),
  ]);
  const show = id => { setup.open(id, "details"); setup.render(); };
  const facts = () => ["status", "notice", "project"].map(key => texts(setup.nodes.get(`reminder-fact-${key}`)));
  show("late");
  assert.deepEqual(plain(setup.nodes.get("reminder-due-month").style.bg), ["#f07050ff"]);
  assert.deepEqual(texts(setup.nodes.get("reminder-due-relative")), ["1 h 20 min ago"]);
  assert.deepEqual(plain(setup.nodes.get("reminder-due-relative").style.text_color), ["#f07050ff"]);
  assert.deepEqual(plain(setup.nodes.get("reminder-due-time").style.font_family), ["mono"]);
  assert.deepEqual(facts().map(values => values.slice(0, 2)), [["Status", "To do"], ["Reminder", "Notified"], ["Project", "Current"]]);
  assert.equal(facts()[1].length, 3);
  show("soon");
  assert.equal(setup.nodes.get("reminder-panel-status").items[0].items[0], "To do");
  assert.deepEqual(plain(setup.nodes.get("reminder-panel-status").items[0].style.variant), ["warning"]);
  assert.deepEqual(plain(setup.nodes.get("reminder-due-month").style.bg), ["#4080f0ff"]);
  assert.deepEqual(texts(setup.nodes.get("reminder-due-relative")), ["in 17 h"]);
  assert.deepEqual(facts()[1], ["Reminder", "Waiting"]);
  show("done");
  assert.equal(setup.nodes.get("reminder-panel-status").items[0].items[0], "Completed");
  assert.deepEqual(plain(setup.nodes.get("reminder-due-month").style.bg), ["#50d080ff"]);
  assert.deepEqual(facts()[0], ["Status", "Completed"]);
  assert.deepEqual(facts()[1], ["Reminder", "None"]);
  show("none");
  assert.deepEqual(plain(setup.nodes.get("reminder-due-month").style.bg), ["#909090ff"]);
  assert.deepEqual(texts(setup.nodes.get("reminder-due-tile")), ["—", "—"]);
  assert.deepEqual(texts(setup.nodes.get("reminder-due-time")), ["No reminder time"]);
  assert.deepEqual(texts(setup.nodes.get("reminder-due-relative")), ["No notification"]);
  assert.deepEqual(texts(setup.nodes.get("reminder-note")), ["No notes"]);
  assert.equal([...setup.nodes.values()].some(node => ["Info", "Notes"].includes(texts(node)[0]) && node.items.length === 1), false);
  setup.view.text = messages("zh"); show("late");
  assert.deepEqual(texts(setup.nodes.get("reminder-due-relative")), ["已过 1 小时 20 分"]);
  show("soon");
  assert.deepEqual(texts(setup.nodes.get("reminder-due-relative")), ["还有 17 小时"]);
});

test("records list only this reminder's deliveries with labels and errors", () => {
  const setup = fixture([reminder("a"), reminder("b")]);
  setup.open("a", "records");
  setup.view.history = [job("j1", "a", "completed"), job("j2", "b", "completed"),
    job("j3", "a", "failed", {error:{code:"internal", message:"Notifier offline"}}), job("j4", "a", "queued", {started_ms:null, finished_ms:null}),
    job("j5", "a", "unknown")];
  setup.render();
  assert.deepEqual(plain(setup.nodes.get("reminder-panel-tabs").props.items.map(item => item.label)), ["Details", "Records 4"]);
  for (const id of ["j1", "j3", "j4", "j5"]) assert.ok(setup.nodes.has(`reminder-record-${id}`));
  assert.equal(setup.nodes.has("reminder-record-j2"), false);
  assert.deepEqual(["j1", "j3", "j4", "j5"].map(id => texts(setup.nodes.get(`reminder-record-card-${id}`))[0]),
    ["Notified", "Failed", "Waiting", "Result unknown"]);
  assert.deepEqual(texts(setup.nodes.get("reminder-record-error-j3")), ["Notifier offline"]);
  assert.ok(setup.nodes.has("reminder-day-0"));
  assert.equal([...setup.nodes.values()].some(node => node.kind === "LoadMore"), false);
  setup.view.history = [job("j2", "b", "completed")]; setup.render();
  assert.equal(setup.nodes.get("reminder-records-empty").kind, "EmptyState");
  assert.deepEqual(plain(setup.nodes.get("reminder-records-empty").props), {variant:"list", icon:"bell", label:"No records"});
  assert.deepEqual(plain(setup.nodes.get("reminder-panel-tabs").props.items.map(item => item.label)), ["Details", "Records 0"]);
});


test("offline reminder copies keep details and disable mutations", () => {
  const item = {id:"offline",project:null,revision:null,title:"Offline",message:"",due_ms:null,notified_ms:null,completed:false,unavailable:true};
  const setup = fixture([item]); setup.render();
  for (const action of ["complete","edit","delete"]) {
    assert.equal(setup.nodes.get(`reminder-${action}-offline`).props.disabled,true);
    setup.view.control({id:`reminder-${action}-offline`,value:true},setup.cx);
  }
  assert.deepEqual(setup.calls,[]);
  assert.deepEqual(texts(setup.nodes.get("reminder-when-offline")),[setup.view.text.executionUnavailable]);
  setup.view.control({id:"reminder-details-offline"},setup.cx);
  assert.equal(setup.view.panel.reminder,"offline");
});

import test from "node:test";
import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";
import {fixture,id,plain} from "./store.mjs";
import {runtimeTest,validateSchema} from "../../scripts/test-host.mjs";

test("initialization captures the execution scope and explicit session config",async()=>{
  const setup=await fixture(), initialized=await setup.initialize();
  assert.deepEqual(setup.turn(),{project:id(1),worktree:id(2),config:setup.config,...setup.execution});
  assert.match(initialized.instruction,/Current UTC: .* ms/);
  setup.session.config={...setup.config,model:"another"};
  assert.equal(setup.turn().config.model,"chosen");
  assert.deepEqual(plain((await setup.initialize({...setup.config,assistant:{package:"assistant"}})).tools),[]);
});

test("creation uses one admitted storage and dispatch transaction",async()=>{
  const setup=await fixture(), timing={kind:"every",data:{anchor_ms:Date.now()+3_600_000,interval_ms:86_400_000}};
  const reply=await setup.plan({action:"create",name:"Daily brief",prompt:"Prepare a brief 中文 🙂",timing});
  assert.equal(reply.call.operation,"storage.transaction");assert.equal(reply.result,undefined);
  assert.equal(setup.values.size,0);assert.equal(setup.transactions.length,0);
  assert.deepEqual(plain(reply.call.arguments.operations).map(operation=>operation.kind==="dispatch"?operation.data.kind:operation.kind),
    ["write","save_handler","save_schedule"]);
  const callback=reply.call.arguments.operations.find(operation=>operation.data.kind==="save_handler").data.data.callback;
  assert.equal(callback.command.kind,"start_session");assert.equal(callback.completion,"turn");
  assert.deepEqual(plain(callback.scope),{worktree:id(2),session:id(5)});
  assert.equal(callback.command.data.project,id(1));assert.equal(callback.command.data.worktree,id(2));
  assert.equal(reply.call.arguments.operations.find(operation=>operation.data.kind==="save_handler").data.data.queue,"default");
  assert.ok(setup.requests.every(request=>request.data?.action?.kind!=="list_queues"));
  assert.deepEqual(plain(callback.command.data.config),setup.config);
  assert.equal(callback.command.data.execution,"team");assert.equal(callback.command.data.team,null);
  assert.deepEqual(plain(callback.command.data.association),{namespace:"scheduled-tasks",kind:"task",id:id(10)});
  const {result}=await setup.commit(reply);
  assert.equal(result.id,id(10));assert.equal(result.revision,"1");assert.equal(result.enabled,true);
  const listed=await setup.plan({action:"list"});
  assert.equal(listed.result.items.length,1);assert.deepEqual(plain(listed.result.items[0].timing),timing);
  assert.equal("queues" in listed.result,false);
});

test("creation inherits captured execution and recurring team identity",async()=>{
  const setup=await fixture(), team={id:id(20),revision:"7"}, models={[id(30)]:{provider:id(3),model:"lead",effort:"high"}};
  setup.execution={execution:"team",team,models};await setup.initialize();
  setup.execution={execution:"model",team:null,models:{}};
  setup.teams[0].revision="8";
  const reply=await setup.plan({action:"create",name:"Team review",prompt:"Review",
    timing:{kind:"every",data:{anchor_ms:Date.now()+3_600_000,interval_ms:86_400_000}}});
  const callback=reply.call.arguments.operations.find(operation=>operation.data.kind==="save_handler").data.data.callback;
  assert.equal(callback.command.data.execution,"team");
  assert.deepEqual(plain(callback.command.data.team),{id:id(20),revision:null});
  assert.deepEqual(plain(callback.command.data.models),models);
  const saved=(await setup.commit(reply)).result;
  assert.deepEqual(plain(saved.team),{id:id(20),revision:null});
  assert.deepEqual(plain(saved.models),models);
  assert.equal(saved.execution,"team");
  setup.nextCall(id(12));await setup.initialize();
  const model=await setup.plan({action:"create",name:"Model review",prompt:"Review",timing:saved.timing});
  const created=(await setup.commit(model)).result;
  assert.equal(created.execution,"model");assert.equal(created.team,null);assert.deepEqual(plain(created.models),{});
});

test("explicit execution choices clear only incompatible member overrides",async()=>{
  const setup=await fixture(), item={...setup.task(11),execution:"team",team:{id:id(20),revision:null},
    models:{[id(30)]:{provider:id(3),model:"lead",effort:"high"}}};setup.seed(item);
  for(const change of [{name:"Renamed"},{execution:"team"},{team:{id:id(20)}}]) {
    const reply=await setup.plan({action:"update",id:item.id,revision:"1",...change});
    assert.deepEqual(plain(reply.call.arguments.operations[0].data.value.models),item.models);
    assert.deepEqual(plain(reply.call.arguments.operations[0].data.value.team),item.team);
  }
  const model=await setup.plan({action:"update",id:item.id,revision:"1",execution:"model"});
  const saved=(await setup.commit(model)).result;
  assert.equal(saved.execution,"model");assert.equal(saved.team,null);assert.deepEqual(plain(saved.models),{});
  const named=await setup.plan({action:"update",id:item.id,revision:"2",team:{id:id(20)}});
  const selected=(await setup.commit(named)).result;
  assert.equal(selected.execution,"team");assert.deepEqual(plain(selected.team),{id:id(20),revision:null});
  assert.deepEqual(plain(selected.models),{});
  const defaults=await setup.plan({action:"update",id:item.id,revision:"3",team:null});
  const reset=(await setup.commit(defaults)).result;
  assert.equal(reset.execution,"team");assert.equal(reset.team,null);assert.deepEqual(plain(reset.models),{});
});

test("creation can explicitly replace inherited team execution",async()=>{
  const setup=await fixture();setup.execution={execution:"team",team:{id:id(20),revision:"7"},
    models:{[id(30)]:{provider:id(3),model:"lead",effort:"high"}}};await setup.initialize();
  const reply=await setup.plan({action:"create",name:"Single model",prompt:"Review",execution:"model",
    timing:{kind:"once",data:{at_ms:Date.now()+3_600_000}}});
  const saved=(await setup.commit(reply)).result;
  assert.equal(saved.execution,"model");assert.equal(saved.team,null);assert.deepEqual(plain(saved.models),{});
  assert.deepEqual(plain(saved.config),setup.config);
});

test("stored team metadata permits reads and task updates",async()=>{
  const setup=await fixture(), item={...setup.task(11),execution:"team",
    team:{id:id(20),revision:null,annotation:{label:"Saved metadata"}},models:{}};
  setup.seed(item);
  const listed=(await setup.plan({action:"list"})).result.items;
  assert.deepEqual(plain(listed[0].team),item.team);
  const reply=await setup.plan({action:"update",id:item.id,revision:"1",name:"Renamed"});
  assert.equal(reply.result,undefined);
  const operation=reply.call.arguments.operations[0];
  assert.equal(operation.data.value.name,"Renamed");
  assert.deepEqual(plain(operation.data.value.team),{id:id(20),revision:null});
  const callback=reply.call.arguments.operations.find(operation=>operation.data.kind==="save_handler").data.data.callback;
  assert.deepEqual(plain(callback.command.data.team),{id:id(20),revision:null});
  assert.deepEqual(plain(callback.scope),{worktree:id(2),session:id(5)});
});

test("captured display uses shared summary and localized action-target approval templates",async()=>{
  const manifest=JSON.parse(await readFile(new URL("../plugin.json",import.meta.url),"utf8"));
  const tool=manifest.extensions["dev.sailry.platform"].tools[0];
  assert.equal(tool.presentation,"summary");assert.deepEqual(tool.display.input,{summary:{path:"/name"}});
  assert.equal(tool.display.output,undefined);
  assert.deepEqual(tool.display.approval.map(prompt=>prompt.message.label),[
    "Create scheduled task %{name}","Update scheduled task %{name}","Update scheduled task %{id}","Delete scheduled task %{id}",
  ]);
  assert.deepEqual(tool.display.approval.map(prompt=>prompt.message.locales["zh-CN"]),[
    "创建定时任务 %{name}","更新定时任务 %{name}","更新定时任务 %{id}","删除定时任务 %{id}",
  ]);
  assert.deepEqual(tool.display.approval.map(prompt=>prompt.values),[
    {name:{source:"arguments",path:"/name"}},{name:{source:"arguments",path:"/name"}},
    {id:{source:"arguments",path:"/id"}},{id:{source:"arguments",path:"/id"}},
  ]);
  assert.deepEqual(tool.display.approval.map(prompt=>prompt.when.all[0]),["create","update","update","delete"]
    .map(action=>({value:{source:"arguments",path:"/action"},equals:action})));
  assert.deepEqual(tool.display.approval[1].when.all[1],{value:{source:"arguments",path:"/name"},equals:null,negate:true});
  assert.deepEqual(tool.display.approval[2].when.all[1],{value:{source:"arguments",path:"/name"},equals:null});
});

test("updates preserve saved model and scope and deletion cancels the matching schedule",async()=>{
  const setup=await fixture(), item=setup.task(11);
  item.config.model="saved-model";item.queue="custom";item.timing={kind:"every",data:{anchor_ms:1,interval_ms:86_400_000}};
  setup.seed(item);
  const reply=await setup.plan({action:"update",id:item.id,revision:"1",name:"Reviewed",enabled:true});
  const saved=(await setup.commit(reply)).result;
  assert.equal(saved.revision,"2");assert.equal(saved.name,"Reviewed");assert.equal(saved.config.model,"saved-model");
  assert.equal(saved.project,item.project);assert.equal(saved.worktree,item.worktree);assert.equal(saved.queue,"custom");
  assert.deepEqual(plain(saved.timing),item.timing);assert.equal(saved.prompt,item.prompt);
  const removal=await setup.plan({action:"delete",id:item.id,revision:"2"});
  const handler=removal.call.arguments.operations.find(operation=>operation.data.kind==="remove_handler");
  assert.equal(handler.data.data.cancel_pending,true);
  const deleted=(await setup.commit(removal)).result;
  assert.deepEqual(plain(deleted),{id:item.id,revision:"3",removed:true});
  assert.equal(setup.values.get(`task/${item.id}`).present,false);assert.equal(setup.schedules.has(item.id),false);assert.equal(setup.handlers.has(item.id),false);
});

test("project and unassigned tools do not list or mutate another scope",async()=>{
  const setup=await fixture();
  const current=setup.task(11), foreign=setup.task(12,id(4)), global=setup.task(13,null);
  for(const item of [current,foreign,global])setup.seed(item);
  assert.deepEqual(plain((await setup.plan({action:"list"})).result.items).map(item=>item.id),[current.id]);
  for(const item of [foreign,global]) {
    for(const args of [{action:"update",id:item.id,revision:"1",enabled:true},{action:"delete",id:item.id,revision:"1"}]) {
      const reply=await setup.plan(args);assert.equal(reply.result.error.code,"permission_denied");assert.equal(reply.call,undefined);
    }
  }
  setup.project=null;await setup.initialize();
  assert.equal(setup.turn().worktree,null);
  assert.deepEqual(plain((await setup.plan({action:"list"})).result.items).map(item=>item.id),[global.id]);
  const reply=await setup.plan({action:"create",name:"Global",prompt:"Review",enabled:false,timing:{kind:"once",data:{at_ms:Date.now()+3_600_000}}});
  const callback=reply.call.arguments.operations.find(operation=>operation.data.kind==="save_handler").data.data.callback;
  assert.deepEqual(plain(callback.scope),{worktree:id(2),session:id(5)});
  assert.equal(callback.command.data.project,null);assert.equal(callback.command.data.worktree,null);
  const saved=(await setup.commit(reply)).result;
  assert.equal(saved.project,null);assert.equal(saved.worktree,null);
});

test("Node-wide management retains its unscoped callback authority",async()=>{
  const setup=await fixture();
  setup.worktree=null;setup.contextSession=null;
  const pending=await setup.api.saveOperations({...setup.task(11),project:null,worktree:null,revision:"0"});
  const callback=pending.operations.find(operation=>operation.data.kind==="save_handler").data.data.callback;
  assert.deepEqual(plain(callback.scope),{worktree:null,session:null});
});

test("strict arguments and stale revisions fail before a mutation is prepared",async()=>{
  const setup=await fixture(), item=setup.task(11);setup.seed(item);
  const create={action:"create",name:"Review",prompt:"Read",timing:{kind:"once",data:{at_ms:Date.now()+3_600_000}}};
  for(const args of [
    {...create,project:id(4)},{...create,config:setup.config},{...create,model:"another"},{...create,enabled:null},
    {...create,execution:"unknown"},{...create,execution:null},{...create,team:"default"},
    {...create,team:{id:"invalid"}},{...create,team:{id:id(20),extra:true}},
    {...create,execution:"model",team:{id:id(20)}},
    {...create,timing:{kind:"once",data:{at_ms:Date.now()-1}}},
    {...create,timing:{kind:"once",data:{at_ms:Date.now()+3_600_000,timezone:"UTC"}}},
    {...create,timing:{kind:"every",data:{anchor_ms:Date.now()+3_600_000,interval_ms:0}}},
    {action:"update",id:item.id,revision:"1"},{action:"delete",id:item.id,revision:"0",prompt:"Unexpected"},
  ]) {
    const reply=await setup.plan(args);assert.equal(reply.result.isError,true);assert.equal(reply.result.error.code,"invalid_request");assert.equal(reply.call,undefined);
  }
  const stale=await setup.plan({action:"update",id:item.id,revision:"2",prompt:"Changed"});
  assert.equal(stale.result.error.code,"revision_conflict");assert.equal(stale.call,undefined);
  assert.equal(setup.values.size,1);assert.equal(setup.schedules.size,0);assert.equal(setup.transactions.length,0);
});

test("continuations preserve uncertainty and never claim success without a receipt",async()=>{
  const setup=await fixture(), reply=await setup.plan({action:"create",name:"Review",prompt:"Read",timing:{kind:"once",data:{at_ms:Date.now()+3_600_000}}});
  const unknown={isError:true,error:{code:"outcome_unknown",message:"Result unavailable"}};
  assert.equal((await setup.finish(reply,unknown)).result,unknown);
  const missing=await setup.finish(reply,{kind:"plugin_transaction",data:[]});
  assert.equal(missing.result.error.code,"outcome_unknown");assert.equal(setup.values.size,0);
});

test("filtered paging advances through foreign records without skipping owned tasks",async()=>{
  const setup=await fixture();
  for(let index=11;index<44;index++)setup.seed(setup.task(index,id(4)));
  setup.seed(setup.task(44));
  const first=(await setup.plan({action:"list"})).result;
  assert.equal(first.items.length,0);assert.equal(first.after,`task/${id(42)}`);
  const second=(await setup.plan({action:"list",after:first.after})).result;
  assert.deepEqual(plain(second.items).map(item=>item.id),[id(44)]);assert.equal(second.after,null);
});

test("weekly tasks preserve scope through create edit and delete",async()=>{
  const setup=await fixture(), timing={kind:"weekly",data:{timezone:"Asia/Shanghai",weekdays:[5,4,3,2,1],hour:9,minute:0}};
  const reply=await setup.plan({action:"create",name:"Weekday review",prompt:"Review yesterday",timing});
  const created=(await setup.commit(reply)).result;
  assert.deepEqual(plain(created.timing),{kind:"weekly",data:{...timing.data,weekdays:[1,2,3,4,5]}});
  assert.deepEqual(plain(created.config),setup.config);assert.equal(created.project,setup.project);assert.equal(created.worktree,setup.worktree);
  const update=await setup.plan({action:"update",id:created.id,revision:"1",name:"Renamed"});
  const edited=(await setup.commit(update)).result;
  assert.deepEqual(plain(edited.timing),plain(created.timing));assert.deepEqual(plain(edited.config),setup.config);
  const listed=(await setup.plan({action:"list"})).result.items;
  assert.deepEqual(plain(listed[0].timing),plain(created.timing));
  const removal=await setup.plan({action:"delete",id:created.id,revision:"2"});
  assert.equal((await setup.commit(removal)).result.removed,true);
  assert.equal(setup.schedules.size,0);assert.equal(setup.handlers.size,0);
});

test("invalid weekly fields fail before mutation preparation",async()=>{
  const setup=await fixture(), data={timezone:"UTC",weekdays:[1,2,3,4,5],hour:9,minute:0};
  for(const invalid of [
    {...data,timezone:undefined},{...data,timezone:""},{...data,timezone:" UTC "},
    {...data,weekdays:[]},{...data,weekdays:[1,1]},{...data,weekdays:[0]},{...data,weekdays:[8]},
    {...data,hour:24},{...data,minute:60},{...data,at_ms:Date.now()+3_600_000},
  ]) {
    const reply=await setup.plan({action:"create",name:"Review",prompt:"Read",timing:{kind:"weekly",data:invalid}});
    assert.equal(reply.result.error.code,"invalid_request");assert.equal(reply.call,undefined);
  }
  assert.equal(setup.values.size,0);assert.equal(setup.schedules.size,0);assert.equal(setup.handlers.size,0);
});

const intervals=[1,31_536_000_001,Number.MAX_SAFE_INTEGER];
const invalidIntervals=[0,-1,1.5,Number.MAX_SAFE_INTEGER+1];
test("recurring tasks preserve positive safe integer intervals",async()=>{
  for(const interval_ms of [...intervals,...invalidIntervals]) {
    const setup=await fixture(), timing={kind:"every",data:{anchor_ms:Date.now()+3_600_000,interval_ms}};
    const reply=await setup.plan({action:"create",name:"Review",prompt:"Read",timing});
    if(intervals.includes(interval_ms)) {
      const schedule=reply.call.arguments.operations.find(operation=>operation.data.kind==="save_schedule");
      assert.deepEqual(plain(schedule.data.data.timing),timing);
    } else {
      assert.equal(reply.result.error.code,"invalid_request");assert.equal(reply.call,undefined);
      assert.equal(setup.transactions.length,0);
    }
  }
});
runtimeTest("interval schemas retain integer types without duplicating handler bounds",async()=>{
  const manifest=JSON.parse(await readFile(new URL("../plugin.json",import.meta.url),"utf8"));
  const extension=manifest.extensions["dev.sailry.platform"];
  const rows=[...intervals,...invalidIntervals].map(interval_ms=>({action:"create",name:"Review",prompt:"Read",
    timing:{kind:"every",data:{anchor_ms:1000,interval_ms}}}));
  const expected=[...intervals,...invalidIntervals].map(value=>Number.isInteger(value)&&value>0);
  assert.deepEqual(validateSchema(extension.tools[0].handler.parameters,rows),expected);
  const schema=extension.storage.collections[0].schema.properties.timing;
  assert.deepEqual(validateSchema(schema,rows.map(row=>row.timing)),expected);
});

test("stored team schemas stay open while tool selectors stay strict",async()=>{
  const manifest=JSON.parse(await readFile(new URL("../plugin.json",import.meta.url),"utf8"));
  const extension=manifest.extensions["dev.sailry.platform"];
  const stored=extension.storage.collections[0].schema.properties.team;
  assert.notEqual(stored.additionalProperties,false);
  assert.notEqual(stored.unevaluatedProperties,false);
  assert.equal(extension.tools[0].handler.parameters.properties.team.additionalProperties,false);
});

runtimeTest("execution schemas preserve optional selectors and old records",async()=>{
  const manifest=JSON.parse(await readFile(new URL("../plugin.json",import.meta.url),"utf8"));
  const extension=manifest.extensions["dev.sailry.platform"], setup=await fixture();
  const schema=extension.tools[0].handler.parameters;
  const update={action:"update",id:id(11),revision:"1"};
  const rows=[{...update,execution:"model"},{...update,execution:"team"},{...update,team:{id:id(20)}},
    {...update,team:null},{...update,execution:"unknown"},{...update,execution:null},{...update,team:{id:"bad"}},
    {...update,team:{id:id(20),extra:true}}];
  assert.deepEqual(validateSchema(schema,rows),[true,true,true,true,false,false,false,false]);
  const stored=extension.storage.collections[0].schema;
  assert.deepEqual(validateSchema(stored,[setup.task(11),{...setup.task(12),execution:"model",team:null,models:{}},
    {...setup.task(13),execution:"team",team:{id:id(20),revision:null},models:{}},
    {...setup.task(14),execution:"invalid"},
    {...setup.task(15),execution:"team",team:{id:id(20),annotation:{label:"Saved metadata"}}}]),[true,true,true,false,true]);
});


test("remote edits preserve owner fields and reject stale listed revisions",async()=>{
  const setup=await fixture(),target=[2,...Array(31).fill(0)];
  const item={...setup.task(11),execution_node:target};setup.seed(item);
  let current={...item,revision:"4",name:"Owner name",prompt:"Owner prompt"};
  setup.host=async call=>({Ok:{data:{Ok:call.handler==="record"?plain(current):{...call.input,revision:"5"}}}});
  const listed=(await setup.plan({action:"list"})).result.items;
  assert.equal(listed[0].revision,"4");assert.equal(listed[0].name,"Owner name");
  const update=await setup.plan({action:"update",id:item.id,revision:"4",name:"Changed"});
  assert.equal(update.result.revision,"5");
  const saved=setup.hostCalls.find(call=>call.handler==="save").input;
  assert.equal(saved.prompt,"Owner prompt");assert.equal(saved.revision,"4");
  current={...current,revision:"6"};
  const stale=await setup.plan({action:"delete",id:item.id,revision:"4"});
  assert.equal(stale.result.error.code,"revision_conflict");
  assert.equal(setup.hostCalls.some(call=>call.handler==="remove"),false);
  assert.equal(setup.transactions.length,0);
});

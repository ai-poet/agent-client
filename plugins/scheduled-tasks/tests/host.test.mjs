import test from "node:test";
import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";
import vm from "node:vm";
import {id,plain,executionNode,otherNode} from "./store.mjs";

async function fixture(localNode=executionNode) {
  const records=(await readFile(new URL("../dev.sailry.platform/host/records.js",import.meta.url),"utf8"))
    .replace(/^import .*;\n/gm,"").replace(/^export /gm,"");
  const dispatch=(await readFile(new URL("../dev.sailry.platform/host/dispatch.js",import.meta.url),"utf8"))
    .replace(/^import .*;\n/gm,"").replace(/^export /gm,"");
  const mutations=(await readFile(new URL("../dev.sailry.platform/host/mutations.js",import.meta.url),"utf8"))
    .replace(/^import .*;\n/gm,"").replace(/^export /gm,"");
  const main=(await readFile(new URL("../dev.sailry.platform/host/management.js",import.meta.url),"utf8"))
    .replace(/^import .*;\n/gm,"").replace(/^export \{.*;\n/gm,"").replace(/^export /gm,"");
  const hostCalls=[];
  const requests=[],transactions=[],values=new Map(),pages=new Map(),config={provider:"provider",model:"selected",effort:"high",mode:"code",permission:"ask"};
  const teams=[{id:id(20),name:"Reviewers",revision:"7"}],handlers=[],reads=[],control={failure:null,beforeCommit:null,host:async()=>({Err:{code:"unavailable",message:"Offline"}})};
  const runtime={listTeams:async()=>teams,context:()=>({package:{name:"scheduled-tasks"},invocation:"00000000-0000-0000-0000-000000000001",execution_node:localNode}),
    getValue:async key=>{reads.push(key);return values.get(key)??{revision:"0",present:false,value:null};},listKeys:async(prefix,after,limit)=>{
      const keys=[...values.keys()].filter(key=>key.startsWith(prefix)&&(!after||key>after)).sort();
      return {keys:keys.slice(0,limit),after:keys.length>limit?keys[limit-1]:null};
    },
    readProjectCatalog:async()=>({projects:[],worktrees:[]}),
    prepareHostCall:(node,handler,input,readOnly=false)=>{const value={node,handler,input,readOnly};hostCalls.push(value);return {host:value};},
    prepareRequest:command=>{requests.push(command);return String(requests.length-1);},
    prepareTransaction:operations=>{transactions.push(operations);return "transaction";},
    completeRequest:async id=>{
      if(id?.host)return control.host(id.host);
      const command=requests[Number(id)];
      if(id==="transaction"&&transactions.at(-1)?.at(-1)?.data.kind==="enqueue") {
        if(control.failure)return {Err:control.failure};
        control.beforeCommit?.();
        const write=transactions.at(-1)[0].data,entry=values.get(write.key);
        if(entry?.revision!==write.expected_revision)return {Err:{code:"revision_conflict",message:"Task changed"}};
        values.set(write.key,{...entry,revision:String(BigInt(entry.revision)+1n),value:plain(write.value)});
        return {Ok:{data:[{kind:"plugin_value",data:values.get(write.key)},{kind:"dispatch",data:{kind:"handler",data:{}}},
          {kind:"dispatch",data:{kind:"job",data:{id:"new-job",handler:transactions.at(-1).at(-1).data.data.handler}}}]}};
      }
      if(command?.kind==="resolve_plugin_model")return control.resolve?.(command.data) ?? {Ok:{kind:"session_config",data:{...config}}};
      if(command?.data?.action.kind==="list_handlers"){reads.push("handlers");return {Ok:{data:{data:handlers}}};}
      if(command?.data?.action.kind==="list_jobs") return {Ok:{data:{data:pages.get(command.data.action.data.before)??{jobs:[],next_before:null}}}};
      if(command?.data?.action.kind==="read_result") return {Ok:{data:{data:{kind:"completed",data:{Ok:{kind:"queued_turn",data:{session:`session-${command.data.action.data.id}`,id:"turn"}}}}}}};
      if(command?.kind==="dispatch")return {Ok:{data:{data:[]}}};
      return {Ok:{data:[{data:{revision:"1"}}]}};
    },forgetRequest(){},
  };
  const api=vm.runInNewContext(`${records}\n${dispatch}\n${mutations}\n${main}\n({draft,changes,list,record,save,remove,run,cancel,stop,states,history})`,runtime);
  const input={execution_node:executionNode,id:"00000000-0000-0000-0000-000000000002",revision:"0",name:"Review",prompt:"Read the project",
    queue:"default",enabled:false,project:null,worktree:null,config:null,timing:{kind:"once",data:{at_ms:Date.now()+300000}}};
  return {api,input,config,teams,handlers,reads,control,values,pages,requests,transactions,hostCalls};
}
test("current host entry points reject missing models before dispatch or storage mutation",async()=>{
  const setup=await fixture(),{api,input}=setup;
  assert.equal((await api.save(input)).Err.code,"not_configured");
  await assert.rejects(api.changes(input),error=>error.code==="not_configured");
  const entry={revision:"4",present:true,value:input};setup.values.set(`task/${input.id}`,entry);
  assert.equal((await api.run({id:input.id})).Err.code,"not_configured");
  assert.equal(setup.values.get(`task/${input.id}`),entry);
  assert.deepEqual(setup.requests.map(command=>command.data.action.kind),[]);
  assert.equal(setup.transactions.length,0);
});

test("management lists tasks without a queue settings lookup",async()=>{
  const setup=await fixture(),listed=await setup.api.list({});
  assert.deepEqual(JSON.parse(JSON.stringify(listed)),{Ok:{items:[],after:null}});
  assert.deepEqual(setup.requests.map(command=>command.data.action.kind),["list_schedules"]);
});
test("native states span job pages and prioritize active runs over later completions",async()=>{
  const setup=await fixture();
  setup.pages.set(null,{jobs:[
    {handler:"repeat",status:"completed",started_ms:3},{handler:"finished",status:"failed",started_ms:4},
    {handler:"queued",status:"completed"},{handler:"unstarted",status:"queued"},
  ],next_before:21});
  setup.pages.set(21,{jobs:[
    {handler:"repeat",status:"running",started_ms:2},{handler:"finished",status:"completed"},
    {handler:"queued",status:"queued"},
  ],next_before:10});
  setup.pages.set(10,{jobs:[{handler:"repeat",status:"queued"}],next_before:null});
  assert.deepEqual(JSON.parse(JSON.stringify(await setup.api.states())),
    {repeat:{status:"running",started_ms:2},finished:{status:"failed",started_ms:4},
      queued:{status:"queued",started_ms:null},unstarted:{status:"queued",started_ms:null}});
  assert.deepEqual(JSON.parse(JSON.stringify(setup.requests.map(command=>command.data.action.data))),
    [{before:null,limit:100},{before:21,limit:100},{before:10,limit:100}]);
  assert.equal(setup.transactions.length,0);
});
test("task history scans native pages for one handler's runs",async()=>{
  const setup=await fixture(),handler=id(41),other=id(42),job=(id,handler)=>({id,handler,status:"completed",event:{payload:{name:"Review"}}});
  setup.pages.set(null,{jobs:[job("a1",handler),job("b1",other)],next_before:21});
  setup.pages.set(21,{jobs:[job("b2",other),job("a2",handler)],next_before:10});
  setup.pages.set(10,{jobs:[job("a3",handler)],next_before:null});
  const page=JSON.parse(JSON.stringify(await setup.api.history({before:null,handler,limit:2})));
  assert.deepEqual(page.Ok.items.map(item=>[item.job.id,item.session]),[["a1","session-a1"],["a2","session-a2"]]);
  assert.equal(page.Ok.next_before,10);
  const reads=setup.requests.map(command=>command.data.action).filter(action=>action.kind==="read_result").map(action=>action.data.id);
  assert.deepEqual(reads,["a1","a2"]);
  const rest=JSON.parse(JSON.stringify(await setup.api.history({before:10,handler,limit:2})));
  assert.deepEqual(rest.Ok.items.map(item=>item.job.id),["a3"]);assert.equal(rest.Ok.next_before,null);
});
test("new handlers capture Node-validated configuration without exporting or choosing defaults",async()=>{
  const setup=await fixture(),requested={...setup.config,effort:"low"};setup.input.config=requested;
  const saved=await setup.api.save(setup.input);
  assert.deepEqual(JSON.parse(JSON.stringify(saved.Ok.config)),setup.config);
  const resolve=setup.requests.find(command=>command.kind==="resolve_plugin_model");
  assert.equal(resolve.data.model,"provider/selected");assert.equal(resolve.data.effort,"low");assert.equal(resolve.data.config,requested);
  const handler=setup.transactions[0].find(operation=>operation.kind==="dispatch"&&operation.data.kind==="save_handler");
  assert.deepEqual(JSON.parse(JSON.stringify(handler.data.data.callback.command.data.config)),setup.config);
  assert.equal(handler.data.data.callback.command.kind,"start_session");
  assert.equal(handler.data.data.callback.command.data.project,null);
});

test("saved execution and task association share the session callback",async()=>{
  for(const selection of [{execution:"model",team:null,models:{}},
    {execution:"team",team:{id:id(20),revision:"7"},models:{[id(30)]:{provider:"provider",model:"lead",effort:"high"}}}]) {
    const setup=await fixture();Object.assign(setup.input,{config:setup.config,...selection});
    const saved=await setup.api.save(setup.input);
    assert.equal(saved.Ok.execution,selection.execution);
    const expected=selection.team===null?null:{id:selection.team.id,revision:null};
    assert.deepEqual(plain(saved.Ok.team),expected);assert.deepEqual(plain(saved.Ok.models),selection.models);
    const handler=setup.transactions[0].find(operation=>operation.data.kind==="save_handler").data.data;
    const start=handler.callback.command.data;
    assert.equal(handler.callback.completion,"turn");assert.equal(start.execution,selection.execution);
    assert.deepEqual(plain(start.team),expected);assert.deepEqual(plain(start.models),selection.models);
    assert.deepEqual(plain(start.association),{namespace:"scheduled-tasks",kind:"task",id:setup.input.id});
    assert.equal(start.project,null);assert.equal(start.worktree,null);
  }
});

test("old task records retain default team execution",async()=>{
  const setup=await fixture();setup.input.config=setup.config;
  const task=setup.api.draft(setup.input);
  assert.equal(task.execution,"team");assert.equal(task.team,null);assert.deepEqual(plain(task.models),{});
  const callback=(await setup.api.changes(task)).find(operation=>operation.data.kind==="save_handler").data.data.callback;
  assert.equal(callback.command.data.execution,"team");assert.equal(callback.command.data.team,null);
});

test("missing teams fail before preparing a transaction",async()=>{
  const setup=await fixture();Object.assign(setup.input,{config:setup.config,execution:"team",team:{id:id(21)}});
  assert.equal((await setup.api.save(setup.input)).Err.code,"not_found");
  assert.equal(setup.transactions.length,0);assert.equal(setup.values.size,0);
  assert.ok(setup.requests.every(request=>request.kind==="resolve_plugin_model"));
});

test("fixed teams resolve current lead defaults before validating stale fallback",async()=>{
  for(const action of ["save","run"]) {
    const setup=await fixture();
    const base={...setup.config,provider:"disabled-provider",model:"stale",effort:"low",options:{feature:{choice:true}}};
    setup.teams[0].model={provider:"lead-provider",model:"lead",effort:null};
    setup.control.resolve=data=>{
      assert.equal(data.model,"lead-provider/lead");
      return {Ok:{kind:"session_config",data:{...base,...data.config,provider:"lead-provider",model:"lead",effort:data.effort ?? "high"}}};
    };
    Object.assign(setup.input,{config:base,execution:"team",team:{id:id(20),revision:null}});
    if(action==="run") {
      setup.values.set(`task/${setup.input.id}`,{revision:"1",present:true,value:plain(setup.input)});
      setup.handlers.push({name:setup.input.id,revision:1});
    }
    const result=await setup.api[action](action==="save"?setup.input:{id:setup.input.id});
    assert.ok(result.Ok,JSON.stringify(result));
    const reads=setup.requests.filter(command=>command.kind==="resolve_plugin_model");
    assert.equal(reads.length,2);assert.equal(reads[0].data.config,null);assert.equal(reads[0].data.effort,null);
    assert.deepEqual(plain(reads[1].data.config),base);assert.equal(reads[1].data.effort,"high");
    const handler=setup.transactions[0].find(operation=>operation.data.kind==="save_handler").data.data;
    const selected=handler.callback.command.data.config;
    assert.equal(selected.provider,"lead-provider");assert.equal(selected.effort,"high");
    assert.deepEqual(plain(selected.options),base.options);assert.equal(base.provider,"disabled-provider");
  }
});

test("invalid execution fields fail before reading host resources",async()=>{
  for(const selection of [{execution:"unknown"},{execution:null},{models:[]},{models:null},
    {team:{id:"invalid"}},{team:{id:id(20),revision:7}},{team:{id:id(20),revision:"0"}},
    {execution:"model",team:{id:id(20)}},{execution:"model",models:{[id(30)]:{}}}]) {
    const setup=await fixture();Object.assign(setup.input,{config:setup.config,...selection});
    assert.equal((await setup.api.save(setup.input)).Err.code,"invalid_request");
    assert.equal(setup.requests.length,0);assert.equal(setup.transactions.length,0);
  }
});

test("manual runs refresh handlers and enqueue atomically without changing schedules",async()=>{
  const setup=await fixture(),item={...setup.input,config:setup.config};
  const entry={revision:"4",present:true,value:item};setup.values.set(`task/${item.id}`,entry);
  setup.handlers.push({name:item.id,revision:7});
  const reply=await setup.api.run({id:item.id});
  assert.deepEqual(plain(reply),{Ok:{id:"new-job",handler:item.id}});
  assert.deepEqual(setup.reads,[`task/${item.id}`,"handlers",`task/${item.id}`]);
  assert.deepEqual(setup.requests.map(command=>command.kind),["dispatch","resolve_plugin_model"]);
  assert.equal(setup.transactions.length,1);
  const operations=plain(setup.transactions[0]);
  assert.equal(operations[0].kind,"write");
  assert.equal(operations[0].data.expected_revision,"4");
  assert.deepEqual(operations[0].data.value,plain(setup.api.draft(item)));
  assert.deepEqual(operations.slice(1).map(operation=>operation.data.kind),["save_handler","enqueue"]);
  const registration=operations[1].data.data;
  assert.equal(registration.revision,"7");assert.equal(registration.callback.completion,"turn");
  assert.equal(registration.callback.command.data.execution,"team");
  assert.deepEqual(registration.callback.command.data.association,{namespace:"scheduled-tasks",kind:"task",id:item.id});
  assert.equal(setup.values.get(`task/${item.id}`).revision,"5");
  assert.equal(item.execution,undefined);assert.equal(item.config,setup.config);
});

test("handler conflicts and uncertain runs do not enqueue a separate retry",async()=>{
  for(const code of ["revision_conflict","outcome_unknown"]) {
    const setup=await fixture(),item={...setup.input,config:setup.config};
    setup.values.set(`task/${item.id}`,{revision:"4",present:true,value:item});
    setup.handlers.push({name:item.id,revision:7});setup.control.failure={code,message:"Injected transaction failure"};
    assert.equal((await setup.api.run({id:item.id})).Err.code,code);
    assert.equal(setup.transactions.length,1);
    assert.ok(setup.requests.every(command=>command.data?.action?.kind!=="enqueue"));
  }
});

test("foreign task controls stay on the recorded host",async()=>{
  const setup=await fixture();
  Object.assign(setup.input,{config:setup.config,execution_node:otherNode,revision:"1"});
  setup.values.set(`task/${setup.input.id}`,{revision:"1",present:true,value:plain(setup.input)});
  setup.control.host=async call=>({Ok:{data:{Ok:{handler:call.handler}}}});
  assert.equal((await setup.api.save({...setup.input,name:"Edited"})).Ok.handler,"save");
  assert.equal((await setup.api.remove({id:setup.input.id,revision:"1"})).Ok.handler,"remove");
  assert.equal((await setup.api.run({id:setup.input.id})).Ok.handler,"run");
  assert.equal((await setup.api.history({handler:setup.input.id})).Ok.handler,"history");
  assert.equal((await setup.api.cancel({id:"job",execution_node:otherNode})).Ok.handler,"cancel");
  assert.equal((await setup.api.stop({id:"job",execution_node:otherNode})).Ok.handler,"stop");
  assert.deepEqual(setup.hostCalls.map(call=>call.handler),["save","remove","run","history","cancel","stop"]);
  assert.ok(setup.hostCalls.every(call=>JSON.stringify(call.node)===JSON.stringify(otherNode)));
  assert.equal(setup.transactions.length,0);
  assert.equal(setup.requests.length,0);
});

test("offline tasks and unknown ownership never enqueue locally",async()=>{
  const setup=await fixture();
  Object.assign(setup.input,{config:setup.config,execution_node:otherNode,revision:"1"});
  setup.values.set(`task/${setup.input.id}`,{revision:"1",present:true,value:plain(setup.input)});
  assert.equal((await setup.api.run({id:setup.input.id})).Err.code,"unavailable");
  assert.equal((await setup.api.save({...setup.input,execution_node:executionNode})).Err.code,"conflict");
  delete setup.input.execution_node;
  setup.values.set(`task/${setup.input.id}`,{revision:"1",present:true,value:plain(setup.input)});
  assert.equal((await setup.api.save(setup.input)).Err.code,"unavailable");
  assert.equal(setup.transactions.length,0);
});

test("remote listing revisions match executor mutations and reject stale drafts",async()=>{
  const source=await fixture(),owner=await fixture(otherNode);
  const item={...source.input,config:source.config,execution_node:otherNode};
  source.values.set(`task/${item.id}`,{present:true,revision:"9",value:plain(item)});
  owner.values.set(`task/${item.id}`,{present:true,revision:"4",value:{...plain(item),name:"Current on owner"}});
  source.control.host=async call=>({Ok:{data:await owner.api[call.handler](call.input)}});
  owner.control.failure=null;
  const listed=(await source.api.list({})).Ok.items;
  assert.equal(listed.length,1);assert.equal(listed[0].revision,"4");assert.equal(listed[0].name,"Current on owner");
  const saved=await source.api.save({...listed[0],name:"Edited",expected_value:listed[0]});
  assert.ok(saved.Ok);assert.equal(owner.transactions.length,1);assert.equal(source.transactions.length,0);
  assert.equal(owner.transactions[0][0].data.expected_revision,"4");
  owner.values.set(`task/${item.id}`,{present:true,revision:"5",value:{...plain(item),name:"Edited"}});
  const stale=await source.api.remove({id:item.id,revision:listed[0].revision,expected_value:listed[0]});
  assert.equal(stale.Err.code,"revision_conflict");assert.equal(owner.transactions.length,1);
});

test("offline listings retain copies without writable revisions",async()=>{
  const setup=await fixture();
  setup.values.set(`task/${setup.input.id}`,{present:true,revision:"9",value:{...setup.input,config:setup.config,execution_node:otherNode}});
  const listed=(await setup.api.list({})).Ok.items;
  assert.equal(listed.length,1);assert.equal(listed[0].revision,null);assert.equal(listed[0].unavailable,true);
  assert.equal(setup.transactions.length,0);
});

test("matching content uses local task revisions across hosts",async()=>{
  for(const handler of ["save","remove"]) {
    const setup=await fixture(),original={...setup.input,config:setup.config};
    setup.values.set(`task/${original.id}`,{revision:"9",present:true,value:plain(original)});
    const input=handler==="save"?{...original,name:"Changed",revision:"2",expected_value:plain(original)}
      :{id:original.id,revision:"2",expected_value:plain(original)};
    const result=await setup.api[handler](input);
    assert.ok(result.Ok,JSON.stringify(result));
    assert.equal(setup.transactions.length,1);
    assert.equal(setup.transactions[0][0].data.expected_revision,"9");
    assert.equal(setup.transactions[0][0].kind,handler==="save"?"write":"remove");
  }
});

test("task content changes reject edits removal and manual runs",async()=>{
  for(const handler of ["save","remove","run"]) {
    const setup=await fixture(),original={...setup.input,config:setup.config};
    setup.values.set(`task/${original.id}`,{revision:"9",present:true,value:{...plain(original),prompt:"Concurrent edit"}});
    setup.handlers.push({name:original.id,revision:4});
    const input=handler==="save"?{...original,name:"Stale edit",revision:"2",expected_value:plain(original)}
      :{id:original.id,revision:"2",expected_value:plain(original)};
    assert.equal((await setup.api[handler](input)).Err.code,"revision_conflict");
    assert.equal(setup.transactions.length,0);
    assert.equal(setup.requests.length,0);
    assert.equal(setup.values.get(`task/${original.id}`).value.prompt,"Concurrent edit");
  }
});

test("forwarded task edits carry original content without projection fields",async()=>{
  const setup=await fixture(),original={...setup.input,config:setup.config,execution_node:otherNode};
  setup.values.set(`task/${original.id}`,{revision:"2",present:true,value:plain(original)});
  setup.control.host=async call=>({Ok:{data:{Ok:{handler:call.handler}}}});
  await setup.api.save({...original,name:"Changed",revision:"2",status:"running",next_ms:42});
  await setup.api.remove({id:original.id,revision:"2"});
  for(const call of setup.hostCalls) {
    assert.equal(call.input.expected_value.name,"Review");
    assert.equal(call.input.expected_value.revision,undefined);
    assert.equal(call.input.expected_value.status,undefined);
    assert.equal(call.input.expected_value.next_ms,undefined);
  }
});

test("manual run content changes fail the transaction fence",async()=>{
  const setup=await fixture(),original={...setup.input,config:setup.config};
  const key=`task/${original.id}`;
  setup.values.set(key,{revision:"9",present:true,value:plain(original)});
  setup.handlers.push({name:original.id,revision:4});
  setup.control.beforeCommit=()=>setup.values.set(key,{revision:"10",present:true,value:{...plain(original),prompt:"Raced"}});
  const result=await setup.api.run({id:original.id,revision:"2",expected_value:plain(original)});
  assert.equal(result.Err.code,"revision_conflict");
  assert.equal(setup.transactions.length,1);
  assert.equal(setup.transactions[0][0].data.expected_revision,"9");
  assert.equal(setup.values.get(key).value.prompt,"Raced");
  assert.equal(setup.values.get(key).revision,"10");
});

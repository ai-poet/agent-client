import test from "node:test";
import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";
import vm from "node:vm";
import {messages} from "../dev.sailry.platform/desktop/locales.js";

const source=(await readFile(new URL("../dev.sailry.platform/desktop/editor.js",import.meta.url),"utf8"))
  .replace(/^import[\s\S]*?;\n/gm,"").replace(/^export /gm,"");
const main=(await readFile(new URL("../dev.sailry.platform/desktop/main.js",import.meta.url),"utf8"))
  .replace(/^import .*;\n/gm,"").replace("export default class","class");
const node=[1,...Array(31).fill(0)],otherNode=[2,...Array(31).fill(0)];
const sameNode=(a,b)=>JSON.stringify(a)===JSON.stringify(b);
const plain=value=>JSON.parse(JSON.stringify(value));
const records=(await readFile(new URL("../dev.sailry.platform/host/records.js",import.meta.url),"utf8"))
  .replace(/^import .*;\n/gm,"").replace(/^export /gm,"");
const snapshot=vm.runInNewContext(`${records}\nsnapshot`);
const config={assistant:null,resource:null,provider:"provider",model:"explicit",effort:"high",mode:"plan",permission:"full",credential:null};
function fixture(resolve=async()=>config,outcome={Err:{code:"outcome_unknown"}}) {
  const nodes=new Map(),texts=new Map(),created=[],released=[],jobs=[],notices=[],requests=[],forgotten=[];
  function element(kind="div",id=null,props={}) {
    const target={kind,selector:id,props,items:[],style:{}};
    const proxy=new Proxy(target,{get(target,key) {
      if(key==="id")return id=>{target.selector=id;nodes.set(id,proxy);return proxy;};
      if(key==="child")return value=>{target.items.push(value);return proxy;};
      if(key==="children")return values=>{target.items.push(...values);return proxy;};
      if(key in target)return target[key];
      return (...args)=>{target.style[key]=args;return proxy;};
    }});
    if(id)nodes.set(id,proxy);return proxy;
  }
  const api=vm.runInNewContext(`${source}\n({editor,release,value,render,effortKey,setTiming,lead,leadEffort})`,{
    sameNode,snapshot,div:()=>element(),Button:function(id){return element("Button",id);},Field:function(){return element("Field");},VForm:function(){return element("Form");},
    ...Object.fromEntries(["TextField","DateTimeField","SelectField","ModelPopup","SegmentedTabs","Toggle"]
      .map(kind=>[kind,{new:(id,props)=>element(kind,id,props)}])),
    createText:(value,props)=>{const id=`text-${texts.size}`;texts.set(id,value);created.push(props);return id;},
    readText:id=>texts.get(id),releaseText:id=>released.push(id),createDateTime:value=>value,readDateTime:value=>value,
    releaseDateTime:value=>released.push(value),
  });
  const Controller=vm.runInNewContext(`${main}\nScheduledTasks`,{View:class{},effortKey:api.effortKey,setTiming:api.setTiming,value:api.value,
    editor:api.editor,lead:api.lead,sameNode,snapshot,
    resolveSessionModel:resolve,hostCall:async(_node,handler,input)=>handler==="model"?resolve(input.model,input.effort,input.config):{models:[],teams:[]},
    prepareHostCall:(node,handler,input)=>{requests.push({node,data:{handler,input}});return "original-request";},
    completeRequest:async()=>outcome,forgetRequest:id=>forgotten.push(id),toast:value=>notices.push(value)});
  const view=new Controller();Object.assign(view,{text:messages("en"),items:[],busy:false,pending:null,loading:false,modelsError:false,teamsError:false,teams:[{id:"reviewers",name:"Reviewers",revision:"7"}],
    executionNode:node,hosts:[{node,name:"Current",projects:[]},{node:otherNode,name:"Other",projects:[]}],
    catalog:{projects:[{id:"project",name:"Project",node},{id:"other",name:"Other",node}],worktrees:[{id:"tree",project:"project",path:"Branch",node},{id:"foreign",project:"other",path:"Other",node}]},
    models:[{id:"provider/explicit",kind:"provider",model:"explicit",efforts:["low","high"]}],editing:api.editor(null,messages("en")),
    close(){},refresh(){}});
  view.editing.draft.execution_node=node;
  const cx={notify(){},spawn:job=>jobs.push(job)};
  return {api,view,cx,nodes,texts,created,released,jobs,notices,requests,forgotten,
    render(){nodes.clear();return api.render(view);},async settle(){await Promise.all(jobs.splice(0).map(job=>job(cx)));}};
}
test("forms use vertical fields, full-width schedules and one model picker",()=>{
  const setup=fixture(),{view}=setup;setup.render();
  const form=setup.nodes.get("task-editor").items[0];
  assert.equal(form.kind,"Form");assert.deepEqual(plain(form.style.w_full),[]);
  assert.ok(form.items.every(field=>field.kind==="Field"));
  assert.deepEqual(setup.created.map(field=>field.label),["Name","Task","Interval (seconds)"]);
  assert.deepEqual(setup.created.map(field=>field.placeholder),["Name this task","What should run?","Seconds"]);
  assert.equal(view.editing.model,null);assert.equal(view.editing.draft.config,null);assert.equal(view.editing.draft.queue,"default");
  assert.equal(setup.nodes.get("task-timing").kind,"SegmentedTabs");
  assert.equal(setup.nodes.get("task-timing").props.fill,true);
  assert.deepEqual(plain(setup.nodes.get("task-timing").props.items),[{id:"once",label:"Once"},{id:"every",label:"Repeat"},{id:"weekly",label:"Weekly"}]);
  assert.equal(setup.nodes.get("task-project").kind,"SelectField");
  assert.equal(setup.nodes.get("task-model").kind,"ModelPopup");assert.equal(setup.nodes.get("task-model").props.selected,null);
  assert.equal(setup.nodes.get("task-strength"),undefined);
  assert.ok(!form.items.some(field=>field.style.label?.[0]==="Queue"));
  setup.texts.set(view.editing.name,"Review");setup.texts.set(view.editing.prompt,"Read the project");
  assert.throws(()=>setup.api.value(view.editing),/modelRequired/);
  setup.api.release(view.editing);assert.deepEqual(setup.released.slice(0,3),["text-0","text-1","text-2"]);assert.equal(setup.released.length,4);
});
test("weekly drafts preserve timezone days and time on rename",()=>{
  const setup=fixture(),{api,view}=setup;
  api.release(view.editing);
  const timing={kind:"weekly",data:{timezone:"Asia/Shanghai",weekdays:[1,2,3,4,5],hour:9,minute:0}};
  const item={execution_node:node,id:"saved",revision:"2",name:"Review",prompt:"Keep this task",queue:"default",project:"project",worktree:"tree",enabled:true,config:{...config},timing};
  view.editing=api.editor(item,view.text);setup.render();
  assert.equal(view.editing.time,null);assert.equal(view.editing.interval,null);
  assert.equal(setup.nodes.get("task-timing").props.selected,"weekly");
  for(const day of [1,2,3,4,5,6,7]) {
    const props=setup.nodes.get(`task-weekday-${day}`).props;
    assert.equal(props.checked,day<=5);assert.equal(props.variant,"button");assert.equal(props.label,view.text[`weekday${day}`]);
  }
  setup.texts.set(view.editing.name,"Renamed");
  assert.deepEqual(plain(api.value(view.editing)),{...item,name:"Renamed",execution:"team",team:null,models:{}});
  assert.deepEqual(item.timing,timing);assert.notEqual(view.editing.weekdays,timing.data.weekdays);
});

test("new drafts default to model execution and old drafts retain teams",()=>{
  const setup=fixture(),{view,api}=setup;
  assert.equal(view.editing.draft.execution,"model");assert.equal(view.editing.draft.team,null);
  assert.deepEqual(plain(view.editing.draft.models),{});
  const old={id:"saved",revision:"2",name:"Review",prompt:"Read",config:{...config},
    timing:{kind:"once",data:{at_ms:Date.now()+3_600_000}}};
  view.editing=api.editor(old,view.text);
  assert.equal(view.editing.draft.execution,"team");assert.equal(view.editing.draft.team,null);
  assert.deepEqual(plain(view.editing.draft.models),{});
  assert.equal(old.execution,undefined);
});

test("execution changes retain configuration and clear member overrides",()=>{
  const setup=fixture(),{view,cx}=setup, models={member:{provider:"provider",model:"member",effort:"high"}};
  Object.assign(view.editing.draft,{config,execution:"team",team:{id:"reviewers",revision:"7"},models});
  view.control({id:"task-team",value:"reviewers"},cx);
  assert.equal(view.editing.draft.models,models);
  view.control({id:"task-team",value:"unknown"},cx);
  assert.equal(view.editing.draft.models,models);
  view.control({id:"task-execution",value:"other"},cx);
  assert.equal(view.editing.draft.execution,"team");
  view.control({id:"task-team",value:"default"},cx);
  assert.equal(view.editing.draft.team,null);assert.deepEqual(plain(view.editing.draft.models),{});
  view.control({id:"task-team",value:"reviewers"},cx);
  assert.deepEqual(plain(view.editing.draft.team),{id:"reviewers",revision:null});
  view.control({id:"task-execution",value:"model"},cx);
  assert.equal(view.editing.draft.execution,"model");assert.equal(view.editing.draft.team,null);
  assert.deepEqual(plain(view.editing.draft.models),{});assert.equal(view.editing.draft.config,config);
  view.control({id:"task-team",value:"reviewers"},cx);
  assert.equal(view.editing.draft.team,null);
  view.control({id:"task-execution",value:"team"},cx);
  assert.equal(view.editing.draft.execution,"team");assert.equal(view.editing.draft.team,null);
  view.pending="original";view.control({id:"task-team",value:"reviewers"},cx);
  view.control({id:"task-execution",value:"model"},cx);
  assert.equal(view.editing.draft.execution,"team");assert.equal(view.editing.draft.team,null);
});

test("fixed leads resolve without a prior manual model selection",async()=>{
  const calls=[],fixed={provider:"lead-provider",model:"lead",effort:"low"};
  const setup=fixture(async(model,effort,base)=>{
    calls.push({model,effort,base});return {...config,...base,provider:fixed.provider,model:fixed.model,effort};
  });
  const {view,cx}=setup;
  view.teams[0].model=fixed;
  view.control({id:"task-execution",value:"team"},cx);
  view.control({id:"task-team",value:"reviewers"},cx);
  assert.equal(view.editing.draft.config,null);assert.equal(view.editing.modelLoading,true);
  assert.equal(setup.api.lead(view),fixed);
  view.control({id:"task-model",value:{model:"provider/explicit",effort:"high"}},cx);
  view.save(cx);assert.equal(setup.requests.length,0);assert.equal(setup.jobs.length,1);
  await setup.settle();
  assert.deepEqual(calls,[{model:"lead-provider/lead",effort:"low",base:null}]);
  assert.equal(view.editing.model,"lead-provider/lead");assert.equal(view.editing.draft.config.effort,"low");
  assert.deepEqual(plain(view.editing.draft.team),{id:"reviewers",revision:null});
  setup.texts.set(view.editing.name,"Review");setup.texts.set(view.editing.prompt,"Read");
  assert.equal(setup.api.value(view.editing).config.model,"lead");
});

test("fixed lead defaults come from model resolution and preserve policies",async()=>{
  const calls=[],base={...config,effort:"low",options:{feature:{choice:true}}};
  const setup=fixture(async(model,effort,config)=>{
    calls.push({model,effort,config});return {...base,...config,effort:effort ?? "high"};
  });
  const {view,cx}=setup;
  Object.assign(view.editing.draft,{config:base,project:"project",worktree:"tree"});
  view.editing.model="provider/explicit";
  view.teams[0].model={provider:"provider",model:"explicit",effort:null};
  view.control({id:"task-execution",value:"team"},cx);
  view.control({id:"task-team",value:"reviewers"},cx);
  await setup.settle();
  assert.equal(calls.length,2);assert.equal(calls[0].config,null);assert.equal(calls[0].effort,null);
  assert.equal(calls[1].config,base);assert.equal(calls[1].effort,"high");
  assert.equal(view.editing.draft.config.effort,"high");
  assert.equal(setup.api.leadEffort(view.editing,view.teams[0].model),"high");
  assert.equal(view.editing.draft.config.mode,"plan");assert.equal(view.editing.draft.config.permission,"full");
  assert.deepEqual(view.editing.draft.config.options,base.options);
  assert.equal(view.editing.draft.project,"project");assert.equal(view.editing.draft.worktree,"tree");
  assert.equal(base.effort,"low");
  view.syncLead(cx);assert.equal(setup.jobs.length,0);
  view.teams[0].model=null;view.syncLead(cx);
  view.control({id:"task-model",value:{model:"provider/explicit",effort:"low"}},cx);await setup.settle();
  assert.equal(view.editing.draft.config.effort,"low");
});

test("team changes discard stale lead replies without replacing drafts",async()=>{
  const replies=[],setup=fixture((model,effort,base)=>new Promise(resolve=>replies.push({model,effort,base,resolve})));
  const {view,cx}=setup;
  view.teams[0].model={provider:"provider",model:"first",effort:"low"};
  view.teams.push({id:"other-team",name:"Other",model:{provider:"provider",model:"second",effort:"high"}});
  view.editing.draft.config=config;view.editing.model="provider/explicit";
  const draft=view.editing.draft;
  view.control({id:"task-execution",value:"team"},cx);
  view.control({id:"task-team",value:"reviewers"},cx);const first=setup.jobs.shift()(cx);
  view.control({id:"task-team",value:"other-team"},cx);const second=setup.jobs.shift()(cx);
  replies[0].resolve({...config,model:"first",effort:"low"});await first;
  assert.equal(view.editing.draft.config,config);
  replies[1].resolve({...config,model:"second",effort:"high"});await second;
  assert.equal(view.editing.draft.config.model,"second");
  view.teams[1].model={provider:"provider",model:"updated",effort:"low"};view.syncLead(cx);
  const updated=setup.jobs.shift()(cx),retained=view.editing.draft.config;
  view.teams=[];view.syncLead(cx);
  replies[2].resolve({...config,model:"updated",effort:"low"});await updated;
  assert.equal(view.editing.draft,draft);assert.equal(draft.config,retained);
  assert.equal(draft.team.id,"other-team");assert.equal(setup.api.lead(view),null);
  assert.equal(view.editing.modelLoading,false);assert.equal(setup.notices.length,0);
});

test("failed lead previews retain configuration and retry on refresh",async()=>{
  let failed=true;
  const setup=fixture(async()=>{if(failed)throw new Error("Unavailable");return {...config,model:"lead"};});
  const {view,cx}=setup;
  view.editing.draft.config=config;view.editing.model="provider/explicit";
  view.teams[0].model={provider:"provider",model:"lead",effort:"high"};
  view.control({id:"task-execution",value:"team"},cx);
  view.control({id:"task-team",value:"reviewers"},cx);await setup.settle();
  assert.equal(view.editing.draft.config,config);assert.equal(view.editing.leadError,true);
  assert.equal(setup.notices.at(-1).message,"Could not select model");
  failed=false;view.syncLead(cx);await setup.settle();
  assert.equal(view.editing.draft.config.model,"lead");assert.equal(view.editing.leadError,false);
  view.control({id:"task-execution",value:"model"},cx);
  assert.equal(view.editing.leadError,false);assert.equal(setup.api.lead(view),null);
});
test("timing switches retain drafts and require an explicit timezone",()=>{
  const setup=fixture(),{view,api}=setup;
  setup.texts.set(view.editing.name,"Review");setup.texts.set(view.editing.prompt,"Read the project");
  view.editing.draft.config={...config};view.editing.model="provider/explicit";
  const start=view.editing.time;
  view.control({id:"task-timing",value:"weekly"},setup.cx);setup.render();
  assert.equal(view.editing.time,null);assert.equal(view.editing.interval,null);
  assert.equal(setup.texts.get(view.editing.timezone),"");
  assert.throws(()=>api.value(view.editing),/timezoneRequired/);
  setup.texts.set(view.editing.timezone,"Asia/Shanghai");setup.texts.set(view.editing.wallTime,"8:15");
  view.control({id:"task-weekday-2",value:false},setup.cx);
  view.control({id:"task-weekday-6",value:true},setup.cx);
  const weekly={kind:"weekly",data:{timezone:"Asia/Shanghai",weekdays:[1,3,4,5,6],hour:8,minute:15}};
  assert.deepEqual(plain(api.value(view.editing).timing),weekly);
  view.control({id:"task-timing",value:"every"},setup.cx);
  assert.equal(view.editing.time,start);assert.equal(view.editing.timezone,null);
  assert.equal(api.value(view.editing).timing.kind,"every");
  view.control({id:"task-timing",value:"weekly"},setup.cx);
  assert.deepEqual(plain(api.value(view.editing).timing),weekly);
  setup.texts.set(view.editing.wallTime,"24:00");assert.throws(()=>api.value(view.editing),/invalidClock/);
  setup.texts.set(view.editing.wallTime,"09:00");view.editing.weekdays=[];
  assert.throws(()=>api.value(view.editing),/weekdaysRequired/);
});
test("model changes preserve queues and policies; effort choices update only configuration",async()=>{
  const calls=[],setup=fixture(async(model,effort,base)=>{calls.push({model,effort,base});return {...config,effort:effort??"high"};});
  const item={...setup.view.editing.draft,id:"saved",revision:"3",name:"Review",prompt:"Keep this",queue:"reviews",config:{...config},project:"removed",worktree:"missing"};
  setup.view.editing=setup.api.editor(item,setup.view.text);setup.render();
  assert.equal(setup.nodes.get("task-project").props.selected,"removed");assert.equal(setup.nodes.get("task-project").props.items.at(-1).label,"Unavailable");
  assert.equal(setup.nodes.get("task-worktree").props.selected,"missing");
  setup.view.control({id:"task-model",value:{model:"provider/explicit",effort:"low"}},setup.cx);assert.equal(setup.view.editing.modelLoading,true);
  setup.render();assert.deepEqual(plain(setup.nodes.get("task-save").style.disabled),[false]);
  assert.equal(setup.nodes.get("task-model").props.disabled,false);
  await setup.settle();const saved=setup.api.value(setup.view.editing);
  assert.equal(saved.queue,"reviews");assert.equal(saved.config.effort,"low");assert.equal(saved.config.mode,"plan");assert.equal(saved.config.permission,"full");
  assert.equal(calls.length,1);assert.equal(calls[0].base,item.config);
  assert.equal(item.config.effort,"high");
  setup.view.control({id:"task-project",value:"project"},setup.cx);assert.equal(setup.view.editing.draft.worktree,null);
  setup.view.control({id:"task-worktree",value:"foreign"},setup.cx);assert.equal(setup.view.editing.draft.worktree,null);
  setup.view.control({id:"task-worktree",value:"tree"},setup.cx);assert.equal(setup.view.editing.draft.worktree,"tree");
  setup.view.pending="original";setup.render();assert.equal(setup.nodes.get("task-project").props.disabled,true);
  setup.view.control({id:"task-project",value:"none"},setup.cx);assert.equal(setup.view.editing.draft.project,"project");
});
test("the model picker carries raw named and numeric efforts in every locale",()=>{
  for(const locale of ["en","zh-CN"]) {
    const setup=fixture(),values=["none","minimal","low","medium","high","xhigh","max",{budget:-1},{budget:1024}];
    setup.view.text=messages(locale);
    setup.view.models[0].efforts=["default",...values];
    setup.view.editing.model="provider/explicit";
    for(const effort of ["default",...values]) {
      setup.view.editing.draft.config={...config,effort};setup.render();
      assert.equal(setup.nodes.get("task-strength"),undefined);
      assert.equal(setup.nodes.get("task-model").props.effort,effort);
    }
  }
});
test("model choices wait for delayed models and supported effort to resolve together",async()=>{
  const calls=[];let reply;
  const setup=fixture((model,effort,base)=>{
    calls.push({model,effort,base});return new Promise(resolve=>{reply=resolve;});
  });
  const item={...setup.view.editing.draft,id:"saved",revision:"3",name:"Review",prompt:"Keep this",queue:"reviews",config:{...config}};
  setup.view.editing=setup.api.editor(item,setup.view.text);
  setup.view.models.push({id:"provider/other",kind:"provider",model:"other",efforts:["low"]});
  setup.view.control({id:"task-model",value:{model:"provider/other",effort:"low"}},setup.cx);
  const pending=setup.jobs.shift()(setup.cx);
  setup.render();
  const choice=setup.nodes.get("task-model");
  assert.equal(choice.props.selected,"provider/explicit");
  assert.equal(choice.props.effort,"high");
  assert.equal(choice.props.disabled,false);
  assert.equal(setup.view.editing.draft.config,item.config);
  assert.equal(calls.length,1);assert.equal(calls[0].model,"provider/other");assert.equal(calls[0].effort,"low");
  reply({...calls[0].base,model:"other",effort:"low"});await pending;
  setup.render();
  assert.equal(setup.nodes.get("task-model").props.disabled,false);
  assert.equal(setup.nodes.get("task-model").props.selected,"provider/other");
  assert.equal(setup.nodes.get("task-model").props.effort,"low");
  const saved=setup.api.value(setup.view.editing);
  assert.equal(saved.queue,"reviews");assert.equal(saved.config.mode,"plan");assert.equal(saved.config.permission,"full");
  assert.equal(item.config.model,"explicit");assert.equal(item.config.effort,"high");
  assert.equal(setup.notices.length,0);
});
test("failed or stale model replies preserve drafts without defaults or editor replacement",async()=>{
  const replies=[],setup=fixture(()=>new Promise((resolve,reject)=>replies.push({resolve,reject}))),original=setup.view.editing;
  setup.view.control({id:"task-model",value:{model:"provider/explicit",effort:"high"}},setup.cx);
  const first=setup.jobs.shift()(setup.cx);replies[0].reject(new Error("unavailable"));await first;
  assert.equal(original.draft.config,null);assert.equal(original.model,null);assert.equal(original.modelLoading,false);assert.equal(setup.notices[0].message,"Could not select model");
  setup.view.control({id:"task-model",value:{model:"provider/explicit",effort:"low"}},setup.cx);const stale=setup.jobs.shift()(setup.cx);
  setup.view.editing=setup.api.editor(null,setup.view.text);replies[1].resolve(config);await stale;
  assert.equal(setup.view.editing.draft.config,null);assert.equal(setup.view.editing.model,null);
  setup.view.editing=original;
  setup.view.control({id:"task-model",value:{model:"provider/explicit",effort:"low"}},setup.cx);const earlier=setup.jobs.shift()(setup.cx);
  setup.view.control({id:"task-model",value:{model:"provider/explicit",effort:"high"}},setup.cx);const latest=setup.jobs.shift()(setup.cx);
  replies[3].resolve(config);await latest;replies[2].resolve({...config,effort:"low"});await earlier;
  assert.equal(original.draft.config.effort,"high");assert.equal(original.modelLoading,false);
});
test("pending model reads keep the committed choice and accept a newer selection",async()=>{
  const replies=[],setup=fixture(()=>new Promise(resolve=>replies.push(resolve)));
  const item={...setup.view.editing.draft,id:"saved",revision:"3",name:"Review",prompt:"Keep this",config:{...config}};
  setup.view.editing=setup.api.editor(item,setup.view.text);
  setup.view.models.push({id:"provider/other",kind:"provider",model:"other",efforts:["low","high"]});
  setup.view.control({id:"task-model",value:{model:"provider/other",effort:"low"}},setup.cx);
  const earlier=setup.jobs.shift()(setup.cx);setup.render();
  assert.equal(setup.nodes.get("task-model").props.selected,"provider/explicit");
  assert.equal(setup.nodes.get("task-model").props.disabled,false);
  setup.view.control({id:"task-model",value:{model:"provider/other",effort:"high"}},setup.cx);
  const latest=setup.jobs.shift()(setup.cx);
  replies[1]({...config,model:"other",effort:"high"});await latest;
  replies[0]({...config,model:"other",effort:"low"});await earlier;
  assert.equal(setup.view.editing.model,"provider/other");
  assert.equal(setup.view.editing.draft.config.effort,"high");
});
test("save freezes the committed model before an uncertain request",async()=>{
  let reply;
  const setup=fixture(()=>new Promise(resolve=>{reply=resolve;}));
  const item={...setup.view.editing.draft,id:"saved",revision:"3",name:"Review",prompt:"Keep this",config:{...config}};
  setup.view.editing=setup.api.editor(item,setup.view.text);
  setup.view.models.push({id:"provider/other",kind:"provider",model:"other",efforts:["low"]});
  setup.view.control({id:"task-model",value:{model:"provider/other",effort:"low"}},setup.cx);
  const model=setup.jobs.shift()(setup.cx);
  assert.equal(setup.view.editing.modelLoading,true);
  setup.view.save(setup.cx);
  await setup.jobs.shift()(setup.cx);
  assert.equal(setup.view.pending,"original-request");
  assert.equal(setup.view.editing.modelLoading,false);
  assert.equal(setup.requests.length,1);
  assert.equal(setup.requests[0].data.input.config.model,"explicit");
  assert.equal(setup.requests[0].data.input.config.effort,"high");
  reply({...config,model:"other",effort:"low"});await model;
  assert.equal(setup.view.editing.model,"provider/explicit");
  assert.equal(setup.view.editing.draft.config.model,"explicit");
  assert.equal(setup.view.editing.draft.config.effort,"high");
  setup.view.save(setup.cx);await setup.jobs.shift()(setup.cx);
  assert.equal(setup.requests.length,1);
  assert.equal(setup.requests[0].data.input.config.model,"explicit");
});

test("repeat editors preserve representable millisecond intervals",()=>{
  const setup=fixture(),{view,api}=setup;
  setup.texts.set(view.editing.name,"Review");setup.texts.set(view.editing.prompt,"Read");
  view.editing.draft.config={...config};view.editing.model="provider/explicit";
  view.editing.kind="every";
  for(const [seconds,interval_ms] of [["0.001",1],["31536000.001",31_536_000_001],["9007199254740",9_007_199_254_740_000]]) {
    setup.texts.set(view.editing.interval,seconds);
    assert.equal(api.value(view.editing).timing.data.interval_ms,interval_ms);
  }
  for(const seconds of ["0","-1","Infinity","NaN","9007199254741"]) {
    setup.texts.set(view.editing.interval,seconds);
    assert.throws(()=>api.value(view.editing),/invalidInterval/);
  }
});

test("host changes clear foreign project and model choices",async()=>{
  const setup=fixture(),{view}=setup;
  view.editing.draft.project="project";view.editing.draft.worktree="tree";view.editing.draft.config={...config};
  view.control({id:"task-host",value:JSON.stringify(otherNode)},setup.cx);
  assert.deepEqual(plain(view.editing.draft.execution_node),otherNode);
  assert.equal(view.editing.draft.project,null);assert.equal(view.editing.draft.worktree,null);
  assert.equal(view.editing.draft.config,null);assert.equal(view.editing.model,null);
  await setup.settle();
  view.editing.draft.id="existing";view.control({id:"task-host",value:JSON.stringify(node)},setup.cx);
  assert.deepEqual(plain(view.editing.draft.execution_node),otherNode);
  setup.render();assert.equal(setup.nodes.get("task-host").props.disabled,true);
});

test("edits keep original content across source refresh and version drift",async()=>{
  const setup=fixture(),item={...setup.view.editing.draft,execution_node:otherNode,id:"saved",revision:"3",
    name:"Original",prompt:"Keep this",config:{...config}};
  setup.view.editing=setup.api.editor(item,setup.view.text);
  const original=plain(setup.view.editing.original);
  setup.view.items=[{...item,name:"Concurrent change",revision:"8"}];
  item.config.effort="low";
  setup.texts.set(setup.view.editing.name,"My edit");
  setup.view.save(setup.cx);await setup.jobs.shift()(setup.cx);
  assert.deepEqual(plain(setup.requests[0].data.input.expected_value),original);
  assert.equal(setup.requests[0].data.input.expected_value.config.effort,"high");
  assert.equal(setup.requests[0].data.input.name,"My edit");
  assert.equal(setup.requests[0].data.input.revision,"3");
  assert.deepEqual(plain(setup.requests[0].node),otherNode);
});

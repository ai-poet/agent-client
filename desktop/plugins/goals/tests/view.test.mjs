import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import {element,find,actions} from './elements.mjs';

const context=vm.createContext({});
const exports={
  'gpui-kit':{div:()=>element()},'gpui-component':{Button:function(id){return element(id);}},
  sailry:{theme:()=>({colors:{muted_foreground:'muted',success:'success',warning:'warning'}})},
  'sailry/ui':{ScrollRegion:{new:element}},
};
const modules=new Map();
async function load(name) {
  if (modules.has(name)) return modules.get(name);
  const module=exports[name] ? new vm.SyntheticModule(Object.keys(exports[name]),function(){
    for (const [key,value] of Object.entries(exports[name])) this.setExport(key,value);
  },{context}) : new vm.SourceTextModule(await readFile(new URL(name,new URL('../dev.sailry.platform/desktop/',import.meta.url)),'utf8'),{context,identifier:name});
  modules.set(name,module);
  if (!exports[name]) await module.link(load);
  return module;
}
const main=await load('./view.js');await main.evaluate();
const {render,stateLine,counters}=main.namespace;
const {messages}=(await (async()=>{const module=await load('./locales.js');await module.evaluate();return module;})()).namespace;

const goal=(overrides={})=>({id:'goal',revision:'5',objective:'Ship it',status:'active',state:'running',reason:null,detail:null,claim:null,evidence:null,
  turns:14,automatic_turns:12,turn_limit:25,tokens:340000,token_budget:0,elapsed_ms:12*60000,time_limit_ms:0,...overrides});
function view(value=goal(),locale='en') {
  return {goal:value,history:[],text:messages(locale),scope:{locale},state:{connected:true,readonly:false,mode:'code'},
    busy:false,loading:false,pending:null,draft:()=>'',perform(){},execute(){}};
}

test('the panel shows the objective, the state, its reason and the counters',()=>{
  const panel=render(view(goal({objective:'First 中文 🙂\r\n\n  Second  ',status:'paused',state:'paused',reason:'turns',automatic_turns:25})));
  assert.equal(panel.identity,'goal-panel');
  assert.deepEqual(find(panel,'goal-objective').text,['First 中文 🙂\r\n\n  Second  ']);
  assert.deepEqual(find(panel,'goal-state').text,['Paused · Turn limit reached']);
  assert.deepEqual(find(panel,'goal-counters').items.map(item=>item.text[0]),['Turns 25 / 25','Time 12m','Tokens 340.0k']);
  assert.equal(find(panel,'goal-note'),null);
});

test('states and reasons are named in the window language',()=>{
  const lines=[['running',null,'Running','执行中'],['waiting',null,'Waiting','等待中'],['verifying',null,'Verifying','核验中'],
    ['paused','stopped','Paused · Stopped','已暂停 · 已停止'],['paused','user','Paused · Paused by you','已暂停 · 已手动暂停'],
    ['paused','tokens','Paused · Token budget reached','已暂停 · 已达 Token 预算'],['paused','time','Paused · Time limit reached','已暂停 · 已达时间上限'],
    ['paused','no_progress','Paused · No progress','已暂停 · 无进展'],['paused','needs_you','Paused · Needs you','已暂停 · 需要处理'],
    ['paused','error','Paused · Error','已暂停 · 出错'],['paused','restart','Paused · App restarted','已暂停 · 应用已重启'],
    ['paused','plan','Paused · Plan mode','已暂停 · 计划模式'],['paused','restored','Paused · Restored from history','已暂停 · 已从历史恢复'],
    ['blocked',null,'Blocked','受阻'],['completed',null,'Completed','已完成']];
  for (const [state,reason,en,zh] of lines) {
    assert.equal(stateLine({state,reason},'en'),en);
    assert.equal(stateLine({state,reason},'zh-CN'),zh);
    // A locale the package does not ship falls back to English.
    assert.equal(stateLine({state,reason},'fr'),en);
  }
  assert.deepEqual([...counters(goal({token_budget:2000000,time_limit_ms:3600000}),messages('zh-CN'))],['轮次 12 / 25','用时 12m / 1h 0m','Token 340.0k / 2.0M']);
});

test('each state offers its own controls',()=>{
  // Two buttons in every state: Pause or Resume, then End. Reopen is the
  // first button of a completed goal only.
  const cases=[[{status:'active',state:'running'},['goal-pause','goal-end'],'Pause','暂停'],
    [{status:'active',state:'waiting'},['goal-pause','goal-end'],'Pause','暂停'],
    [{status:'paused',state:'paused',reason:'stopped'},['goal-resume','goal-end'],'Resume','继续'],
    [{status:'blocked',state:'blocked',detail:'Need the key'},['goal-resume','goal-end'],'Resume','继续'],
    [{status:'completed',state:'completed',evidence:'Suite passed'},['goal-resume','goal-end'],'Reopen','重新打开']];
  for (const [overrides,ids,first,zh] of cases) {
    const panel=render(view(goal(overrides)));
    assert.deepEqual(actions(panel),ids);
    assert.deepEqual([find(panel,ids[0]).labelValue,find(panel,'goal-end').labelValue],[first,'End']);
    const local=render(view(goal(overrides),'zh-CN'));
    assert.deepEqual([find(local,ids[0]).labelValue,find(local,'goal-end').labelValue],[zh,'结束']);
  }
  for (const locale of ['en','zh-CN']) {
    const text=messages(locale);
    for (const gone of ['edit','edited','save','cancel','clear','removed','hint']) assert.equal(text[gone],undefined,gone);
  }
  assert.deepEqual([messages('en').ended,messages('zh-CN').ended],['Goal ended','目标已结束']);
  const blocked=render(view(goal({status:'blocked',state:'blocked',detail:'Need the key'})));
  assert.deepEqual(find(blocked,'goal-note-text').text,['Need the key']);assert.equal(find(blocked,'goal-note-label'),null);
  const done=render(view(goal({status:'completed',state:'completed',evidence:'Suite passed'})));
  assert.deepEqual(find(done,'goal-note-label').text,['Evidence']);assert.deepEqual(find(done,'goal-note-text').text,['Suite passed']);
});

test('without a goal the panel shows the draft or the empty text',()=>{
  const setup=view(null);
  assert.deepEqual(find(render(setup),'goal-empty').text,['No goal']);
  assert.deepEqual(actions(render(setup)),[]);
  setup.draft=()=>'/literal content';
  assert.deepEqual(find(render(setup),'goal-empty').text,['/literal content']);
});

test('earlier goals list their outcome',()=>{
  const setup=view();
  setup.history=[{id:'a',objective:'Earlier one',outcome:'completed',elapsed_ms:65*60000},{id:'b',objective:'Earlier two',outcome:'replaced',elapsed_ms:0},
    {id:'c',objective:'Earlier three',outcome:'cleared',elapsed_ms:5*60000}];
  const panel=render(setup);
  assert.deepEqual(find(panel,'goal-earlier-title').text,['Earlier goals']);
  assert.deepEqual(['a','b','c'].map(id=>[find(panel,`goal-earlier-text-${id}`).text[0],find(panel,`goal-earlier-outcome-${id}`).text[0]]),
    [['Earlier one','Completed · 1h 5m'],['Earlier two','Replaced · <1m'],['Earlier three','Ended · 5m']]);
  setup.text=messages('zh-CN');
  assert.deepEqual(find(render(setup),'goal-earlier-outcome-c').text,['已结束 · 5m']);
  assert.equal(find(render(view()),'goal-earlier'),null);
});

test('refresh and a pending action change the controls',()=>{
  const setup=view();
  setup.loading=true;
  assert.ok(find(render(setup),'goal-actions').items.every(item=>item.disabledValue===true));
  setup.loading=false;setup.pending={id:'request'};
  assert.deepEqual(actions(render(setup)),['goal-retry','goal-pause','goal-end']);
  assert.equal(find(render(setup),'goal-retry').disabledValue,false);
  assert.equal(find(render(setup),'goal-pause').disabledValue,true);
  setup.pending=null;setup.state.readonly=true;
  assert.ok(find(render(setup),'goal-actions').items.every(item=>item.disabledValue===true));
});

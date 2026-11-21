import assert from 'node:assert/strict';
import test from 'node:test';
import vm from 'node:vm';
import {readFileSync} from 'node:fs';
import {Controller} from '../dev.sailry.platform/desktop/controller.js';
import {messages} from '../dev.sailry.platform/desktop/locales.js';
import {navigationLabel} from '../dev.sailry.platform/desktop/policy.js';

const source=readFileSync(new URL('../dev.sailry.platform/desktop/main.js',import.meta.url),'utf8')
  .replace(/^import[\s\S]*?;\n/gm,'').replace('export default class Worktrees','class Worktrees');
function fixture(location,locale='en') {
  const published=[],tasks=[],events=[],menuEvents=[],changes=[],subscriptions=[];
  const names=['selectLocation','readWorktreeCatalog','listWorktrees','inspectGit','resolveGitRevision',
    'prepareWorktreeChange','prepareGitChange','completeRequest','forgetRequest','newId','worktreeHasDrafts',
    'releaseWorktree','toast','nextLocationChange','nextContributionEvent','nextPickerEvent','modal_closed',
    'nextTextEvent','nextToastEvent','createText','readText','releaseText','focusText','dismissToast'];
  const bindings=Object.fromEntries(names.map(name=>[name,()=>{}]));
  const Worktrees=vm.runInNewContext(`${source}\nWorktrees`,{
    ...bindings,View:class {},Controller,messages,navigationLabel,render:()=>{},context:()=>JSON.stringify({locale}),
    readLocation:()=>location,publishContributions:states=>published.push(states),
    nextContributionEvent:async()=>{if(events.length)return events.shift();throw Error('Closed');},
    nextMenuEvent:async()=>{if(menuEvents.length)return menuEvents.shift();throw Error('Closed');},
    nextChange:async(cursor,scope)=>{
      subscriptions.push({cursor,scope});
      if(changes.length)return changes.shift();
      throw Error('Closed');
    },
  });
  const view=new Worktrees();
  const context={spawn:task=>tasks.push(task),notify:()=>{}};
  view.init({},context);
  tasks[0](context);
  return {view,published,tasks,events,menuEvents,changes,subscriptions,context};
}
const base={project:'project',project_name:'Project',worktree:'main',branch:'main',main:true,
  can_move:true,can_retarget:true,connected:true,git:true,session:null,projects:[]};

test('menu declaration keeps management as the semantic action',()=>{
  const manifest=JSON.parse(readFileSync(new URL('../plugin.json',import.meta.url),'utf8'));
  const entries=manifest.extensions['dev.sailry.platform'].ui;
  const location=entries.find(entry=>entry.id==='location');
  assert.equal(location.kind,'menu');
  assert.equal(location.handler,'location');
  assert.equal(location.intent,undefined);
  assert.equal(entries.find(entry=>entry.id==='manage').intent,'worktrees');
});

test('initialization keeps declared labels for unrepresentable names',()=>{
  for (const main of [false,true]) for (const name of [null,undefined,'','   ','\u2003','line\nbreak',
    '\u0085','a'.repeat(129),'中文'.repeat(22),'🙂'.repeat(33)]) {
    const location={...base,main,project_name:name,branch:name};
    const {view,published}=fixture(location);
    assert.equal(published.length,1);
    assert.equal(published[0][0].label,undefined);
    assert.equal(view.model.location,location);
    assert.equal(location[main ? 'project_name' : 'branch'],name);
    assert.deepEqual(Array.from(published[0],state=>state.id),['location','manage','create','fork']);
  }
});

test('menu state publishes its choices and selection',()=>{
  const {view,published}=fixture({...base,project:null,projects:[{id:'project',name:'Project'}]});
  const initial=published[0][0];
  assert.equal(initial.value,'unassigned');
  assert.deepEqual(Array.from(initial.choices,choice=>choice.id),['unassigned','project:project','add_project']);
  assert.deepEqual(initial.choices[0].group,initial.choices[1].group);
  assert.notDeepEqual(initial.choices[1].group,initial.choices[2].group);
  view.model.accept({...base,worktree:'main'});
  assert.equal(view.model.selected(),'project:project');
});

test('choice events select and keep their replies',async()=>{
  const {view,published,tasks,events,context}=fixture(base);
  const selected=[],opened=[];
  view.model.select=async id=>selected.push(id);
  view.model.open=async handler=>opened.push(handler);
  events.push({id:'location',handler:'location',kind:'change',value:'add_project',sequence:7},
    {id:'manage',handler:'manage',kind:'invoke',sequence:8});
  await tasks[3](context);
  await Promise.all(tasks.slice(7).map(task=>task(context)));
  assert.deepEqual(selected,['add_project']);
  assert.deepEqual(opened,['manage']);
  assert.equal(published[1][0].reply_to,7);
  assert.equal(published[2][1].reply_to,8);
});

test('inventory subscriptions refresh without moving the location',async()=>{
  const location={...base,cursor:'location'};
  const {view,tasks,changes,subscriptions,context}=fixture(location);
  let reads=0;
  view.model.invalidate=async()=>{reads++;};
  changes.push({cursor:'first'},{cursor:'second'});
  await tasks[1](context);
  await Promise.all(tasks.slice(7).map(task=>task(context)));
  assert.equal(reads,2);
  assert.deepEqual(subscriptions,[
    {cursor:undefined,scope:'package'},{cursor:'first',scope:'package'},{cursor:'second',scope:'package'},
  ]);
  assert.equal(view.model.location,location);
});

test('large inventories stay inside the choice limit',()=>{
  const projects=Array.from({length:1000},(_,index)=>({id:`project-${index}`,name:`Project ${index}`}));
  const {view,published}=fixture({...base,project:null,projects});
  const choices=published[0][0].choices;
  assert.ok(choices.length<=256);
  assert.equal(choices.at(-2).id,'projects');
  view.publish({id:'location',sequence:7});
  assert.equal(published.at(-1)[0].reply_to,7);
});

test('valid UTF-8 labels and captured replies remain intact',()=>{
  for (const name of ['a'.repeat(128),'中文'.repeat(21),'🙂'.repeat(32),'e\u0301','\ufeff']) {
    const {view,published}=fixture({...base,main:false,project_name:name});
    assert.equal(published[0][0].label.label,name);
    view.publish({id:'location',sequence:7});
    assert.equal(published[1][0].reply_to,7);
    assert.equal(published[1][1].reply_to,undefined);
    assert.equal(published[1][0].enabled,true);
  }
});

test('empty unassigned locations disable their popup',()=>{
  const {view,published}=fixture({...base,project:null,session:null,can_retarget:false});
  assert.deepEqual(view.model.choices(),[]);
  assert.equal(published[0][0].enabled,false);
});

test('chat labels and disabled context preserve scope',()=>{
  for (const [locale,label] of Object.entries({
    en:'Chats','zh-CN':'对话','zh-TW':'對話',de:'Chats',es:'Chats',fr:'Chats',
    ja:'チャット',ko:'대화','pt-BR':'Conversas',ru:'Чаты',
  })) {
    const {published}=fixture({...base,project:null,session:'session',can_move:false},locale);
    assert.equal(published[0][0].label.label,label,locale);
    assert.equal(published[0][0].enabled,false);
    assert.equal(published[0][1].visible,false);
    assert.equal(published[0][2].visible,false);
    assert.equal(published[0][3].visible,false);
  }
});

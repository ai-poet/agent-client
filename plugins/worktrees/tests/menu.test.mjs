import test from 'node:test';
import assert from 'node:assert/strict';
import {menu,fit,navigationLabel,MENU_PROJECTS} from '../dev.sailry.platform/desktop/policy.js';
import {messages} from '../dev.sailry.platform/desktop/locales.js';

const strings=messages('en'),text=key=>strings[key];
const location={project:'project',worktree:'main',main:true,branch:'main',session:null,git:true,can_retarget:true,
  project_name:'Project',projects:[{id:'project',name:'Project'},{id:'other',name:'Other'}]};

test('project menus list chats, projects and the add action',()=>{
  const choices=menu(location,text);
  assert.deepEqual(choices.map(choice=>choice.id),['unassigned','project:project','project:other','add_project']);
  assert.equal(choices[0].label.label,'Chats');
  assert.deepEqual(choices.slice(0,3).map(choice=>choice.icon),[undefined,undefined,undefined]);
  assert.equal(choices[3].icon,'plus');
  assert.deepEqual(choices[0].group,choices[2].group);
  assert.notDeepEqual(choices[2].group,choices[3].group);
  assert.ok(!choices.some(choice=>choice.id.startsWith('worktree:')));
});

test('fixed projects offer no menu',()=>{
  assert.deepEqual(menu({...location,can_retarget:false},text),[]);
  assert.deepEqual(menu({...location,project:null,session:'session',can_retarget:false},text),[]);
});

test('control characters preserve valid selectable project labels',()=>{
  const projects=[{id:'newline',name:'one\ntwo'},{id:'tab',name:'one\ttwo'},{id:'controls',name:'\u0000\u0085'}];
  const choices=menu({...location,projects},text).filter(choice=>choice.id.startsWith('project:'));
  assert.deepEqual(choices.map(choice=>choice.id),['project:newline','project:tab','project:controls']);
  assert.deepEqual(choices.map(choice=>choice.label.label),['one two','one two',text('location_project')]);
  for (const choice of choices) assert.deepEqual(navigationLabel(choice.label.label),choice.label);
  assert.deepEqual(projects.map(project=>project.name),['one\ntwo','one\ttwo','\u0000\u0085']);
});

test('large hosts keep the current project and open the picker',()=>{
  const projects=Array.from({length:MENU_PROJECTS+50},(_,index)=>({id:`p${index}`,name:`Project ${index}`}));
  const choices=menu({...location,project:`p${MENU_PROJECTS+10}`,projects},text);
  assert.equal(choices.filter(choice=>choice.id.startsWith('project:')).length,MENU_PROJECTS+1);
  assert.ok(choices.some(choice=>choice.id===`project:p${MENU_PROJECTS+10}`));
  assert.equal(choices.at(-2).id,'projects');
  assert.ok(choices.length<=256);
});

test('long names fit the label limit',()=>{
  assert.equal(fit('a'.repeat(128)),'a'.repeat(128));
  for (const name of ['a'.repeat(129),'中文'.repeat(30),'🙂'.repeat(40)]) {
    const label=fit(name);
    assert.ok(new TextEncoder().encode(label).length<=128,name);
    assert.ok(label.endsWith('…'));
  }
  const choices=menu({...location,projects:[{id:'project',name:'a'.repeat(200)},{id:'blank',name:'  '}]},text);
  assert.ok(choices[1].label.label.endsWith('…'));
  assert.ok(!choices.some(choice=>choice.id==='project:blank'));
});

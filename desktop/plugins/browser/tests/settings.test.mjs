import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFile} from 'node:fs/promises';
import {messages,errorKey,imported} from '../dev.sailry.platform/desktop/locales.js';

async function fixture(importProfile, listProfiles = async()=>[]) {
  const source=(await readFile(new URL('../dev.sailry.platform/desktop/settings.js',import.meta.url),'utf8'))
    .replace(/^import[\s\S]*?;\n/gm,'').replace('export default class','class');
  const notices=[],tasks=[],nodes=new Map(),pending=()=>new Promise(()=>{});
  function element(kind='div',id=null,props={}) {
    const value={kind,id,props,items:[]};
    const proxy=new Proxy(value,{get(target,key) {
      if(key==='id')return id=>{target.id=id;nodes.set(id,proxy);return proxy;};
      if(key==='child')return child=>{target.items.push(child);return proxy;};
      if(key==='children')return children=>{target.items.push(...children);return proxy;};
      if(key in target)return target[key];
      return (...args)=>{target.props[key]=args.length===1?args[0]:args;return proxy;};
    }});
    if(id)nodes.set(id,proxy);
    return proxy;
  }
  const Controller=vm.runInNewContext(`${source}\nSettings`,{View:class{},context:()=>'{"locale":"en"}',messages,errorKey,imported,
    readBrowserSettings:()=>({supported:true,enabled:true,persistent:false}),nextControlEvent:pending,modal_closed:pending,
    importBrowserProfile:importProfile,listBrowserProfiles:listProfiles,toast:value=>notices.push(value),
    div:()=>element(),theme:()=>({colors:{muted_foreground:'muted'}}),
    Button:function(id){return element('Button',id);},Spinner:function(){return element('Spinner');},
    SettingsGroup:{new:(id,props)=>element('SettingsGroup',id,props)},Modal:{new:(id,props)=>element('Modal',id,props)},
    Toggle:{new:(id,props)=>element('Toggle',id,props)}});
  const owner=new Controller(),cx={notify(){},spawn:work=>tasks.push(work(cx))};owner.init({},cx);
  return {owner,cx,notices,nodes,act:async work=>{const before=tasks.length;work();await Promise.all(tasks.slice(before));}};
}

test('failed imports toast once and keep the captured profile picker',async()=>{
  const setup=await fixture(async()=>{throw new Error('Import unavailable');}),profiles=[{id:'captured',name:'Fixture'}];setup.owner.profiles=profiles;
  await setup.act(()=>setup.owner.import(profiles[0],setup.cx));
  assert.equal(setup.owner.profiles,profiles);assert.equal(Object.hasOwn(setup.owner,'message'),false);assert.equal(setup.notices.length,1);assert.equal(setup.notices[0].kind,'error');
  assert.equal(setup.owner.busy,false);assert.equal(setup.owner.importing,null);
});

test('successful imports close their picker and report only through a toast',async()=>{
  const setup=await fixture(async()=>({count:2,skipped:0}));setup.owner.profiles=[{id:'captured',name:'Fixture'}];
  await setup.act(()=>setup.owner.import(setup.owner.profiles[0],setup.cx));
  assert.equal(setup.owner.profiles,null);assert.equal(Object.hasOwn(setup.owner,'message'),false);assert.equal(setup.notices.length,1);assert.equal(setup.notices[0].kind,'info');
  assert.equal(setup.notices[0].message,imported(messages('en'),2,0));
});

test('scanning multiple profiles opens the picker without importing',async()=>{
  const profiles=[{id:'captured',name:'Fixture'},{id:'other',name:'Other'}];
  const setup=await fixture(null,async()=>profiles);
  await setup.act(()=>setup.owner.scan(setup.cx));
  assert.equal(setup.owner.profiles,profiles);assert.equal(setup.owner.busy,false);assert.equal(setup.notices.length,0);
  setup.owner.render();
  const modal=setup.nodes.get('browser-profiles-dialog');
  assert.deepEqual(JSON.parse(JSON.stringify(modal.props)),{open:true,form:true,title:'Choose a profile',width:428});
  assert.equal(modal.items.length,1);
  assert.equal(setup.nodes.has('browser-profiles-close'),false);
});

test('scanning one profile imports once without opening a picker',async()=>{
  const profiles=[{id:'captured',name:'Fixture'}],calls=[];
  const setup=await fixture(async id=>{calls.push(id);return {count:3,skipped:1};},async()=>profiles);
  await setup.act(()=>{setup.owner.scan(setup.cx);setup.owner.scan(setup.cx);});
  assert.deepEqual(calls,['captured']);assert.equal(setup.owner.profiles,null);assert.equal(setup.owner.busy,false);
  assert.equal(setup.notices.length,1);assert.equal(setup.notices[0].message,imported(messages('en'),3,1));
});

test('cancelled import retains the picker without a toast',async()=>{
  const setup=await fixture(async()=>null),profiles=[{id:'captured',name:'Fixture'}];setup.owner.profiles=profiles;
  await setup.act(()=>setup.owner.import(profiles[0],setup.cx));
  assert.equal(setup.owner.profiles,profiles);assert.equal(setup.owner.busy,false);assert.equal(setup.owner.importing,null);
  assert.equal(setup.notices.length,0);
});

test('an empty scan reports a missing profile once',async()=>{
  const setup=await fixture(null);
  await setup.act(()=>setup.owner.scan(setup.cx));
  assert.equal(setup.owner.profiles,null);assert.equal(setup.owner.busy,false);assert.equal(setup.notices.length,1);
  assert.equal(setup.notices[0].message,messages('en').browser_chrome_missing);
});

test('cancelled folder selection is silent and releases the scan',async()=>{
  const setup=await fixture(null,async()=>null);
  await setup.act(()=>setup.owner.scan(setup.cx));
  assert.equal(setup.owner.profiles,null);assert.equal(setup.owner.busy,false);assert.equal(setup.notices.length,0);
});

test('denied access is not reported as a missing profile',async()=>{
  const setup=await fixture(null,async()=>{throw new Error('browser_chrome_access_denied');});
  await setup.act(()=>setup.owner.scan(setup.cx));
  assert.equal(setup.owner.busy,false);assert.equal(setup.notices.length,1);
  assert.equal(setup.notices[0].message,messages('en').browser_chrome_access_denied);
});

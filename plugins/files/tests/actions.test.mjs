import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const source = (await Promise.all(['actions','explorer','locales'].map(name =>
  readFile(new URL(`../dev.sailry.platform/desktop/${name}.js`,import.meta.url),'utf8'))))
  .map(value => value.replace(/^import .*;\n/gm,'').replace(/^export /gm,''))
  .join('\n');
const revealed = [];
const {Actions,Explorer,messages} = vm.runInNewContext(`${source}\n({Actions,Explorer,messages})`,{
  revealSystem:async path => { revealed.push(path); },
});
const plain = value => JSON.parse(JSON.stringify(value));

function fixture(count = 3) {
  const explorer = new Explorer(error => assert.fail(error));
  const entries = Array.from({length:count},(_,index) => ({name:`file-${index}.txt`,kind:'file'}));
  explorer.pages.set('',{entries,index:new Map(entries.map(entry => [entry.name,entry]))});
  const view = {explorer,canInsert:false,text:{},transfers:{canPaste:() => false}};
  return {view,actions:new Actions(view)};
}

function item(actions,path,action) {
  return actions.treeMenu()(path).find(item => actions.entries.get(item.id)?.action === action);
}

test('managed descendants retain all existing manual menu operations', () => {
  const {view,actions} = fixture(0);
  const base = '.sailry/worktrees/id', path = `${base}/source.txt`;
  const entry = {name:'source.txt',kind:'file'};
  view.explorer.pages.set(base,{entries:[entry],index:new Map([[entry.name,entry]])});
  view.explorer.expanded.add(base);
  view.transfers.canPaste = () => true;
  const menu = actions.tree(path,[]);
  const operations = menu.filter(item => !item.separator).map(item => actions.entries.get(item.id));
  for (const action of ['copy','cut','paste','rename','download','trash','new_file','new_directory']) {
    const operation = operations.find(operation => operation.action === action);
    assert.ok(operation,action);
    assert.equal(operation.target.directory,base);
    assert.deepEqual(plain(operation.target.paths),[path]);
    assert.equal(menu.find(item => actions.entries.get(item.id) === operation).enabled,true);
  }
});

test('large directory renders reuse prepared menu targets', () => {
  const {view,actions} = fixture(1_100);
  let builds = 0;
  const build = actions.tree.bind(actions);
  actions.tree = (...args) => { builds++; return build(...args); };
  actions.prepareTree();
  assert.equal(builds,1_100);
  const first = actions.treeMenu()('file-0.txt');
  for (let render = 0; render < 3; render++) {
    const rows = view.explorer.items({},actions.treeMenu());
    assert.equal(rows.length,1_100);
    assert.equal(rows[0].menu,first);
    assert.equal(rows[1_099].menu.length,first.length);
  }
  assert.equal(builds,1_100);
  assert.deepEqual(plain(actions.entries.get(item(actions,'file-0.txt','copy').id)),{
    action:'copy',target:{paths:['file-0.txt'],directory:''},
  });
});

test('selection refresh preserves earlier captured actions', () => {
  const {view,actions} = fixture();
  actions.prepareTree();
  const captured = item(actions,'file-0.txt','copy').id;
  view.explorer.select('file-1.txt');
  view.explorer.select('file-2.txt',{shift:true});
  actions.prepareTree();
  assert.equal(actions.treeMenu()('file-1.txt'),actions.treeMenu()('file-2.txt'));
  assert.deepEqual(plain(actions.entries.get(item(actions,'file-1.txt','copy').id)),{
    action:'copy',target:{paths:['file-1.txt','file-2.txt'],directory:''},
  });
  assert.deepEqual(plain(actions.entries.get(captured)),{
    action:'copy',target:{paths:['file-0.txt'],directory:''},
  });
});

test('menu refresh follows access and clipboard state', () => {
  const {view,actions} = fixture();
  actions.prepareTree();
  assert.equal(item(actions,'file-0.txt','paste').enabled,false);
  assert.equal(item(actions,'file-0.txt','insert').enabled,false);
  view.canInsert = true;
  view.transfers.canPaste = () => true;
  actions.prepareTree();
  assert.equal(item(actions,'file-0.txt','paste').enabled,true);
  assert.equal(item(actions,'file-0.txt','insert').enabled,true);
  view.explorer.connected = false;
  actions.prepareTree();
  assert.equal(item(actions,'file-0.txt','paste').enabled,false);
  assert.equal(item(actions,'file-0.txt','trash').enabled,false);
});

test('native reveal follows local single-item availability', () => {
  const {view,actions} = fixture();
  view.text = messages('en');
  const reveal = () => actions.tree('file-0.txt',[])
    .find(item => actions.entries.get(item.id)?.action === 'reveal_system');
  view.fileManager = {available:false,platform:'macos'};
  assert.equal(reveal(),undefined);
  for (const [platform,label] of [['macos','Show in Finder'],['windows','Show in Explorer'],['linux','Show in file manager']]) {
    view.fileManager = {available:true,platform};
    assert.equal(reveal().label,label);
  }
  view.fileManager.platform = 'macos';
  view.text = messages('zh-CN');
  assert.equal(reveal().label,'在访达中显示');
  view.explorer.select('file-0.txt');
  view.explorer.select('file-1.txt',{additive:true});
  const multiple = actions.tree('file-0.txt',view.explorer.paths());
  assert.equal(multiple.some(item => actions.entries.get(item.id)?.action === 'reveal_system'),false);
  view.explorer.connected = false;
  assert.equal(reveal().enabled,false);
});

test('native reveal retains the original selected item', async () => {
  const {view,actions} = fixture();
  view.fileManager = {available:true,platform:'macos'};
  actions.prepareTree();
  const captured = item(actions,'file-0.txt','reveal_system').id;
  view.explorer.select('file-1.txt');
  actions.prepareTree();
  await actions.invoke(captured,{});
  assert.equal(revealed.at(-1),'file-0.txt');
});


test('system opens overlap and retain each result',async()=>{
  const started=[],pending=new Map();
  const {Actions}=vm.runInNewContext(`${source}\n({Actions})`,{
    openSystem:path=>{
      started.push(path);
      return new Promise((resolve,reject)=>pending.set(path,{resolve,reject}));
    },
  });
  const actions=new Actions({text:{}});
  const item=actions.item('open_system','files_open_system',{paths:['one.txt','two.txt']});
  let settled=false;
  const opened=actions.invoke(item.id,{}).finally(()=>{settled=true;});
  assert.deepEqual(started,['one.txt','two.txt']);
  pending.get('one.txt').reject(Error('open failed'));
  await Promise.resolve();await Promise.resolve();
  assert.equal(settled,false,'the other requested open remains observable');
  pending.get('two.txt').resolve();
  await assert.rejects(opened,/open failed/);
  assert.deepEqual(started,['one.txt','two.txt']);
});

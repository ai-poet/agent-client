import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFile} from 'node:fs/promises';
import {messages} from '../dev.sailry.platform/desktop/locales.js';

const source=(await readFile(new URL('../dev.sailry.platform/desktop/files.js',import.meta.url),'utf8'))
  .replace(/^import .*;\n/gm,'').replace(/^export /gm,'');
const plain=value=>JSON.parse(JSON.stringify(value));
const directory=(path,entries=[],next=null)=>({kind:'directory',path,entries,next});
const entry=(name,kind='file')=>({name,kind});
function deferred(){let resolve;const promise=new Promise(accept=>{resolve=accept;});return {promise,resolve};}
function fixture(){const requests=[],reports=[],texts=new Map(),host={read:async({path})=>directory(path)},cx={notify(){}};
  const {Files}=vm.runInNewContext(`${source}\n({Files})`,{browseSshDirectory:async request=>{requests.push(plain(request));return host.read(request);},createText:value=>{texts.set('path',value);return 'path';},setText:(id,value)=>texts.set(id,value),releaseText:id=>texts.delete(id)});
  const text=messages('en'),files=new Files({id:'remote',revision:7},(error,key)=>reports.push({error,key}),text);
  return {files,host,requests,reports,texts,text,cx,items:()=>plain(files.items(path=>[{id:`menu:${path}`}],text))};}

test('breadcrumbs resolve absolute and relative ancestor paths',()=>{
  const {files}=fixture();
  for(const [path,expected]of[
    ['/',[{label:'/',path:'/'}]],
    ['/home/remote/project',[{label:'/',path:'/'},{label:'home',path:'/home'},{label:'remote',path:'/home/remote'},{label:'project',path:'/home/remote/project'}]],
    ['./project/src',[{label:'.',path:'.'},{label:'project',path:'./project'},{label:'src',path:'./project/src'}]],
  ]){files.path=path;assert.deepEqual(plain(files.segments()),expected);}
});

test('directory branches load on expansion and retain cached children',async()=>{
  const setup=fixture(),{files,host,cx,requests}=setup;
  host.read=async({path})=>directory(path,path==='/'?[entry('project','directory'),entry('readme')]:[entry('source')]);
  await files.load('/',cx);assert.equal(requests.length,1);
  const initial=setup.items();assert.equal(initial[0].expanded,false);assert.equal(initial[0].children[0].icon,'loader-circle');
  await files.expand('/project',cx);
  assert.equal(files.path,'/');assert.equal(requests.length,2);assert.equal(files.entry('/project/source').name,'source');
  assert.deepEqual(setup.items()[0].children,[{id:'/project/source',label:'source',icon:'file',menu:[{id:'menu:/project/source'}]}]);assert.equal(setup.items()[0].folder,true);
  files.expanded.delete('/project');await files.expand('/project',cx);assert.equal(requests.length,2,'reopening a loaded branch uses its own cache');
  assert.deepEqual(requests[1],{profile:'remote',expected_revision:7,path:'/project',after:null});
});

test('branch placeholders distinguish loading failure empty and pagination',async()=>{
  const setup=fixture(),{files,host,cx,text}=setup,held=deferred();
  host.read=async({path})=>path==='/'?directory('/',[entry('folder','directory')]):held.promise;
  await files.load('/',cx);const loading=files.expand('/folder',cx);
  assert.equal(setup.items()[0].children[0].label,text.ssh_files_loading);
  held.resolve(directory('/folder'));await loading;
  assert.deepEqual(setup.items()[0].children,[],'an empty folder opens with no placeholder row');assert.equal(setup.items()[0].folder,true);
  host.read=async()=>{throw Error('read failed');};await files.loadBranch('/folder',cx);
  const failed=setup.items()[0].children[0];assert.equal(failed.label,text.ssh_files_failed);assert.equal(files.placeholder(failed.id).kind,'retry');
  host.read=async()=>directory('/folder',[entry('first')],'page-2');await files.loadBranch('/folder',cx);
  const more=setup.items()[0].children.at(-1);assert.equal(more.label,text.files_load_more);assert.equal(files.placeholder(more.id).path,'/folder');
  host.read=async request=>{assert.equal(request.after,'page-2');return directory('/folder',[entry('second')]);};await files.loadBranch('/folder',cx,true);
  assert.deepEqual(setup.items()[0].children.map(item=>item.id),['/folder/first','/folder/second']);
});

test('root pagination preserves existing entries and branch state',async()=>{
  const {files,host,cx,requests}=fixture();
  host.read=async()=>directory('/',[entry('folder','directory')],'root-page');await files.load('/',cx);
  host.read=async()=>directory('/folder',[entry('nested')]);await files.expand('/folder',cx);
  host.read=async request=>{assert.equal(request.after,'root-page');return directory('/',[entry('last')]);};await files.load('/',cx,true);
  assert.deepEqual(Array.from(files.visible()),['/folder','/folder/nested','/last']);assert.equal(requests.length,3);
});

test('shift and additive selection follow visible expanded rows',async()=>{
  const {files,host,cx}=fixture();host.read=async({path})=>directory(path,path==='/'?[entry('folder','directory'),entry('last')]:[entry('a'),entry('b')]);
  await files.load('/',cx);await files.expand('/folder',cx);
  files.select('/folder/a',{});files.select('/last',{shift:true});
  assert.deepEqual(Array.from(files.selected),['/folder/a','/folder/b','/last']);
  files.select('/folder',{additive:true});files.select('/folder/b',{shift:true,additive:true});
  assert.deepEqual(Array.from(files.selected),['/folder/a','/folder/b','/last','/folder']);
  files.expanded.delete('/folder');assert.deepEqual(Array.from(files.visible()),['/folder','/last']);
});

test('root pagination retains an in flight directory expansion',async()=>{
  const setup=fixture(),{files,host,cx}=setup,held=deferred();
  host.read=async()=>directory('/',[entry('folder','directory')],'root-page');await files.load('/',cx);
  host.read=async({path})=>path==='/folder'?held.promise:directory('/',[entry('last')]);
  const expanding=files.expand('/folder',cx);await files.load('/',cx,true);
  held.resolve(directory('/folder',[entry('nested')]));await expanding;
  assert.deepEqual(Array.from(files.visible()),['/folder','/folder/nested','/last']);
  assert.equal(setup.items()[0].children[0].id,'/folder/nested');
  assert.equal(files.directories.get('/folder').loaded,true);
});

test('refresh reloads expanded directories and suppresses obsolete reads',async()=>{
  const {files,host,cx,requests}=fixture();host.read=async({path})=>directory(path,path==='/'?[entry('folder','directory')]:[entry('old')]);
  await files.load('/',cx);await files.expand('/folder',cx);
  host.read=async({path})=>directory(path,path==='/'?[entry('folder','directory')]:[entry('new')]);await files.load('/',cx);
  assert.equal(files.entry('/folder/old'),undefined);assert.equal(files.entry('/folder/new').name,'new');assert.equal(requests.length,4);
  const held=deferred();host.read=async({path})=>path==='/folder'?held.promise:directory('/elsewhere');
  const stale=files.loadBranch('/folder',cx);await files.load('/elsewhere',cx);
  held.resolve(directory('/folder',[entry('late')]));await stale;
  assert.equal(files.path,'/elsewhere');assert.equal(files.entry('/folder/late'),undefined);
});

test('untrusted fingerprints keep their specific error and never enter branches',async()=>{
  for(const changed of [false,true]){const setup=fixture(),{files,host,cx,reports,text}=setup;
    host.read=async()=>({kind:'host_key_required',changed});await files.load('/',cx);
    const key=changed?'ssh_key_changed':'ssh_key_unknown';assert.equal(files.error,key);assert.equal(reports[0].key,key);assert.equal(setup.items()[0].label,text[key]);
    host.read=async()=>directory('/',[entry('folder','directory')]);await files.load('/',cx);
    host.read=async()=>({kind:'host_key_required',changed});await files.expand('/folder',cx);
    assert.equal(setup.items()[0].children[0].label,text[key]);assert.equal(reports.at(-1).key,key);
  }
});

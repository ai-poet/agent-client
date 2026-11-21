import assert from 'node:assert/strict';
import test from 'node:test';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import {Element,flatten} from '../../tests/elements.mjs';
import {Controller} from '../dev.sailry.platform/desktop/controller.js';
import {messages} from '../dev.sailry.platform/desktop/locales.js';

const content=revision=>({artifact:{id:'artifact',title:'Captured document',revision,kind:'document'}});
const source=name=>readFileSync(new URL(`../dev.sailry.platform/desktop/${name}.js`,import.meta.url),'utf8')
  .replace(/^import .*;\n/gm,'');
const render=vm.runInNewContext(`${source('view').replace(/^export /gm,'')}\nrender`,{
  div:()=>new Element(),Header:Element,IconButton:Element,Loading:Element,ArtifactSurface:Element,
});
const idle=()=>new Promise(()=>{});
const settle=()=>new Promise(done=>setImmediate(done));

function fixture(read,exported=async()=>true) {
  const reads=[],waiting=[],queued=[],exports=[],headerWaiting=[],headerQueued=[],feedback=[];
  const next=()=>queued.length?Promise.resolve(queued.shift()):new Promise(done=>waiting.push(done));
  const artifacts={
    artifactResource:()=>({id:'artifact',session:'session'}),
    readArtifact:(id,revision)=>{reads.push(revision);return read(id,revision);},
    listArtifactRevisions:async()=>({items:[{revision:2},{revision:1}],next_before:null}),
    nextArtifactChange:idle,nextArtifactEvent:next,
    exportArtifact:value=>{exports.push(value);return exported(value);},
  };
  const Brief=vm.runInNewContext(`${source('main').replace('export default class Brief','class Brief')}\nBrief`,{
    View:class{},Controller,messages,render,artifacts,context:()=>JSON.stringify({locale:'en'}),
    header_action:()=>headerQueued.length?Promise.resolve(headerQueued.shift()):new Promise(done=>headerWaiting.push(done)),
    nextControlEvent:idle,toast:value=>feedback.push(value),
  });
  const view=new Brief(),errors=[];
  const cx={notify(){},spawn:callback=>callback(cx).catch(error=>errors.push(error))};
  view.init({},cx);
  return {view,reads,errors,exports,feedback,
    send(event){const resolve=waiting.shift();if(resolve)resolve(event);else queued.push(event);},
    header(event){const raw=typeof event==='string'?event:JSON.stringify(event),resolve=headerWaiting.shift();if(resolve)resolve(raw);else headerQueued.push(raw);},
  };
}

function header(view) {
  const element=flatten(view.render()).find(element=>element.props.id==='brief-header');
  return JSON.parse(element.props.content);
}
function surface(view) {
  return flatten(view.render()).find(element=>element.props.id==='brief-preview-artifact');
}

test('a restored edit pins header and surface before its captured read returns',async()=>{
  let restore;
  const owner=fixture(read);
  function read(_id,revision){
    return revision===1?new Promise(done=>{restore=()=>done(content(1));}):Promise.resolve(content(2));
  }
  await settle();assert.equal(header(owner.view).filters[0].selected,'2');
  owner.send({kind:'edited',artifact:'other',revision:1});await settle();
  assert.deepEqual(owner.reads,[undefined]);assert.equal(owner.view.state.follow,true);
  owner.send({kind:'edited',artifact:'artifact',revision:1});await settle();
  assert.equal(header(owner.view).filters[0].selected,'1');
  assert.equal(surface(owner.view).props.revision,1);
  assert.equal(owner.view.state.artifact.revision,2,'the prior read remains visible until restoration completes');
  restore();await settle();
  assert.equal(owner.view.state.artifact.revision,1);assert.equal(owner.view.state.follow,false);
  assert.deepEqual(owner.reads,[undefined,1]);assert.deepEqual(owner.errors,[]);
  owner.view.release();
});

test('a restored edit supersedes the initial latest read',async()=>{
  let initial;
  const owner=fixture((_id,revision)=>revision===undefined
    ?new Promise(done=>{initial=()=>done(content(2));}):Promise.resolve(content(revision)));
  owner.send({kind:'edited',artifact:'artifact',revision:1});await settle();
  assert.equal(header(owner.view).filters[0].selected,'1');assert.equal(owner.view.state.follow,false);
  initial();await settle();
  assert.equal(owner.view.state.artifact.revision,1);assert.equal(header(owner.view).filters[0].selected,'1');
  assert.equal(surface(owner.view).props.revision,1);assert.deepEqual(owner.reads,[undefined,1]);
  assert.deepEqual(owner.errors,[]);owner.view.release();
});

test('header leaves export captured revisions without replaying cancellation',async()=>{
  let finish;
  const owner=fixture((_id,revision)=>Promise.resolve(content(revision??2)),
    ()=>new Promise(done=>{finish=done;}));
  await settle();
  assert.equal('exportOpen' in owner.view,false);
  owner.header('brief-export');await settle();
  assert.deepEqual(owner.exports,[],'opening the menu is not an export selection');
  owner.header('brief-export-html');await settle();
  assert.deepEqual(owner.exports,[{artifact:'artifact',revision:2,format:'html'}]);
  assert.equal(header(owner.view).actions[0].disabled,true);
  assert(header(owner.view).actions[0].menu.every(item=>!item.enabled));
  finish(false);await settle();await settle();
  assert.equal(owner.view.state.exporting,false);assert.equal(owner.exports.length,1);
  owner.send({kind:'edited',artifact:'artifact',revision:1});await settle();
  assert.equal(header(owner.view).filters[0].selected,'1');assert.equal(surface(owner.view).props.revision,1);
  owner.header('brief-export-image');await settle();
  assert.deepEqual(owner.exports,[{artifact:'artifact',revision:2,format:'html'},
    {artifact:'artifact',revision:1,format:'png'}]);
  finish(true);await settle();assert.deepEqual(owner.errors,[]);assert.deepEqual(owner.feedback,[]);
  owner.view.release();
});

test('header dispatch retains guards while refresh keeps export',async()=>{
  const owner=fixture(()=>Promise.resolve(content(2)));
  await settle();
  owner.view.state.connected=false;assert.equal(header(owner.view).actions[0].disabled,true);
  owner.header('brief-export-html');await settle();assert.deepEqual(owner.exports,[]);
  owner.view.state.connected=true;owner.view.state.loading=true;
  assert.equal(header(owner.view).actions[0].disabled,false);
  owner.header('brief-export-html');await settle();
  assert.deepEqual(owner.exports,[{artifact:'artifact',revision:2,format:'html'}]);
  owner.view.state.loading=false;owner.view.state.exporting=true;
  assert.equal(header(owner.view).actions[0].disabled,true);
  owner.header('brief-export-html');await settle();assert.equal(owner.exports.length,1);
  owner.view.state.exporting=false;
  owner.view.state.artifact.kind='preview';
  assert.deepEqual(header(owner.view).actions[0].menu.map(item=>item.id),['brief-export-html']);
  owner.header('brief-export-image');await settle();assert.equal(owner.exports.length,1);
  assert.deepEqual(owner.feedback.map(item=>item.message),[messages('en').brief_export_unsupported]);
  owner.header('brief-export-html');await settle();
  assert.deepEqual(owner.exports,[{artifact:'artifact',revision:2,format:'html'},
    {artifact:'artifact',revision:2,format:'html'}]);
  owner.view.release();owner.header('brief-export-html');await settle();
  assert.equal(owner.exports.length,2);assert.deepEqual(owner.errors,[]);
});

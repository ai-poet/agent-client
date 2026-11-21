import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';
import {messages,errorKey,imported} from '../dev.sailry.platform/desktop/locales.js';
import {destination} from '../dev.sailry.platform/desktop/address.js';

async function controller(file, bindings) {
  const annotation = file === 'main' ? (await readFile(new URL('../dev.sailry.platform/desktop/annotation.js',import.meta.url),'utf8'))
    .replace(/^import[\s\S]*?from '[^']+';\n/gm,'').replace(/export (class|function) /g,'$1 ') : '';
  const source = (await readFile(new URL(`../dev.sailry.platform/desktop/${file}.js`,import.meta.url),'utf8'))
    .replace(/^import[\s\S]*?from '[^']+';\n/gm,'')
    .replace('export default class ','class ');
  const name = file === 'main' ? 'Browser' : 'Settings';
  const pending = () => new Promise(() => {});
  const notices=[];
  const type = vm.runInNewContext(`${annotation}\n${source}\n${name}`,{
    View:class {},context:() => '{"locale":"en"}',messages,errorKey,imported,destination,
    nextBrowserChange:pending,nextBrowserShortcut:pending,nextNavigationTabEvent:pending,nextTextEvent:pending,
    nextControlEvent:pending,modal_closed:pending,releaseText() {},toast:value=>notices.push(value),...bindings
  });
  const work = [];
  const cx = {notify() {},spawn(task) { const result = task(cx); work.push(result); return result; }};
  const view = new type(); view.init({},cx);
  return {view,cx,notices,async complete(action) {
    const before = work.length; action(); await Promise.all(work.slice(before));
  }};
}

test('page updates preserve drafts; selecting another tab replaces them', async () => {
  let focused = true, address = '', requests = [];
  const snapshot = (selected,url,cursor) => ({selected,cursor,tabs:[{id:selected,url,title:'Page',loading:false,error:null}],history:[false,false]});
  const {view,cx,complete} = await controller('main',{
    readBrowser:() => snapshot(7,'https://example.com/first','1'),
    createText:value => { address = value; return 'address'; },readText:() => address,
    setText:(_id,value) => { address = value; },isTextFocused:() => focused,focusText:() => { focused = true; },
    browserAction:async action => { requests.push(action); return snapshot(9,'https://example.com/selected','3'); },
    toast() {}
  });
  address = 'unfinished search';
  view.accept(snapshot(7,'https://example.com/redirect','2'),false,cx);
  assert.equal(address,'unfinished search');
  await complete(() => view.perform('select',{id:9},cx));
  assert.equal(address,'https://example.com/selected');
  assert.equal(requests[0].id,9);
  assert.equal(view.selected().id,9);
  focused = false;
  view.accept(snapshot(9,'https://example.com/next','4'),false,cx);
  assert.equal(address,'https://example.com/next');
  view.accept(snapshot(7,'https://example.com/stale','2'),true,cx);
  assert.equal(address,'https://example.com/next');
  assert.equal(view.selected().id,9);
  focused = true;
  address = 'another draft';
  view.accept(snapshot(9,'https://example.com/next','4'),true,cx);
  assert.equal(address,'https://example.com/next');
  view.accept(snapshot(9,'https://example.com/nine','9'),true,cx);
  view.accept(snapshot(9,'https://example.com/ten','10'),true,cx);
  view.accept(snapshot(7,'https://example.com/older','9'),true,cx);
  assert.equal(address,'https://example.com/ten');
  assert.equal(view.state.cursor,'10');
});

test('native new-tab shortcuts use the address focus owner',async()=>{
  let deliver,focused;
  const focus=new Promise(resolve=>{focused=resolve;});
  const requests=[];
  let address='https://example.com/';
  const snapshot=(cursor,id,url)=>({cursor,selected:id,tabs:[{id,url}],history:[false,false]});
  const {view}=await controller('main',{
    readBrowser:()=>snapshot('1',1,address),
    nextBrowserShortcut:()=>new Promise(resolve=>{deliver=resolve;}),
    createText:()=> 'address',readText:()=>address,
    setText:(_id,value)=>{address=value;},isTextFocused:()=>false,
    focusText:id=>focused(id),
    browserAction:async action=>{requests.push(action);return snapshot('2',2,'');}
  });
  deliver({kind:'add'});
  assert.equal(await focus,'address');
  assert.deepEqual(requests.map(request=>JSON.parse(JSON.stringify(request))),[{kind:'add'}]);
  assert.equal(view.selected().id,2);
  assert.equal(address,'');
});

test('capture updates preserve drafts and deduplicate failures', async () => {
  const fields = new Map(), released = [], notices = [];
  const capture = {token:'capture',element:{url:'https://example.com',selector:'#heading',text:'Heading',truncated:false}};
  const snapshot = cursor => ({cursor,selected:1,tabs:[{id:1,url:'https://example.com'}],history:[false,false],annotation:capture});
  const {view,cx} = await controller('main',{
    readBrowser:() => snapshot('1'),
    createText:value => { const id = `field-${fields.size}`; fields.set(id,value); return id; },
    readText:id => fields.get(id),setText:(id,value) => fields.set(id,value),
    releaseText:id => released.push(id),isTextFocused:() => true,
    toast:notice => notices.push(notice)
  });
  const note = view.review.current.note;
  fields.set(note,'Keep this annotation'); fields.set(view.address,'unfinished address');
  view.accept({...snapshot('2'),annotation_error:'browser_annotation_unavailable'},false,cx);
  view.accept({...snapshot('3'),annotation_error:'browser_annotation_unavailable'},false,cx);
  assert.equal(view.review.current.note,note);
  assert.equal(fields.get(note),'Keep this annotation');
  assert.equal(fields.get(view.address),'unfinished address');
  assert.equal(notices.length,1);
  assert.equal(notices[0].message,messages('en').browser_annotation_unavailable);
  view.accept({...snapshot('4'),annotation:null},false,cx);
  assert.deepEqual(released,[note]);
  assert.equal(view.review.current,null);
});

test('element selection uses the captured browser without navigating', async () => {
  let address = 'unfinished address';
  const requests = [];
  const snapshot = {cursor:'1',selected:7,tabs:[{id:7,url:'https://example.com/page',loaded:true,loading:false}],history:[false,false],supported:true,annotating:false};
  const {view,cx,complete} = await controller('main',{
    readBrowser:() => snapshot,
    createText:() => 'address',readText:() => address,setText:(_id,value) => { address = value; },
    isTextFocused:() => true,focusText() {},
    browserAction:async action => { requests.push(action); return {...snapshot,cursor:'2',annotating:true}; }
  });
  await complete(() => view.perform('annotate',{},cx));
  assert.deepEqual(requests.map(value => JSON.parse(JSON.stringify(value))),[{kind:'annotate'}]);
  assert.equal(address,'unfinished address');
  assert.equal(view.state.annotating,true);
  assert.equal(view.selected().id,7);
  for (const locale of ['en','zh-CN','zh-TW','ja','ko','de','fr','es','pt-BR','ru']) {
    assert.ok(messages(locale).browser_annotate);
    assert.ok(messages(locale).browser_cancel_selection);
    assert.ok(messages(locale).browser_annotation_unavailable);
  }
});

test('an uncertain browser action is reported once and never replayed', async () => {
  let calls = 0, notices = [];
  const {view,cx,complete} = await controller('main',{
    readBrowser:() => ({cursor:'1',selected:0,tabs:[{id:0,url:''}],history:[false,false]}),
    createText:() => 'address',readText:() => 'example.com',setText() {},focusText() {},isTextFocused:() => false,
    browserAction:async () => { calls++; throw new Error('browser_unavailable'); },
    toast:notice => notices.push(notice)
  });
  await complete(() => view.navigate(cx));
  assert.equal(calls,1);
  assert.equal(notices.length,1);
  assert.equal(notices[0].message,messages('en').browser_unavailable);
});

test('a blurred address draft survives page status changes and rejected navigation', async () => {
  let address = '', focused = false, requests = [], notices = [];
  const snapshot = (cursor, changes = {}) => ({cursor,selected:3,tabs:[{
    id:3,url:'https://example.com/',title:'Page',loading:false,error:null,...changes
  }],history:[false,false]});
  let result = snapshot('4',{error:'browser_invalid_address'});
  const {view,cx,complete} = await controller('main',{
    readBrowser:() => snapshot('1'),
    createText:value => { address = value; return 'address'; },readText:() => address,
    setText:(_id,value) => { address = value; },isTextFocused:() => focused,focusText() {},
    browserAction:async action => { requests.push(action); return result; },
    toast:notice => notices.push(notice)
  });
  address = 'https://';
  view.accept(snapshot('2',{title:'Updated title'}),false,cx);
  assert.equal(address,'https://');
  view.accept(snapshot('3',{title:'Updated title',loading:true}),false,cx);
  assert.equal(address,'https://');
  await complete(() => view.navigate(cx));
  assert.equal(requests[0].kind,'navigate');
  assert.equal(requests[0].url,'https://');
  assert.equal(address,'https://');
  assert.equal(view.selected().url,'https://example.com/');
  assert.equal(notices.length,1);
  assert.equal(notices[0].message,messages('en').browser_invalid_address);
  focused = true;
  address = 'example.com';
  result = snapshot('5');
  await complete(() => view.navigate(cx));
  assert.equal(requests[1].url,'https://example.com');
  assert.equal(address,'https://example.com/');
});

test('invalid address policy preserves the draft without dispatching navigation', async () => {
  let address = '', requests = [], notices = [];
  const {view,cx,complete} = await controller('main',{
    readBrowser:() => ({cursor:'1',selected:0,tabs:[{id:0,url:'https://example.com/'}],history:[false,false]}),
    createText:value => { address = value; return 'address'; },readText:() => address,
    setText:(_id,value) => { address = value; },isTextFocused:() => false,
    browserAction:async action => { requests.push(action); },
    toast:notice => notices.push(notice)
  });
  for (const invalid of ['', 'javascript:alert(1)', 'file:///tmp/file', 'data:text/html,test']) {
    address = invalid;
    await complete(() => view.navigate(cx));
    assert.equal(address,invalid);
  }
  assert.equal(requests.length,0);
  assert.equal(notices.length,4);
  assert.ok(notices.every(notice => notice.message === messages('en').browser_invalid_address));
});

test('profile import uses the selected opaque ID and reports verified counts', async () => {
  let importedId, persisted;
  const {view,cx,complete,notices} = await controller('settings',{
    readBrowserSettings:() => ({supported:true,enabled:true,persistent:persisted ?? true}),
    setBrowserPersistent:value => { persisted = value; },
    listBrowserProfiles:async () => [{id:'profile-other',name:'Work'},{id:'profile-opaque',name:'Personal'}],
    importBrowserProfile:async id => { importedId = id; return {count:8,skipped:2}; }
  });
  await complete(() => view.scan(cx));
  assert.equal(view.profiles[1].id,'profile-opaque');
  assert.equal(importedId,undefined);
  await complete(() => view.import(view.profiles[1],cx));
  assert.equal(importedId,'profile-opaque');
  assert.equal(view.profiles,null);
  assert.equal(Object.hasOwn(view,'message'),false);assert.equal(notices.length,1);assert.equal(notices[0].message,'Imported 8 cookies, skipped 2');assert.equal(notices[0].kind,'info');
  assert.equal(view.busy,false);
  view.persist(false,cx);
  assert.equal(persisted,false);
  assert.equal(view.state.persistent,false);
});

test('profile errors clear busy state and retain localized failure meaning', async () => {
  const {view,cx,complete,notices} = await controller('settings',{
    readBrowserSettings:() => ({supported:true,enabled:true,persistent:true}),
    listBrowserProfiles:async () => { throw new Error('sailry/ui.listBrowserProfiles: browser_chrome_key_denied'); }
  });
  await complete(() => view.scan(cx));
  assert.equal(view.busy,false);
  assert.equal(view.profiles,null);
  assert.equal(Object.hasOwn(view,'message'),false);assert.equal(notices.length,1);assert.equal(notices[0].message,messages('en').browser_chrome_key_denied);assert.equal(notices[0].kind,'error');
});

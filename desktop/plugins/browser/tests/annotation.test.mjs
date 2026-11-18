import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';
import {Element,flatten} from '../../tests/elements.mjs';
import {messages,errorKey} from '../dev.sailry.platform/desktop/locales.js';

const source = (await readFile(new URL('../dev.sailry.platform/desktop/annotation.js',import.meta.url),'utf8'))
  .replace(/^import[\s\S]*?from '[^']+';\n/gm,'').replace(/export (class|function) /g,'$1 ');
const page = (await readFile(new URL('../dev.sailry.platform/desktop/view.js',import.meta.url),'utf8'))
  .replace(/^import[\s\S]*?from '[^']+';\n/gm,'').replace(/export function /g,'function ');
const element = {url:'https://example.com',title:'Example',selector:'#heading',path:'html > body > h1#heading',
  tag:'h1',id:'heading',classes:[],attributes:{},name:'',text:'Heading',html:'<h1 id="heading">Heading</h1>',
  styles:{},component:null,bounds:[10,20,100,40],viewport:[1280,800],truncated:false};
const capture = token => ({token,element});

function fixture(submit = async () => {}) {
  const fields = new Map(), released = [], requests = [], notices = [], work = [];
  let serial = 0;
  const api = vm.runInNewContext(`${source}\n${page}\n({Review,reference,renderReview,render})`,{
    div:() => new Element(),Button:Element,Modal:Element,BrowserAnnotationImage:Element,TextField:Element,
    Icon:Element,BrowserSurface:Element,PanelHeader:Element,NavigationTabs:Element,IconButton:Element,
    theme:() => ({colors:{muted_foreground:'theme-muted'}}),
    createText:(value,options) => { const id = `note-${++serial}`; fields.set(id,{value,options}); return id; },
    readText:id => fields.get(id).value,
    releaseText:id => { released.push(id); fields.delete(id); },submitBrowserAnnotation:submit
  });
  const view = {text:messages('en'),address:'address',state:{annotation:null,selected:1,
    tabs:[{id:1,url:element.url,title:'Page',loaded:true,loading:false}],
    history:[false,false],supported:true,annotating:false},
    selected() { return this.state.tabs.find(tab => tab.id === this.state.selected); },
    report:(error,fallback) => notices.push(errorKey(error,fallback)),
    perform:(kind,args) => requests.push({kind,...args})};
  const cx = {notify() {},theme:() => ({typography:{mono:'theme-mono'}}),
    spawn(task) { const result = task(cx); work.push(result); return result; }};
  view.review = new api.Review(view);
  return {...api,view,cx,fields,released,requests,notices,
    accept(token) { view.state.annotation = token ? capture(token) : null; view.review.accept(view.state.annotation); },
    note(value) { fields.get(view.review.current.note).value = value; },
    complete() { return Promise.all(work); }};
}

test('matching captures retain drafts and replacements release text handles', () => {
  const f = fixture(); f.accept('first');
  const note = f.view.review.current.note;
  f.note('Keep the draft'); f.accept('first');
  assert.equal(f.view.review.current.note,note);
  assert.equal(f.fields.get(note).value,'Keep the draft');
  assert.equal(f.fields.get(note).options.multiline,true);
  f.accept('second');
  assert.deepEqual(f.released,[note]);
  assert.equal(f.fields.get(f.view.review.current.note).value,'');
  f.accept(null);
  assert.equal(f.fields.size,0);
  assert.equal(f.view.review.current,null);
});

test('failed submissions retain drafts and retry the original capture', async () => {
  let fail = true; const submitted = [];
  const f = fixture(async (token,value) => {
    submitted.push({token,value}); if (fail) throw new Error('chat_attachment_limit');
  });
  f.accept('original'); f.note('Keep this annotation');
  const note = f.view.review.current.note;
  f.view.review.submit(f.cx); await f.complete();
  assert.equal(f.view.review.pending,false);
  assert.equal(f.view.review.current.note,note);
  assert.equal(f.fields.get(note).value,'Keep this annotation');
  assert.deepEqual(f.notices,['chat_attachment_limit']);
  assert.equal(f.requests.length,0);
  fail = false;
  f.view.review.submit(f.cx); await f.complete();
  assert.deepEqual(submitted.map(item=>item.token),['original','original']);
  assert.deepEqual(JSON.parse(submitted[1].value),{page_element:element,annotation:'Keep this annotation'});
  assert.equal(f.view.review.current,null);
  assert.equal(f.fields.size,0);
  assert.deepEqual(f.requests,[{kind:'cancel_annotation',token:'original'}]);
});

test('pending submissions run once and cannot close newer drafts', async () => {
  let finish, count = 0;
  const f = fixture(() => { count++; return new Promise(resolve => { finish = resolve; }); });
  f.accept('first'); f.note('First draft');
  f.view.review.submit(f.cx); f.view.review.submit(f.cx);
  assert.equal(count,1);
  assert.equal(f.view.review.pending,true);
  f.view.review.close(f.cx);
  assert.deepEqual(f.requests,[{kind:'cancel_annotation',token:'first'}]);
  f.accept('second'); f.note('Second draft');
  finish(); await f.complete();
  assert.equal(f.view.review.current.token,'second');
  assert.equal(f.fields.get(f.view.review.current.note).value,'Second draft');
  assert.equal(f.view.review.pending,false);
  assert.equal(f.requests.length,1);
});

test('invalid notes and stale captures never dispatch', async () => {
  let count = 0; const f = fixture(async () => { count++; });
  f.accept('first'); f.note('x\0y');
  f.view.review.submit(f.cx); await f.complete();
  assert.deepEqual(f.notices,['browser_annotation_invalid']);
  assert.equal(f.view.review.current.token,'first');
  f.note('Valid draft'); f.view.state.annotation = capture('second');
  f.view.review.submit(f.cx); await f.complete();
  assert.equal(count,0);
});

test('references preserve context and enforce the UTF8 byte limit', () => {
  const f = fixture();
  assert.equal(JSON.parse(f.reference(element,'😀'.repeat(1024))).annotation.length,2048);
  assert.throws(()=>f.reference(element,'😀'.repeat(1025)),/browser_annotation_invalid/);
  assert.throws(()=>f.reference(element,'x\0y'),/browser_annotation_invalid/);
  assert.deepEqual(JSON.parse(f.reference(element,'')),{page_element:element,annotation:''});
});

test('review uses a Kit form with private preview and aligned actions', () => {
  const f = fixture(); f.accept('private-capture');
  f.view.review.current.element = {...element,truncated:true};
  const modal = f.renderReview(f.view,f.cx), nodes = flatten(modal);
  assert.equal(modal.props.open,true);
  assert.equal(modal.props.form,true);
  assert.equal(modal.props.width,560);
  const content = modal.items[0], card = content.items[0], selector = content.items[1];
  assert.equal(card.props.id,'browser-annotation-preview');
  assert.ok(Object.hasOwn(card.props,'w_full'));
  assert.equal(card.props.max_h,240);
  assert.ok(Object.hasOwn(card.props,'rounded_lg'));
  assert.ok(Object.hasOwn(card.props,'overflow_hidden'));
  const preview = nodes.find(node=>node?.props?.id === 'browser-annotation-image');
  assert.equal(card.items[0],preview);
  assert.deepEqual(preview.props,{id:'browser-annotation-image',token:'private-capture'});
  assert.equal(selector.props.id,'browser-annotation-selector');
  assert.ok(Object.hasOwn(selector.props,'truncate'));
  assert.equal(selector.props.font_family,'theme-mono');
  assert.equal(selector.props.text_color,'theme-muted');
  assert.equal(selector.props.tooltip,`Page\n${element.url}`);
  assert.deepEqual(selector.items,[element.selector]);
  assert.equal(content.items[2].props.id,'browser-annotation-text');
  assert.equal(content.items.at(-1).props.id,'browser-annotation-note');
  const footer = modal.items[1];
  assert.ok(Object.hasOwn(footer.props,'justify_end'));
  assert.deepEqual(footer.items.map(node=>node.props.id),['browser-annotation-cancel','browser-annotation-add']);
  assert.ok(Object.hasOwn(footer.items[1].items[0].props,'primary'));
  f.view.review.pending = true;
  const busy = flatten(f.renderReview(f.view,f.cx)).find(node=>node?.props?.id === 'browser-annotation-add' && Object.hasOwn(node.props,'disabled'));
  assert.equal(busy.props.disabled,true); assert.equal(busy.props.loading,true);
  f.accept(null);
  assert.equal(f.renderReview(f.view,f.cx).items.length,0);
});

test('review names development components', () => {
  const f = fixture(); f.accept('component');
  const component = () => flatten(f.renderReview(f.view,f.cx)).find(node=>node?.props?.id === 'browser-annotation-component');
  assert.equal(component(), undefined);
  f.view.review.current.element = {...element,component:{framework:'react',name:'Heading',source:'src/App.tsx:12',stack:['Heading']}};
  assert.deepEqual(component().items, ['Heading · src/App.tsx:12']);
  f.view.review.current.element = {...element,component:{framework:'vue',name:'Card',source:null,stack:['Card']}};
  assert.deepEqual(component().items, ['Card']);
});

test('review captures page titles and omits duplicate text', () => {
  const f = fixture();
  f.view.state.tabs.push({id:2,url:element.url,title:'Selected heading'});
  f.view.state.selected = 2;
  f.accept('selected'); f.note('Keep this annotation');
  const current = f.view.review.current, note = current.note;
  assert.equal(current.title,'Selected heading');
  f.view.state.tabs[1].title = 'Updated title'; f.accept('selected');
  assert.equal(f.view.review.current,current);
  assert.equal(current.title,'Selected heading');
  assert.equal(f.fields.get(note).value,'Keep this annotation');
  current.element = {...element,text:' Selected heading\n'};
  const rendered = () => flatten(f.renderReview(f.view,f.cx));
  let nodes = rendered();
  assert.equal(nodes.find(node=>node?.props?.id === 'browser-annotation-selector').props.tooltip,
    `Selected heading\n${element.url}`);
  assert.ok(!nodes.some(node=>node?.props?.id === 'browser-annotation-text'));
  assert.equal(JSON.parse(f.reference(current.element,'')).page_element.text,' Selected heading\n');
  current.element = {...element,text:'Different selected content'};
  nodes = rendered();
  assert.deepEqual(nodes.find(node=>node?.props?.id === 'browser-annotation-text').items,
    ['Different selected content']);
  current.element = {...element,text:''};
  assert.ok(!rendered().some(node=>node?.props?.id === 'browser-annotation-text'));
});

test('controls use bundled icons', () => {
  const f = fixture();
  const control = () => flatten(f.render(f.view,f.cx)).find(node=>node?.props?.id === 'browser-annotate');
  assert.equal(control().props.icon,'reicon:map/pointer');
  assert.equal(control().props.disabled,false);
  f.view.state.annotating = true;
  assert.equal(control().props.icon,'close');
  assert.equal(control().props.label,f.view.text.browser_cancel_selection);
  f.view.state.tabs[0].loading = true;
  assert.equal(control().props.disabled,true);
  f.view.state.annotating = false;
  assert.equal(control().props.icon,'reicon:map/pointer');
});

test('review captions and attachment failures cover every locale', () => {
  for (const locale of ['en','zh-CN','zh-TW','ja','ko','de','fr','es','pt-BR','ru']) {
    const text = messages(locale);
    for (const key of ['browser_annotation','browser_annotation_note','browser_annotation_add',
      'browser_annotation_stale','browser_annotation_invalid','browser_annotation_unavailable',
      'browser_annotation_truncated','settings_cancel','chat_attachment_limit']) assert.ok(text[key],`${locale}: ${key}`);
  }
  assert.equal(errorKey('sailry/ui.submitBrowserAnnotation: chat_attachment_limit'),'chat_attachment_limit');
  assert.equal(errorKey('Unexpected failure','browser_annotation_stale'),'browser_annotation_stale');
});

test('page retains its surface and review draft', () => {
  const f = fixture();
  Object.assign(f.view,{address:'address',selected() { return this.state.tabs[0]; }});
  Object.assign(f.view.state,{selected:1,tabs:[{id:1,url:element.url,title:'Page',loaded:true,loading:false}],
    history:[false,false],supported:true,annotating:false});
  const rendered = () => flatten(f.render(f.view,f.cx));
  const surface = nodes => nodes.find(node=>node?.props?.id === 'browser-surface');
  const header = rendered().find(node=>node?.props?.id === 'browser-header');
  assert.equal(header.props.padding,8);
  assert.equal(Object.hasOwn(header.props,'surface'),false);
  const tabs = rendered().find(node=>node?.props?.id === 'browser-tabs');
  assert.equal(tabs.props.selected,'1');
  assert.equal(tabs.props.items[0].label,'Page');
  assert.deepEqual(surface(rendered()).props,{id:'browser-surface'});
  f.accept('selected');
  f.note('Keep this review');
  const note = f.view.review.current.note, nodes = rendered();
  assert.deepEqual(surface(nodes).props,{id:'browser-surface'});
  assert.equal(nodes.find(node=>node?.props?.id === 'browser-annotation-dialog').props.open,true);
  assert.equal(f.fields.get(note).value,'Keep this review');
  assert.equal(f.view.review.current.token,'selected');
  f.view.review.close(f.cx);
  assert.deepEqual(surface(rendered()).props,{id:'browser-surface'});
  assert.equal(f.view.review.current,null);
  assert.deepEqual(f.released,[note]);
  assert.deepEqual(f.requests,[{kind:'cancel_annotation',token:'selected'}]);
});

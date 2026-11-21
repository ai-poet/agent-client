import assert from 'node:assert/strict';
import test from 'node:test';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import {Element,flatten} from '../../tests/elements.mjs';
import {messages,locales} from '../dev.sailry.platform/desktop/locales.js';
import {Controller} from '../dev.sailry.platform/desktop/controller.js';

const source = readFileSync(new URL('../dev.sailry.platform/desktop/view.js',import.meta.url),'utf8')
  .replace(/^import .*;\n/gm,'').replace(/^export /gm,'');
const {render} = vm.runInNewContext(`${source}\n({render})`,{
  div:() => new Element(),Header:Element,IconButton:Element,Loading:Element,ArtifactSurface:Element,
});
function setup() {
  const state = new Controller({}, {id:'artifact',session:'session'}, () => {}, () => {});
  state.artifact = {id:'artifact',revision:2,title:'Brief title',kind:'document'};state.revision = 2;state.items = [{revision:2},{revision:1}];
  return {state,text:messages('zh-CN')};
}
const find = (view,id) => flatten(render(view)).find(item => item instanceof Element && item.props.id === id);

test('panel header owns version selection and the export action',() => {
  const view = setup(),header = JSON.parse(find(view,'brief-header').props.content),menu = header.actions[0].menu;
  assert.equal(header.filters[0].selected,'2');assert.equal(header.filters[0].label,'版本');assert.equal(header.actions[0].icon,'icons/ellipsis-vertical.svg');
  assert.deepEqual(JSON.parse(JSON.stringify(menu)),[
    {id:'brief-export-html',label:'HTML',enabled:true},{id:'brief-export-image',label:'图片',enabled:true},
  ]);
  assert.equal(find(view,'brief-export-picker'),undefined);
  const surface = find(view,'brief-preview-artifact');assert.equal(surface.props.artifact,'artifact');assert.equal(surface.props.revision,undefined);
  view.state.follow = false;assert.equal(find(view,'brief-preview-artifact').props.revision,2);
  assert.equal(surface.props.labels.submit,'提交');
  assert.equal(surface.props.labels.render_error,undefined);
  assert.equal(surface.props.labels.required,view.text.brief_form_required);
  assert.equal(surface.props.labels.invalid_form,view.text.brief_form_invalid);
});
test('header actions use complete bundled icon paths',() => {
  const view = setup();view.state.before = 1;
  const header = JSON.parse(find(view,'brief-header').props.content);
  assert.deepEqual(header.actions.map(({id,icon}) => ({id,icon})),[
    {id:'brief-more-versions',icon:'icons/ellipsis.svg'},
    {id:'brief-export',icon:'icons/ellipsis-vertical.svg'},
  ]);
});
test('interactive previews offer only HTML export',() => {
  const view = setup();view.state.artifact.kind = 'preview';
  const menu = JSON.parse(find(view,'brief-header').props.content).actions[0].menu;
  assert.deepEqual(JSON.parse(JSON.stringify(menu)),[
    {id:'brief-export-html',label:'HTML',enabled:true},
  ]);
  assert(find(view,'brief-preview-artifact'));
  assert.equal(JSON.parse(find(view,'brief-header').props.content).filters[0].selected,'2');
});
test('export declarations preserve connection and duplicate guards',() => {
  for (const field of ['connected','exporting']) {
    const view = setup();view.state[field] = field === 'connected' ? false : true;
    const action = JSON.parse(find(view,'brief-header').props.content).actions[0];
    assert.equal(action.disabled,true);assert(action.menu.every(item => !item.enabled));
    assert(find(view,'brief-preview-artifact'));
  }
  const view = setup();delete view.state.artifact.kind;
  assert.deepEqual(JSON.parse(find(view,'brief-header').props.content).actions[0].menu.map(item => item.id),['brief-export-html']);
  view.state.before = 1;
  assert.equal(JSON.parse(find(view,'brief-header').props.content).actions[0].menu,undefined);
});
test('refresh keeps the preview and independent controls available',() => {
  const view = setup();view.state.loading = true;
  assert.equal(find(view,'brief-preview-artifact').props.revision,2);
  const header = JSON.parse(find(view,'brief-header').props.content);
  assert.equal(header.filters[0].disabled,false);assert.equal(header.actions[0].disabled,false);
  assert(header.actions[0].menu.every(item => item.enabled));
  view.state.revision = 1;
  assert.equal(find(view,'brief-preview-artifact').props.revision,2);
  view.state.artifact = null;
  assert.equal(find(view,'brief-loading'),undefined);assert.equal(find(view,'brief-retry'),undefined);
  assert(find(view,'brief-header'));assert.equal(find(view,'brief-preview-artifact'),undefined);
});
test('unavailable reads show a retry control without repeating error feedback',() => {
  const view = setup();view.state.artifact = null;view.state.failed = true;
  assert.equal(find(view,'brief-retry').props.label,'重试');assert.equal(find(view,'brief-preview-artifact'),undefined);
});
test('all ten locales have the complete concise form and panel vocabulary',() => {
  assert.equal(Object.keys(locales).length,10);
  for (const [locale,values] of Object.entries(locales)) {
    assert.deepEqual(Object.keys(values),Object.keys(locales.en),locale);
    assert(Object.values(values).every(value => typeof value === 'string' && value && !/[.。]$/.test(value)),locale);
  }
  assert.equal(messages('unknown'),locales.en);
});

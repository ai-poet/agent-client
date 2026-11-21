import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFile} from 'node:fs/promises';
import {messages} from '../dev.sailry.platform/desktop/locales.js';
import {choices} from '../dev.sailry.platform/desktop/access.js';

const source = (await readFile(new URL('../dev.sailry.platform/desktop/view.js', import.meta.url), 'utf8'))
  .replace(/^import .*;\n/gm, '').replace(/^export /gm, '');

function fixture(item, error = null) {
  const nodes = new Map(), calls = [];
  function element(kind = 'div', id = null, props = {}) {
    const node = {kind, id, props, items:[], styles:{}, handlers:{}};
    const proxy = new Proxy(node, {get(target, key) {
      if (key === 'id') return id => { target.id = id; nodes.set(id, proxy); return proxy; };
      if (key === 'child') return child => { target.items.push(child); return proxy; };
      if (key === 'children') return children => { target.items.push(...children); return proxy; };
      if (key === 'on_click' || key === 'on_change') return callback => { target.handlers[key] = callback; return proxy; };
      if (key in target) return target[key];
      return (...args) => { target.styles[key] = args; return proxy; };
    }});
    if (id) nodes.set(id, proxy);
    return proxy;
  }
  const constructors = Object.fromEntries(['Button', 'Tab', 'TabBar','Spinner'].map(kind =>
    [kind, function(id) { return element(kind, id); }]));
  const {render} = vm.runInNewContext(`${source}\n({render})`, {
    div:() => element(), choices, ...constructors, theme:() => ({colors:{destructive:'danger'}}),
    Terminal:{new:(id, props) => element('Terminal', id, props)},
    Menu:{new:(id, props) => element('Menu', id, props)},
  });
  const view = {items:[item], tools:[], selected:item.id, resource:{id:item.id}, text:messages('en'),
    loaded:true, loading:false, busy:!error, pending:null, openRetry:error === 'failed' ? item.id : null, error, retry:() => calls.push('retry')};
  const root = render(view);
  const text = node => typeof node === 'string' ? node : node.items.map(text).join(' ');
  return {nodes, calls, view, root, render:() => render(view), text:text(root)};
}

test('opening a shell renders only the terminal background', () => {
  const setup = fixture({id:'shell', tool:null, ssh:null, status:{kind:'stopped'}, revision:3});
  const kinds = node => typeof node === 'string' ? [] : [node.kind,...node.items.flatMap(kinds)];
  const text = node => typeof node === 'string' ? node : node.items.map(text).join(' ');
  for (const state of [
    {busy:true, openAttempts:new Map([['shell', {revision:3, failed:false}]])},
    {busy:false, openAttempts:new Map([['shell', {revision:3, failed:false}]])},
    {busy:false, openAttempts:new Map(), openIntent:{target:'shell'}},
    {busy:false, openAttempts:new Map(), openIntent:{target:null}},
  ]) {
    Object.assign(setup.view, {openIntent:null}, state);
    const root = setup.render();
    assert.equal(kinds(root).includes('Spinner'), false);
    assert.doesNotMatch(text(root), /Terminal stopped|Loading/);
    assert.equal(setup.nodes.get('commands-terminal').props.terminal, null);
  }
});

test('opening state ignores external, failed and outdated attempts', () => {
  const view = {error:null, openIntent:null, openAttempts:new Map([['shell', {revision:3, failed:true}]])};
  const {opening} = vm.runInNewContext(`${source}\n({opening})`, {});
  const shell = {id:'shell', tool:null, ssh:null, status:{kind:'stopped'}, revision:3};
  assert.equal(opening(view, shell), false);
  view.openAttempts.set('shell', {revision:2, failed:false});
  assert.equal(opening(view, shell), false);
  view.openIntent = {target:'other'};
  assert.equal(opening(view, shell), false);
  view.openIntent = {target:'shell'};
  assert.equal(opening(view, {...shell, tool:'claude'}), false);
  assert.equal(opening(view, {...shell, status:{kind:'running'}}), false);
  assert.equal(opening({...view, error:'failed'}, shell), false);
  assert.equal(opening(view, shell), true);
});

test('idle restored shells remain stopped without a loading spinner', () => {
  const setup = fixture({id:'shell', tool:null, ssh:null, status:{kind:'stopped'}});
  setup.view.busy = false;
  const root = setup.render();
  const kinds = node => typeof node === 'string' ? [] : [node.kind,...node.items.flatMap(kinds)];
  const text = node => typeof node === 'string' ? node : node.items.map(text).join(' ');
  assert.equal(kinds(root).includes('Spinner'), false);
  assert.match(text(root), /Terminal stopped/);
  assert.equal(setup.nodes.has('terminal-resume'), false);
  assert.equal(setup.nodes.get('commands-terminal').props.terminal, null);
});

test('an actual open error keeps retry visible without claiming a running terminal', () => {
  const setup = fixture({id:'shell', tool:null, ssh:null, status:{kind:'stopped'}}, 'failed');
  assert.doesNotMatch(setup.text, /Terminal action failed/);
  assert.doesNotMatch(setup.text, /Loading|Terminal stopped/);
  setup.nodes.get('terminal-retry').handlers.on_click();
  assert.deepEqual(setup.calls, ['retry']);
  assert.equal(setup.nodes.get('commands-terminal').props.terminal, null);
});

test('stopped CLI and SSH terminals remain stopped without automatic launch controls', () => {
  for (const extra of [{tool:'claude'}, {ssh:'saved-host'}]) {
    const setup = fixture({id:'external', ...extra, status:{kind:'stopped'}});
    assert.match(setup.text, /Terminal stopped/);
    assert.equal(setup.nodes.has('terminal-resume'), false);
  }
});

test('catalog failures add neither retry bars nor attached terminals', () => {
  const setup = fixture({id:'shell', tool:null, ssh:null, status:{kind:'stopped'}}, 'loadFailed');
  assert.equal(setup.nodes.has('terminal-retry'), false);
  assert.equal(setup.nodes.get('commands-terminal').props.terminal, null);
});


test('manual creation offers full access and explicit workspace choices',()=>{
  const setup=fixture({id:'existing',status:{kind:'running'}});
  Object.assign(setup.view,{resource:null,busy:false,tools:[{tool:'claude',available:true}],perform:(...args)=>setup.calls.push(args)});
  setup.render();
  setup.nodes.get('terminal-new').handlers.on_click(null,{});
  assert.deepEqual(setup.calls[0].slice(0,2),['create',null]);
  const access=setup.nodes.get('terminal-access');
  assert.equal(access.kind,'Menu');
  assert.deepEqual(Array.from(access.props.items,item=>item.id),['unrestricted','workspace']);
  assert.match(access.props.items[0].label,/filesystem and network/);
  assert.match(access.props.items[1].label,/Workspace/);
  const tools=setup.nodes.get('terminal-tools').props.items;
  assert.equal(tools[0].id,'claude');
  assert.ok(tools.find(item=>item.id==='claude:workspace').label.includes('Workspace'));
});


test('opening a resource waits for its terminal metadata', () => {
  const setup = fixture({id:'shell',status:{kind:'running'}});
  Object.assign(setup.view,{items:[],busy:false,loaded:false});
  const text = node => typeof node === 'string' ? node : node.items.map(text).join(' ');
  const kinds = node => typeof node === 'string' ? [] : [node.kind,...node.items.flatMap(kinds)];
  for (const loaded of [false,true]) {
    setup.view.loaded = loaded;
    const root = setup.render();
    assert.doesNotMatch(text(root),/No terminals|Terminal stopped/);
    assert.equal(kinds(root).includes('Spinner'),false);
  }
  setup.view.items = [{id:'shell',status:{kind:'running'}}];
  const root = setup.render();
  assert.equal(setup.nodes.get('commands-terminal').props.terminal,'shell');
  assert.equal(kinds(root).includes('Spinner'),false);
  assert.doesNotMatch(text(root),/No terminals/);
});

test('only a confirmed empty inventory shows the empty state', () => {
  const setup = fixture({id:'shell',status:{kind:'running'}});
  Object.assign(setup.view,{items:[],selected:null,resource:null,busy:false,loaded:false});
  const text = node => typeof node === 'string' ? node : node.items.map(text).join(' ');
  assert.doesNotMatch(text(setup.render()),/No terminals/);
  setup.view.loaded = true;
  assert.match(text(setup.render()),/No terminals/);
  for (const state of [{loading:true},{loading:false,busy:true},{busy:false,pending:{id:'creating'}}]) {
    Object.assign(setup.view,state);
    assert.doesNotMatch(text(setup.render()),/No terminals/);
  }
  Object.assign(setup.view,{busy:false,pending:null,loaded:false,error:'loadFailed'});
  assert.doesNotMatch(text(setup.render()),/No terminals|Could not load terminals/);
});

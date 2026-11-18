import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const source = (await readFile(new URL('../dev.sailry.platform/desktop/explorer-view.js',import.meta.url),'utf8'))
  .replace(/^import .*;\n/gm,'').replace(/^export /gm,'');

function element(kind,props = {}) {
  const node = {kind,props,items:[]};
  const api = new Proxy(node,{get(target,key) {
    if (key in target) return target[key];
    return (...args) => {
      if (key === 'child') target.items.push(args[0]);
      else if (key === 'children') target.items.push(...args[0]);
      else target.props[key] = args.length === 1 ? args[0] : args;
      return api;
    };
  }});
  return api;
}

function all(node) {
  return node && typeof node === 'object' ? [node,...node.items.flatMap(all)] : [node];
}

test('main workspace starts its explorer at 320 pixels', async () => {
  const main = (await readFile(new URL('../dev.sailry.platform/desktop/main.js',import.meta.url),'utf8'))
    .replace(/^import[\s\S]*?;\n/gm,'').replace('export default class','class');
  const Files = vm.runInNewContext(`${main}\nFiles`,{
    View:class {},div:() => element('div'),
    Workspace:{new:(id,props) => element('workspace',{id,...props})},
    ActionScope:{new:(id,props) => element('actions',{id,...props})},
    Modal:{new:(id,props) => element('modal',{id,...props})},
    documentView:{header:() => element('header'),content:() => element('document')},
    explorerView:() => element('explorer'),
  });
  const view = new Files();
  Object.assign(view,{mode:'main',text:{file_tree:'File tree'},documents:{current:() => null},
    transfers:{value:() => null}});
  const workspace = all(view.render()).find(node => node?.kind === 'workspace');
  assert.equal(workspace.props.default_details_width,320);
  assert.equal(workspace.props.min_details_width,260);
  assert.equal(workspace.props.navigation,'none');
});

test('failed directory activation retries the real directory',async()=>{
  const main = (await readFile(new URL('../dev.sailry.platform/desktop/main.js',import.meta.url),'utf8'))
    .replace(/^import[\s\S]*?;\n/gm,'').replace('export default class','class');
  const Files = vm.runInNewContext(`${main}\nFiles`,{View:class {}});
  const view = new Files(), calls = [], eventCx = {};
  Object.assign(view,{explorer:{
    entry:path => ({kind:path === 'file.txt' ? 'file' : 'directory'}),
    pages:new Map([['worktrees',{error:'files_directory_failed'}],['empty',{loaded:true}]]),
    load:async (...args) => calls.push(args),
  },open:async (...args) => calls.push(['file',...args])});
  await view.tree({tree:'files-tree',id:'worktrees',kind:'open'},eventCx);
  await view.tree({tree:'files-tree',id:'empty',kind:'open'},eventCx);
  await view.tree({tree:'files-tree',id:'file.txt',kind:'open'},eventCx);
  await view.tree({tree:'another-tree',id:'worktrees',kind:'open'},eventCx);
  assert.deepEqual(calls,[['worktrees',false,eventCx],['file','file.txt',undefined,eventCx]]);
});

test('search results eagerly register native button actions with captured line targets', async () => {
  const search = vm.runInNewContext(`${source}\nsearch`,{
    div:() => element('div'),theme:() => ({colors:{}}),
    Button:function(id) {return element('button',{id});},
    TextField:{new:id => element('input',{id})},IconButton:{new:id => element('icon',{id})},
    Toggle:{new:(id,props) => element('toggle',{id,...props})},
  });
  const targets = [],cx = {};
  const view = {text:{},explorer:{connected:true},
    search:{query:'query',filter:'filter',case_sensitive:true,regex:false,result:{matches:[
      {path:'日本語/first.txt',line_number:7,line:'match'},
      {path:'second.txt',line_number:12,line:'other'},
    ]}},
    run:(action,eventCx) => {assert.equal(eventCx,cx); return action(cx);},
    open:async (...args) => targets.push(args),
  };
  const nodes = all(search(view));
  const sensitive = nodes.find(node => node?.props?.id === 'file-search-case');
  const regex = nodes.find(node => node?.props?.id === 'file-search-regex');
  assert.equal(sensitive.kind,'toggle');
  assert.equal(sensitive.props.variant,'button');
  assert.equal(sensitive.props.text,'Aa');
  assert.equal(sensitive.props.checked,true);
  assert.equal(regex.kind,'toggle');
  assert.equal(regex.props.text,'.*');
  assert.equal(regex.props.checked,false);
  const first = nodes.find(node => node?.props?.id === 'file-search-result-0');
  assert.equal(first.kind,'button');
  assert.equal(typeof first.props.on_click,'function');
  view.search.result = {matches:[]};
  await first.props.on_click(null,cx);
  assert.deepEqual(targets,[['日本語/first.txt',7,cx]]);
});

test('empty directories fill the sidebar without cards', () => {
  const render = vm.runInNewContext(`${source}\nrender`,{
    div:() => element('div'),theme:() => ({colors:{}}),
    PanelHeader:{new:id => element('header',{id})},
    IconButton:{new:id => element('icon',{id})},Menu:{new:id => element('menu',{id})},
    NativeContextMenu:{new:id => element('context',{id})},
    EmptyState:{new:(id,props) => element('empty',{id,...props})},
    Spinner:function() {return element('spinner');},
    ResourceTree:{new:() => assert.fail('an empty directory must not mount a tree')},
  });
  const view = {text:{files_directory_empty:'Empty folder'},search:null,
    explorer:{pages:new Map([['',{loaded:true,entries:[]}]]),connected:true,directory:() => ''},
    actions:{create:() => [],tree:() => []},
  };
  const empty = all(render(view)).find(node => node?.kind === 'empty');
  assert.deepEqual({...empty.props},{id:'files_directory_empty',variant:'list',fill_height:true,icon:'folder',label:'Empty folder'});
  view.explorer.pages.set('',{loaded:false,entries:[]});
  const nodes = all(render(view)),loading = nodes.find(node => node?.props?.id === 'files_loading');
  assert.equal(loading.props.id,'files_loading');
  assert.equal(nodes.some(node => node?.kind === 'empty'),false);
  assert.equal(loading.items[0].kind,'spinner');
});

test('empty search fills its viewport', () => {
  const search = vm.runInNewContext(`${source}\nsearch`,{
    div:() => element('div'),theme:() => ({colors:{}}),
    TextField:{new:id => element('input',{id})},IconButton:{new:id => element('icon',{id})},
    Toggle:{new:(id,props) => element('toggle',{id,...props})},
    EmptyState:{new:(id,props) => element('empty',{id,...props})},
  });
  const view = {text:{file_search_empty:'No matches',file_search_running:'Searching',file_search_partial:'Partial results'},
    explorer:{connected:true},search:{query:'retained query',filter:'retained filter',running:false,
      status:'file_search_empty',result:{matches:[]}}};
  let nodes = all(search(view)),empty = nodes.find(node => node?.kind === 'empty');
  assert.deepEqual({...empty.props},{id:'file_search_empty',variant:'list',fill_height:true,icon:'search',label:'No matches'});
  assert.equal(nodes.filter(node => node?.items?.includes('No matches')).length,0);
  for (const [status,running,label] of [['file_search_running',true,'Searching'],['file_search_partial',false,'Partial results']]) {
    Object.assign(view.search,{status,running});
    nodes = all(search(view));
    assert.equal(nodes.some(node => node?.kind === 'empty'),false);
    assert.ok(nodes.some(node => node?.items?.includes(label)));
    assert.equal(view.search.query,'retained query');
    assert.equal(view.search.filter,'retained filter');
  }
});

test('main document tabs use Shell headers; embedded tabs stay local', async () => {
  const documentSource = (await readFile(new URL('../dev.sailry.platform/desktop/document-view.js',import.meta.url),'utf8'))
    .replace(/^import .*;\n/gm,'').replace(/^export /gm,'');
  const header = vm.runInNewContext(`${documentSource}\nheader`,{
    div:() => element('div'),Header:{new:(id,props) => element('metadata',{id,...props})},
    PanelHeader:{new:id => element('header',{id})},
    NavigationTabs:{new:(id,props) => element('tabs',{id,...props})},
  });
  const view = {mode:'main',text:{files:'Files',close:'Close'},
    documents:{selected:'draft',items:() => [{id:'draft',path:'src/main.rs',dirty:true}]} };
  const main = header(view);
  assert.equal(main.kind,'metadata');
  const metadata = JSON.parse(main.props.content);
  assert.equal(metadata.title,'Files');
  assert.equal(metadata.tabs.id,'file-tabs');
  assert.equal(metadata.tabs.selected,'draft');
  assert.deepEqual(metadata.tabs.items,[{id:'draft',label:'main.rs',dirty:true,closable:true,close_label:'Close src/main.rs'}]);
  view.mode = 'embedded';
  const embedded = header(view);
  assert.equal(embedded.kind,'header');
  assert.equal(all(embedded).filter(node => node?.kind === 'tabs').length,1);
  view.mode = 'main'; view.documents.items = () => [];
  assert.equal(JSON.parse(header(view).props.content).title,'Files');
});

test('empty document content uses the full pane without a blank footer', async () => {
  const documentSource = (await readFile(new URL('../dev.sailry.platform/desktop/document-view.js',import.meta.url),'utf8'))
    .replace(/^import .*;\n/gm,'').replace(/^export /gm,'');
  const content = vm.runInNewContext(`${documentSource}\ncontent`,{
    div:() => element('div'),theme:() => ({colors:{}}),
    EmptyState:{new:(id,props) => element('empty',{id,...props})},
    StatusBar:function() {assert.fail('an empty document must not reserve a status bar');},
    NativeContextMenu:{new:() => assert.fail('an empty path must not mount actions')},
    DocumentSurface:{new:() => assert.fail('an empty document must not mount an editor')},
  });
  const view = {text:{file_empty:'Open a file'},documents:{current:() => null},
    pathItems:() => assert.fail('an empty path has no captured target')};
  const result = content(view),nodes = all(result);
  assert.equal(result.props.id,'file-document');
  assert.equal(result.items.length,1);
  assert.deepEqual(Array.from(result.items[0].props.flex_1),[]);
  assert.equal(result.items[0].items[0].kind,'empty');
  assert.equal(nodes.some(node => node?.props?.id === 'document-path'),false);
  assert.equal(nodes.some(node => node?.props?.id === 'file-toolbar'),false);
});

test('open documents retain path actions and native status content', async () => {
  const documentSource = (await readFile(new URL('../dev.sailry.platform/desktop/document-view.js',import.meta.url),'utf8'))
    .replace(/^import .*;\n/gm,'').replace(/^export /gm,'');
  const content = vm.runInNewContext(`${documentSource}\ncontent`,{
    div:() => element('div'),theme:() => ({colors:{}}),
    IconButton:{new:(id,props) => element('button',{id,...props})},
    Menu:{new:(id,props) => element('dropdown',{id,...props})},
    SegmentedTabs:{new:(id,props) => element('tabs',{id,...props})},Measure:{new:id => element('measure',{id})},
    StatusBar:function() {return element('status');},
    NativeContextMenu:{new:(id,props) => element('menu',{id,...props})},
    DocumentSurface:{new:(id,props) => element('editor',{id,...props})},
    EmptyState:{new:() => assert.fail('an open document must not mount an empty state')},
  });
  const actions = [{id:'copy',label:'Copy path'}];
  const document = {id:'draft',path:'notes.txt',position:{line:2,column:4},language:'Plain text',saving:true,wrap:true};
  const tree = content({text:{files_saving:'Saving',files_cursor_position:'Line {line}, Column {column}'},
    documents:{current:() => document},pathItems:target => {assert.equal(target,document);return actions;}});
  const nodes = all(tree),bar = nodes.find(node => node?.kind === 'status');
  const status = [bar.props.left_content,bar.props.right_content].flatMap(all);
  assert.ok(nodes.some(node => node?.props?.id === 'file-toolbar'));
  assert.deepEqual(Array.from(bar.props.h_8),[]);
  assert.equal(status.find(node => node?.props?.id === 'document-path-label').items[0],'notes.txt');
  assert.deepEqual(Array.from(status.find(node => node?.kind === 'menu').props.items),actions);
  assert.ok(nodes.some(node => node?.items?.includes('Saving')));
  assert.ok(status.some(node => node?.items?.includes('Line 2, Column 4')));
  assert.ok(status.some(node => node?.items?.includes('UTF-8')));
  assert.ok(status.some(node => node?.items?.includes('Plain text')));
});

test('populated explorers retain native tree insets and header control states', () => {
  const render = vm.runInNewContext(`${source}\nrender`,{
    div:() => element('div'),theme:() => ({colors:{}}),
    PanelHeader:{new:id => element('header',{id})},
    IconButton:{new:(id,props) => element('icon',{id,...props})},Menu:{new:(id,props) => element('menu',{id,...props})},
    NativeContextMenu:{new:id => element('context',{id})},
    ResourceTree:{new:(id,props) => element('tree',{id,...props})},
    TextField:{new:id => element('text',{id})},Toggle:{new:id => element('toggle',{id})},
  });
  const view = {text:{},search:null,
    explorer:{pages:new Map([['',{loaded:true,entries:[{path:'notes.txt'}]}]]),connected:true,
      directory:() => '',items:() => [],paths:() => [],current:null},
    actions:{create:() => [],tree:() => [],treeMenu:() => []},
  };
  const nodes = all(render(view));
  const inset = nodes.find(node => node?.items?.some(item => item?.kind === 'tree'));
  assert.ok(inset);
  assert.deepEqual(Array.from(inset.props.p_2),[]);
  assert.deepEqual(Array.from(inset.props.flex_1),[]);
  const tree=nodes.find(node=>node?.kind==='tree');
  assert.equal(tree.props.guides,true);
  assert.equal(tree.props.indent,undefined);
  assert.equal(tree.props.skip_depth,undefined);
  assert.equal(nodes.find(node => node?.props?.id === 'file-create-menu').props.size,'medium');
  assert.equal(nodes.find(node => node?.props?.id === 'file-search-toggle').props.size,'medium');
  assert.equal(nodes.find(node => node?.props?.id === 'file-search-toggle').props.selected,false);
  view.search = {};
  assert.equal(all(render(view)).find(node => node?.props?.id === 'file-search-toggle').props.selected,true);
});


async function toolbarScope() {
  const documentSource = (await readFile(new URL('../dev.sailry.platform/desktop/document-view.js',import.meta.url),'utf8'))
    .replace(/^import .*;\n/gm,'').replace(/^export /gm,'');
  return vm.runInNewContext(`${documentSource}\n({toolbar,groups,fit})`,{
    div:() => element('div'),IconButton:{new:(id,props) => element('button',{id,...props})},
    Menu:{new:(id,props) => element('menu',{id,...props})},
    SegmentedTabs:{new:(id,props) => element('tabs',{id,...props})},
    Measure:{new:id => element('measure',{id})},
  });
}

const toolbarText = {content_document:'Document',content_source:'Source',content_preview:'Preview',
  files_save:'Save',files_undo:'Undo',files_redo:'Redo',files_find:'Find',files_replace:'Find and replace',
  files_wrap:'Soft wrap',files_more:'More',files_bold:'Bold',files_italic:'Italic',files_strike:'Strikethrough',
  files_inline_code:'Inline code',files_link:'Link',files_block:'Text',files_block_paragraph:'Paragraph',
  files_block_heading:'Heading {level}',files_block_bullet:'Bulleted list',files_block_ordered:'Numbered list',
  files_block_task:'Task list',files_block_quote:'Quote',files_block_callout:'Callout',files_block_code:'Code block',
  files_insert:'Insert',files_insert_table:'Table',files_insert_image:'Image',files_insert_rule:'Divider',
  files_insert_code:'Code block',files_insert_diagrams:'Diagrams',files_insert_mermaid:'Mermaid',
  files_insert_math:'Math',files_insert_chart:'Chart',files_insert_html:'HTML block'};

const plain = value => JSON.parse(JSON.stringify(value));
const ids = nodes => nodes.filter(node => node?.kind === 'button' || node?.kind === 'menu' || node?.kind === 'tabs')
  .map(node => node.props.id);

test('markdown documents compose formatting groups and an icon mode switch', async () => {
  const {toolbar} = await toolbarScope();
  const document = {markdown:true,notebook:false,mode:'document',can_edit:true,can_save:true,dirty:true,wrap:true,
    format:{bold:true,italic:false,strike:false,code:false,link:true,block:'heading2'}};
  const nodes = all(toolbar({text:toolbarText},document,{}));
  assert.deepEqual(ids(nodes),['files_save','files_undo','files_redo','file-block-menu','files_bold',
    'files_italic','files_strike','files_inline_code','files_link',
    'block:bullet','block:ordered','block:task','block:quote','insert:table','file-insert-menu',
    'files_find','file-editor-mode']);
  const find = id => nodes.find(node => node?.props?.id === id);
  assert.equal(find('files_save').props.tone,'success');
  assert.equal(find('files_bold').props.shortcut,'secondary-b');
  assert.equal(find('files_link').props.selected,true);
  assert.equal(find('files_italic').props.selected,false);
  const block = find('file-block-menu');
  assert.equal(block.props.label,'Heading 2');
  assert.equal(block.props.caret,true);
  assert.equal(block.props.items.find(item => item.checked).id,'block:heading2');
  assert.equal(block.props.items[2].shortcut,'secondary-alt-2');
  const insert = find('file-insert-menu');
  assert.ok(insert.props.items.some(item => item.header && item.label === 'Diagrams'));
  assert.deepEqual(plain(insert.props.items.filter(item => item.id).map(item => item.id)),
    ['insert:image','insert:rule','insert:code','insert:mermaid','insert:math','insert:chart','insert:html']);
  const tabs = find('file-editor-mode');
  assert.equal(tabs.props.selected,'document');
  assert.deepEqual(plain(tabs.props.items.map(item => [item.id,item.icon,item.label])),
    [['document','eye','Document'],['source','code-xml','Source']]);
  for (const removed of ['files_copy','files_cut','files_paste']) assert.equal(find(removed),undefined);
  assert.ok(nodes.some(node => node?.kind === 'measure'));
});

test('markdown source adds soft wrap; plain code offers replace and wrap', async () => {
  const {toolbar} = await toolbarScope();
  const source = {markdown:true,notebook:false,mode:'source',can_edit:true,can_save:false,wrap:false,format:{block:null}};
  let nodes = all(toolbar({text:toolbarText},source,{}));
  assert.deepEqual(ids(nodes).slice(-3),['files_find','files_wrap','file-editor-mode']);
  assert.equal(nodes.find(node => node?.props?.id === 'file-block-menu').props.label,'Text');
  assert.equal(nodes.find(node => node?.props?.id === 'files_save').props.tone,'muted');
  const code = {markdown:false,notebook:false,mode:'source',can_edit:false,can_save:false,wrap:true,format:null};
  nodes = all(toolbar({text:toolbarText},code,{}));
  assert.deepEqual(ids(nodes),['files_save','files_undo','files_redo','files_find','files_replace','files_wrap']);
  const find = id => nodes.find(node => node?.props?.id === id);
  assert.equal(find('files_replace').props.disabled,true);
  assert.equal(find('files_find').props.disabled,false);
  assert.equal(find('files_wrap').props.selected,true);
});

test('notebook previews keep Save, Find and a Preview switch', async () => {
  const {toolbar} = await toolbarScope();
  const document = {notebook:true,markdown:false,mode:'document',can_edit:false,can_save:false,wrap:true,format:null};
  let nodes = all(toolbar({text:toolbarText},document,{}));
  assert.deepEqual(ids(nodes),['files_save','files_find','file-editor-mode']);
  const tabs = nodes.find(node => node?.props?.id === 'file-editor-mode');
  assert.equal(tabs.props.items[0].label,'Preview');
  document.mode = 'source'; document.can_edit = true;
  nodes = all(toolbar({text:toolbarText},document,{}));
  assert.deepEqual(ids(nodes),['files_save','files_undo','files_redo','files_find','files_replace','files_wrap','file-editor-mode']);
});

test('narrow toolbars collapse controls right to left into More', async () => {
  const {toolbar,groups,fit} = await toolbarScope();
  const document = {markdown:true,notebook:false,mode:'source',can_edit:true,can_save:false,wrap:true,
    format:{bold:false,italic:true,strike:false,code:false,link:false,block:'paragraph'}};
  const all_groups = groups({text:toolbarText},document);
  assert.equal(fit(all_groups,undefined).hidden.length,0);
  assert.equal(fit(all_groups,10000).hidden.length,0);
  const narrow = fit(all_groups,150);
  assert.deepEqual(plain(narrow.visible.flat().map(item => item.id)),['files_italic','files_strike','files_inline_code']);
  assert.deepEqual(plain(narrow.hidden.flat().map(item => item.id)),['files_link','block:bullet','block:ordered',
    'block:task','block:quote','insert:table','files_insert','files_find','files_wrap']);
  const nodes = all(toolbar({text:toolbarText,toolbarWidth:300},document,{}));
  const shown = ids(nodes);
  for (const kept of ['files_save','files_undo','files_redo','file-block-menu','files_bold','file-editor-mode','file-more-menu'])
    assert.ok(shown.includes(kept),kept);
  const more = nodes.find(node => node?.props?.id === 'file-more-menu');
  assert.equal(more.props.icon,'ellipsis');
  const insert = more.props.items.find(item => item.children);
  assert.equal(insert.label,'Insert');
  assert.ok(more.props.items.some(item => item.id === 'files_wrap' && item.checked));
  assert.equal(all(toolbar({text:toolbarText,toolbarWidth:4000},document,{})).some(node => node?.props?.id === 'file-more-menu'),false);
});

test('toolbar events dispatch document actions with their parameters', async () => {
  const main = (await readFile(new URL('../dev.sailry.platform/desktop/main.js',import.meta.url),'utf8'))
    .replace(/^import[\s\S]*?;\n/gm,'').replace('export default class','class');
  const Files = vm.runInNewContext(`${main}\nFiles`,{View:class {}});
  const view = new Files(), calls = [], cx = {};
  Object.assign(view,{run:action => action(cx),documents:{
    action:async (kind,_cx,options) => calls.push(options ? [kind,options] : [kind]),
    save:async () => calls.push(['save'])}});
  for (const id of ['files_save','files_bold','files_inline_code','block:heading3','insert:mermaid','files_wrap','files_replace','unknown'])
    view.toolbar(id,cx);
  view.control('file-editor-mode',cx,'source');
  view.control('file-toolbar-width',cx,420);
  assert.equal(view.toolbarWidth,420);
  assert.deepEqual(plain(calls),[['save'],['bold'],['inline_code'],['block',{block:'heading3'}],['insert',{block:'mermaid'}],
    ['wrap'],['replace'],['source']]);
});

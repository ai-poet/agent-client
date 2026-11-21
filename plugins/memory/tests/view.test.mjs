import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';

const source = (await readFile(new URL('../dev.sailry.platform/desktop/view.js',import.meta.url),'utf8'))
  .replace(/^import .*;\n/gm,'').replace(/^export /gm,'');
const plain = value => JSON.parse(JSON.stringify(value));

function fixture() {
  const nodes = new Map(),calls = [];
  function element(kind='div',id=null,props={}) {
    const value = {kind,selector:id,props,items:[],style:{}};
    const proxy = new Proxy(value,{get(target,key) {
      if(key==='id')return id=>{target.selector=id;nodes.set(id,proxy);return proxy;};
      if(key==='child')return child=>{target.items.push(child);return proxy;};
      if(key==='children')return children=>{target.items.push(...children);return proxy;};
      if(key in target)return target[key];
      if(key==='on_click')return callback=>{target.on_click=callback;return proxy;};
      return (...args)=>{target.style[key]=args;return proxy;};
    }});
    if(id)nodes.set(id,proxy);
    return proxy;
  }
  const functions = vm.runInNewContext(`${source}\n({catalog,configuration,editor,renderView:render})`,{
    div:()=>element(),theme:()=>({colors:{muted_foreground:'muted'}}),
    Button:function(id){return element('Button',id);},
    Switch:function(id){return element('Switch',id);},
    Tab:function(id){return element('Tab',id);},
    TabBar:function(id){return element('TabBar',id);},
    Field:function(){return element('Field');},
    Tag:function(){return element('Tag');},
    DropdownMenu:function(id){return element('DropdownMenu',id);},
    SelectField:{new:(id,props)=>element('SelectField',id,props)},
    SegmentedTabs:{new:(id,props)=>element('SegmentedTabs',id,props)},
    TextField:{new:(id,props)=>element('TextField',id,props)},
    SettingsGroup:{new:(id,props)=>element('SettingsGroup',id,props)},
    EmptyState:{new:(id,props)=>element('EmptyState',id,props)},
    Loading:{new:(id,props)=>element('Loading',id,props)},
    IconButton:{new:(id,props)=>element('IconButton',id,props)},
    Modal:{new:(id,props)=>element('Modal',id,props)},
    window:{viewport_size:()=>({width:1280,height:820})},
  });
  const view = {text:{settings_memory:'Memories',memory_empty:'No memories',settings_edit:'Edit',settings_delete:'Delete'},
    saved:{},results:[],reviews:[],loading:false,busy:()=>false,kindLabel:()=> 'Feedback',projectLabel:()=> 'Global',
    edit:entry=>calls.push(['edit',entry]),remove:entry=>calls.push(['remove',entry]),draft:null,
    configuration:{pending:false,request:null,error:null},query:'query',scope:'all',projects:[],entries:[],connected:true,
    dialogId:0,editing:null,removal:null};
  return {view,nodes,calls,functions,render(){nodes.clear();return functions.catalog(view);}};
}

test('settings choices use real selects and editor tabs fill their form control',()=>{
  const setup=fixture(),view=setup.view;
  view.text={...view.text,memory_context_budget:'Budget',memory_review_age:'Review',memory_title:'Title',memory_kind:'Type',memory_body:'Content'};
  view.draft={enabled:true,auto_write:true,context_bytes:8192,review_after_days:90};
  view.configuration={pending:false,request:null,error:null};view.connected=true;
  setup.functions.configuration(view);
  const budget=setup.nodes.get('memory-budget'),age=setup.nodes.get('memory-review-age');
  assert.equal(budget.kind,'SelectField');assert.equal(age.kind,'SelectField');
  assert.equal(budget.props.selected,'8');assert.equal(age.props.selected,'90');
  assert.deepEqual(plain(budget.props.items.map(item=>item.id)),['2','4','8','16','32','64']);
  view.dialogId=4;view.entries=[];
  view.editing={entry:{summary:{id:'draft',kind:'feedback',project:null,revision:0}},title:'title',body:'body',sources:[],loading:false,pending:false,request:null};
  setup.functions.editor(view);
  const tabs=setup.nodes.get('memory-kind-4');
  assert.equal(tabs.kind,'SegmentedTabs');assert.equal(tabs.props.selected,'feedback');
  assert.equal(tabs.props.fill,true);
  assert.deepEqual(plain(tabs.props.items.map(item=>item.id)),['user','feedback','project','reference']);
  view.configuration.pending=true;setup.functions.configuration(view);
  assert.equal(setup.nodes.get('memory-budget').props.disabled,true);
  assert.equal(setup.nodes.get('memory-review-age').props.disabled,true);
});

test('memory forms use modal title and footer slots',()=>{
  const setup=fixture(),view=setup.view;
  view.text={...view.text,memory_editor:'Edit memory',memory_create:'New memory',memory_title:'Title',memory_kind:'Type',
    memory_body:'Content',memory_retry:'Retry',memory_merge:'Merge',settings_save:'Save',settings_cancel:'Cancel',
    memory_delete:'Delete memory',settings_delete:'Delete'};
  view.editing={entry:{summary:{id:'draft',kind:'feedback',project:null,revision:0,archived:false}},title:'title',body:'body',
    sources:[],loading:false,pending:false,request:null};
  setup.functions.renderView(view);
  let modal=setup.nodes.get('memory-dialog-0');
  assert.deepEqual(plain(modal.props),{open:true,form:true,title:'New memory',width:640});
  assert.deepEqual(modal.items.map(item=>item.selector),['memory-editor',null]);
  view.editing=null;view.removal={summary:{title:'Keep this target'},pending:false,request:null};
  setup.functions.renderView(view);modal=setup.nodes.get('memory-dialog-0');
  assert.deepEqual(plain(modal.props),{open:true,form:true,title:'Delete memory',width:448});
  assert.deepEqual(modal.items.map(item=>item.selector),['memory-removal',null]);
  assert.deepEqual(modal.items[0].items,['Keep this target']);
  const confirm=setup.nodes.get('memory-delete-confirm');
  assert.deepEqual(confirm.style.danger,[]);assert.equal('primary' in confirm.style,false);
  assert.deepEqual(confirm.style.label,['Delete']);
});

test('settled empty catalogs keep their card during refresh',()=>{
  const setup=fixture(),group=setup.render();
  assert.equal(group.kind,'SettingsGroup');
  assert.deepEqual(plain(group.props),{heading:false});
  const empty=setup.nodes.get('memory-empty');
  assert.deepEqual(plain(empty.props),{variant:'list',icon:'inbox',label:'No memories'});
  assert.equal(group.items[0],empty);
  setup.view.loading=true;setup.render();
  assert.equal(setup.nodes.has('memory-empty'),true);
  assert.equal(setup.nodes.has('memory-loading'),false);
});

test('initial reads reserve configuration and list space',()=>{
  const setup=fixture();
  setup.view.saved=null;setup.view.draft=null;setup.view.loading=true;
  setup.functions.configuration(setup.view);
  const fields=setup.nodes.get('memory_configuration');
  assert.equal(fields.kind,'SettingsGroup');
  assert.equal(setup.nodes.get('memory-configuration-loading').kind,'Loading');
  setup.render();
  assert.equal(setup.nodes.get('memory-loading').kind,'Loading');
  assert.equal(setup.nodes.has('memory-empty'),false);
  setup.view.loading=false;setup.view.error='memory_failed';setup.view.connected=true;
  setup.render();
  assert.equal(setup.nodes.has('memory-loading'),false);
  assert.equal(setup.nodes.has('memory-empty'),false);
  assert.ok(setup.nodes.has('memory-search-retry'));
});

test('catalog rows use named icon controls without empty content',()=>{
  const setup=fixture(),entry={id:'retained',title:'Retained memory',kind:'feedback',project:null};
  setup.view.results=[entry];setup.render();
  assert.equal(setup.nodes.has('memory-empty'),false);
  assert.ok(setup.nodes.has('memory-retained'));
  const edit=setup.nodes.get('memory-edit-retained'),remove=setup.nodes.get('memory-delete-retained');
  assert.equal(edit.kind,'IconButton');assert.equal(remove.kind,'IconButton');
  assert.deepEqual(plain(edit.props),{icon:'reicon:newicons/edit',label:'Edit',disabled:false});
  assert.deepEqual(plain(remove.props),{icon:'reicon:ui/trash2',label:'Delete',disabled:false});
  assert.equal(edit.props.show_label,undefined);assert.equal(remove.props.show_label,undefined);
  assert.deepEqual(setup.calls,[]);
  setup.view.loading=true;const group=setup.render();
  assert.equal(setup.nodes.has('memory-loading'),false);
  assert.ok(setup.nodes.has('memory-retained'));
  assert.deepEqual(plain(group.props),{heading:false});
  setup.view.busy=()=>true;setup.render();
  assert.equal(setup.nodes.get('memory-edit-retained').props.disabled,true);
  assert.equal(setup.nodes.get('memory-delete-retained').props.disabled,true);
});

test('catalog failures keep existing memories without a retry bar',()=>{
  const setup=fixture();setup.view.error='memory_failed';
  setup.view.results=[{id:'retained',title:'Retained memory',kind:'feedback',project:null}];setup.render();
  assert.equal(setup.nodes.has('memory-search-retry'),false);
  assert.ok(setup.nodes.has('memory-retained'));
});

test('scope tags retain global and project labels',()=>{
  const setup=fixture();
  setup.view.projectLabel=id=>id===null ? 'Global' : 'Project Alpha';
  setup.view.results=[
    {id:'global',title:'Global preference',kind:'feedback',project:null},
    {id:'project',title:'Project preference',kind:'feedback',project:'project-id'},
  ];
  setup.render();
  for(const [id,label] of [['global','Global'],['project','Project Alpha']]) {
    const description=setup.nodes.get(`memory-${id}`).items[0].items.at(-1);
    assert.deepEqual(plain(description.items[0].items),['Feedback']);
    const tag=description.items[1];
    assert.equal(tag.kind,'Tag');
    assert.deepEqual(plain(tag.items),[label]);
    assert.deepEqual(plain(tag.style.size),['small']);
    assert.deepEqual(plain(tag.style.variant),['secondary']);
  }
});

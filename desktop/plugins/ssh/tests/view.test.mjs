import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import {messages} from '../dev.sailry.platform/desktop/locales.js';

const source=(await readFile(new URL('../dev.sailry.platform/desktop/view.js',import.meta.url),'utf8'))
  .replace(/^import .*;\n/gm,'').replace(/^export /gm,'');
const plain=value=>JSON.parse(JSON.stringify(value));

function fixture() {
  const nodes=new Map();
  function element(kind='div',id=null,props={}) {
    const value={kind,selector:id,props,items:[],style:{}};
    const proxy=new Proxy(value,{get(target,key) {
      if(key==='id')return id=>{target.selector=id;nodes.set(id,proxy);return proxy;};
      if(key==='child')return child=>{target.items.push(child);return proxy;};
      if(key==='children')return children=>{target.items.push(...children);return proxy;};
      if(key in target)return target[key];
      return (...args)=>{target.style[key]=args;return proxy;};
    }});
    if(id)nodes.set(id,proxy);
    return proxy;
  }
  const adapters=Object.fromEntries(['Workspace','EmptyState','Loading','Picker','SelectableRow','IconButton',
    'NativeContextMenu','Modal','ActionScope','Loading','Header','Conversation','Terminal','PathBar','ResourceTree','TextField','PanelHeader']
    .map(kind=>[kind,{new:(id,props)=>element(kind,id,props)}]));
  const {render}=vm.runInNewContext(`${source}\n({render})`,{
    div:()=>element(),...adapters,Icon:function(path){return element('Icon',null,{path});},Button:function(id){return element('Button',id);},Progress:function(id){return element('Progress',id);},Spinner:function(){return element('Spinner');},
    theme:()=>({colors:{muted_foreground:'muted'}}),
  });
  const view={text:messages('en'),profiles:[],terminals:[],terminal:null,home:false,loaded:false,
    pending:null,error:null,editor:null,confirmation:null,fileEdit:null,transfer:null,commandOpen:false,
    jobs:[],dialog:0,picker:false,pickerRevision:0,pickerProfile:null,locked:()=>false,connectionLocked:()=>false,activeProfile:()=>null};
  return {view,nodes,render(){nodes.clear();return render(view);}};
}

test('initial reads reserve the workspace instead of showing landing',()=>{
  const setup=fixture();
  for(const error of [null,'unavailable']) {
    setup.view.error=error;setup.render();
    assert.equal(setup.nodes.has('ssh-loading-rows'),false);
    assert.deepEqual(plain(setup.nodes.get('ssh-loading').style.size_full),[]);
    assert.equal(setup.nodes.has('ssh_workspace_empty'),false);
    assert.ok(setup.nodes.has('ssh-header'));assert.ok(setup.nodes.has('ssh-workspace'));
  }
  setup.view.loaded=true;setup.render();
  assert.equal(setup.nodes.has('ssh-loading-rows'),false);
  assert.ok(setup.nodes.has('ssh_workspace_empty'));
});

test('restored terminals and their tabs remain during refresh failures',()=>{
  const setup=fixture();
  setup.view.loaded=true;
  setup.view.profiles=[{id:'profile',name:'Saved host',username:'user',host:'host',port:22}];
  setup.view.terminals=[{id:'terminal',ssh:'profile'}];setup.view.terminal='terminal';
  for(const error of [null,'unavailable']) {
    setup.view.error=error;setup.render();
    assert.equal(setup.nodes.get('ssh-terminal-terminal').kind,'Terminal');
    assert.equal(setup.nodes.has('ssh-loading-rows'),false);
    assert.equal(setup.nodes.has('ssh_workspace_empty'),false);
    const header=JSON.parse(setup.nodes.get('ssh-header').props.content);
    assert.equal(header.tabs.selected,'terminal');
    assert.equal(header.tabs.items[0].label,'Saved host');
  }
});

test('opening shows progress without landing or recovery buttons',()=>{
  const setup=fixture();
  setup.view.loaded=true;
  setup.view.profiles=[{id:'profile',name:'Saved host',username:'user',host:'host',port:22}];
  setup.view.pending={request:{command:{kind:'open_ssh_terminal'},unknown:false}};
  setup.render();
  assert.ok(setup.nodes.has('ssh-connecting'));
  assert.equal(setup.nodes.has('ssh_workspace_empty'),false);
  assert.equal(setup.nodes.has('ssh-new'),false);
  assert.equal(setup.nodes.has('ssh-pending'),false);
  assert.equal(setup.nodes.has('ssh-inspect'),false);
  assert.equal(setup.nodes.has('ssh-stop'),false);

  setup.view.pending.request.unknown=true;
  setup.render();
  assert.equal(setup.nodes.has('ssh-connecting'),false);
  assert.ok(setup.nodes.has('ssh_workspace_empty'));
  assert.ok(setup.nodes.has('ssh-open-profile'));
  assert.equal(setup.nodes.has('ssh-pending'),false);
  assert.equal(setup.nodes.has('ssh-inspect'),false);
  assert.equal(setup.nodes.has('ssh-stop'),false);
});

test('files use breadcrumbs branches and keep transfers at the bottom',()=>{
  const setup=fixture(),profile={id:'host',name:'Host'},files={profile,path:'/remote',pending:false,editing:false,revision:1,entries:[{name:'a',kind:'file'}],selected:['/remote/a','/remote/b'],current:'/remote/a',
    segments:()=>[{label:'/',path:'/'},{label:'remote',path:'/remote'}],items:menu=>[{id:'/remote/a',label:'a',menu:menu('/remote/a')}],input:'path-field'};
  Object.assign(setup.view,{loaded:true,profiles:[profile],terminals:[{id:'terminal',ssh:'host'}],terminal:'terminal',activeProfile:()=>profile,files:()=>files,
    fileMenu:path=>[{id:`menu:${path}`,label:'Menu'}],jobs:[{id:'job',profile:'host',source:'source.txt',can_replace:true,can_cancel:true}]});
  setup.render();
  assert.deepEqual(plain(setup.nodes.get('ssh-files-pathbar').props),{segments:['/','remote'],disabled:false});
  assert.equal(setup.nodes.has('ssh-files-edit-path'),false);
  const row=setup.nodes.get('ssh-files-path-row');
  assert.equal(setup.nodes.get('ssh-files-path').items[0],row,'the divider is the inner row border, inset by the header padding');
  assert.deepEqual(plain(row.style.border_b_1),[]);assert.ok('border_color' in row.style);
  let edits=0;setup.view.editPath=()=>edits++;row.style.on_click[0](null,{});assert.equal(edits,1,'a blank header click edits the path');
  const tree=setup.nodes.get('ssh-files').props;
  assert.equal(tree.variant,undefined);assert.equal(tree.guides,true);assert.equal(tree.open,'double');assert.equal(tree.external_files,true);assert.deepEqual(plain(tree.selected),files.selected);
  assert.deepEqual(JSON.parse(tree.items)[0].menu,[{id:'menu:/remote/a',label:'Menu'}]);
  assert.deepEqual(plain(setup.nodes.get('ssh-files-root').props.items),[{id:'menu:',label:'Menu'}]);
  const panel=setup.nodes.get('ssh-files-panel');assert.equal(panel.items.length,3);assert.equal(panel.items[0].selector,'ssh-files-path');assert.equal(panel.items.at(-1).selector,'ssh-transfer-job');
  assert.ok(setup.nodes.has('ssh-replace-job'));assert.equal(setup.nodes.get('ssh-replace-job').style.label[0],messages('en').ssh_files_replace);
  for(const id of ['ssh-files-header','ssh-files-parent','ssh-files-count','ssh-files-more'])assert.equal(setup.nodes.has(id),false,id);
  assert.equal(panel.items.some(item=>item.items?.includes('%{count} items')),false);
});

test('path editing keeps the manual input and hides its breadcrumb row controls',()=>{
  const setup=fixture(),profile={id:'host'},files={profile,pending:false,editing:true,selected:[],current:null,input:'path-field',segments:()=>[],items:()=>[]};
  Object.assign(setup.view,{loaded:true,terminals:[{id:'terminal',ssh:'host'}],terminal:'terminal',activeProfile:()=>profile,files:()=>files,fileMenu:()=>[]});
  setup.render();assert.equal(setup.nodes.get('path-field').kind,'TextField');assert.equal(setup.nodes.get('path-field').props.disabled,false);
  assert.equal(setup.nodes.has('ssh-files-pathbar'),false);assert.equal(setup.nodes.has('ssh-files-refresh'),false);
  assert.equal('on_click' in setup.nodes.get('ssh-files-path-row').style,false,'clicks inside the editor stay with the field');
  files.pending=true;setup.render();assert.equal(setup.nodes.get('path-field').props.disabled,true);
});

test('file panel replaces its tree while loading, empty or failed',()=>{
  const setup=fixture(),profile={id:'host'},files={profile,path:'/',pending:true,editing:false,revision:0,entries:[],error:null,selected:[],current:null,input:'path-field',segments:()=>[{label:'/',path:'/'}],items:()=>[]};
  Object.assign(setup.view,{loaded:true,terminals:[{id:'terminal',ssh:'host'}],terminal:'terminal',activeProfile:()=>profile,files:()=>files,fileMenu:()=>[]});
  setup.render();assert.ok(setup.nodes.has('ssh-files-loading'));assert.ok(setup.nodes.has('ssh-files-busy'));assert.equal(setup.nodes.has('ssh-files'),false);
  Object.assign(files,{pending:false,revision:1});setup.render();
  assert.ok(setup.nodes.has('files_directory_empty'));assert.equal(setup.nodes.has('ssh-files-loading'),false);assert.equal(setup.nodes.get('ssh-files-refresh').props.disabled,false);
  Object.assign(files,{error:'ssh_files_failed'});setup.render();
  assert.ok(setup.nodes.has('ssh-files-retry'));assert.equal(setup.nodes.has('files_directory_empty'),false);
});

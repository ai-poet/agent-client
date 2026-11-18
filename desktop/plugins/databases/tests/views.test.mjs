import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFile} from 'node:fs/promises';

const plain=value=>JSON.parse(JSON.stringify(value));

async function fixture(packageName) {
  let phase='render';
  const buttons=new Map(),nodes=new Map();
  const buttonMethods=new Set(['label','tooltip','loading','size','disabled','outline','primary','secondary','danger','success','warning','ghost','link','compact',
    'w_full','max_w_96']);
  function element(type='div',id=null,props={}) {
    const value={type,selector:id,nodes:[],style:{},props};
    let proxy;
    proxy=new Proxy(value,{get(target,key) {
      if(key in target)return target[key];
      if(key==='id')return id=>{target.selector=id;nodes.set(id,proxy);return proxy;};
      if(key==='child')return child=>{target.nodes.push(child);return proxy;};
      if(key==='children')return children=>{target.nodes.push(...children);return proxy;};
      if(['on_click','on_change','on_key_down'].includes(key))return callback=>{
        assert.equal(phase,'render','callbacks require an open snapshot generation');
        target[key]=callback;if(key==='on_click')buttons.set(target.selector,target);return proxy;
      };
      if(type==='Button')assert.ok(buttonMethods.has(key),`Button.${String(key)} is not exported by the pinned script API`);
      return (...args)=>{target.style[key]=args;return proxy;};
    }});
    if(id)nodes.set(id,proxy);
    return proxy;
  }
  const native=type=>({new:(id,props)=>element(type,id,props)});
  const source=(await readFile(new URL(`../../${packageName}/dev.sailry.platform/desktop/view.js`,import.meta.url),'utf8'))
    .replace(/^import[\s\S]*?;\n/gm,'').replace(/^export /gm,'');
  const names=['Button','Icon','Spinner','Pagination','Tab','TabBar','Resizable','ResizablePanel','DropdownMenu','VForm','Field','Progress'];
  const constructors=Object.fromEntries(names.map(name=>[name,function(id){return element(name,id);}]))
  const globals=Object.fromEntries(['Workspace','PanelHeader','PathBar','Header','EmptyState','Loading','InputSurface','Picker','SelectableRow','IconButton','SelectField','Tooltip','ResourceTree','DataTable','Modal','NativeContextMenu','Conversation','Terminal','TextField','ActionScope'].map(name=>[name,native(name)]));
  const functions=vm.runInNewContext(`${source}\n({picker,landing,render${packageName==='databases'?',log,results,sqlEditor,sqlFooter,historyPanel':''}})`,{
    div:()=>element(),...constructors,...globals,
    theme:()=>({colors:{muted_foreground:'muted',foreground:'foreground',border:'border'}}),
    window:{viewport_size:()=>({width:1440,height:940})},
    command:(kind,data)=>({kind,data}),cell:value=>String(value.value),readText:value=>value,
    historyEntries:logs=>[...logs].reverse(),historyRevision:(profile,logs)=>`${profile}:${logs.length}:${logs.at(-1)?.id??''}`,
    parent:path=>path.split('/').slice(0,-1).join('/')||'/',failure:()=>'',format:(text,values)=>`${text}:${JSON.stringify(values)}`,
    editor:()=>element('editor',`${packageName==='databases'?'db':'ssh'}-editor`),
    editorFooter:()=>element('editorFooter',`${packageName==='databases'?'db':'ssh'}-editor-footer`),
  });
  const cx={notify(){},stop_propagation(){}};
  return {functions,buttons,nodes,cx,layout(){phase='event';},click(id){const button=buttons.get(id);assert.ok(button,`missing callable ${id}`);button.on_click({},cx);}};
}

function view(packageName,profiles=[]) {
  const owner={text:new Proxy({},{get:(_,key)=>key}),profiles,loaded:true,locked:()=>false,dialog:0,
    picker:false,pickerProfile:null,pickerRevision:0,pending:null,error:null,status:null,editor:null,confirmation:null,
    terminals:[],terminal:null,files:()=>null,commandOpen:false,transfer:null,fileEdit:null,outcome:null,
    opened:null,catalog:{loading:new Set(),names:[],failed:new Set(),items:()=>[],database:null,current:null},
    result:null,page:0,epoch:0,sqlOpen:false,historyPage:0,logs:[],sql:'',logFocus:{},rowMenu:()=>[],
  };
  owner.connectionLocked=()=>owner.locked();
  owner.profile=()=>profiles.find(profile=>profile.id===owner.opened);
  owner.activeProfile=()=>profiles.find(profile=>profile.id===owner.terminals.find(info=>info.id===owner.terminal)?.ssh);
  return owner;
}

const profiles=[{id:'first',revision:2,name:'First',username:'one',host:'host',port:22,connection:{kind:'sqlite',path:'/fixture/first.sqlite3'}},
  {id:'second',revision:7,name:'Second',username:'two',host:'host',port:2222,connection:{kind:'network',engine:'postgres',username:'two',host:'host',port:5432}}];

for(const packageName of ['databases','ssh']) {
  const prefix=packageName==='databases'?'db':'ssh';
  test(`${packageName} operation outcomes never add feedback to landing or editor bodies`,async()=>{
    for(const editing of [false,true]){
      const setup=await fixture(packageName),owner=view(packageName,profiles);
      owner.error={message:'Raw failure must stay in toast'};owner.status='db_connected';owner.outcome={kind:'connected'};
      if(editing)owner.editor={original:{revision:0}};
      const root=setup.functions.render(owner),strings=[];
      const visit=node=>{if(typeof node==='string')strings.push(node);else for(const child of node?.nodes??[])visit(child);};visit(root);
      assert.equal(setup.nodes.has(`${prefix}-error`),false);assert.equal(setup.nodes.has(`${prefix}-connected`),false);assert.equal(setup.nodes.has('db-status'),false);
      assert.equal(strings.includes('Raw failure must stay in toast'),false);assert.equal(strings.includes('db_connected'),false);assert.equal(strings.includes('ssh_connected'),false);
      assert.ok(setup.nodes.has(`${prefix}_workspace_empty`),'landing content survives the failure');
      if(editing)assert.equal(setup.nodes.get(`${prefix}-dialog-0`).nodes[0].nodes.length,1,'the draft body contains only its editor');
    }
  });
  test(`${packageName} landing uses a native empty state with only new and open actions`,async()=>{
    const setup=await fixture(packageName),owner=view(packageName,profiles);
    setup.functions.render(owner);setup.layout();
    assert.ok(setup.nodes.has(`${prefix}_workspace_empty`));
    assert.equal(setup.nodes.has(`${prefix}-connections-close`),false);
    assert.equal(setup.buttons.size,0,'landing actions use native control events');
    const add=setup.nodes.get(`${prefix}-new`);
    assert.equal(add.type,'IconButton');
    assert.deepEqual(plain(add.props),{icon:'plus',label:`${prefix}_new`,show_label:true,full_width:true,variant:'primary',size:'medium',disabled:false});
    const empty=setup.nodes.get(`${prefix}_workspace_empty`),wrapper=empty.nodes.find(node=>node.nodes?.[0]===add);
    assert.deepEqual(plain(wrapper.style.w_full),[]);assert.deepEqual(plain(wrapper.style.max_w_96),[]);
    assert.deepEqual(plain(setup.nodes.get(`${prefix}-open-second`).props),{variant:'card',label:'Second',disabled:false,selected:false});
    const workspace=setup.nodes.get(packageName==='ssh'?'ssh-workspace':'database-workspace');
    assert.deepEqual(plain(workspace.props),{has_details:false,details_label:packageName==='ssh'?'ssh_assistant':'connection_assistant',min_navigation_width:200,min_details_width:320,navigation:'none'});
    assert.equal(workspace.nodes.length,3);
    assert.equal(setup.nodes.get(`${prefix}-picker-0`).props.open,false);
    assert.equal(setup.nodes.get(`${prefix}-dialog-0`).props.open,false);
    const header=JSON.parse(setup.nodes.get(packageName==='ssh'?'ssh-header':'database-header').props.content);
    assert.equal(header.title,packageName==='ssh'?'ssh':'database');
    const actions=packageName==='databases'?header.actions.slice(1):header.actions;
    if(packageName==='databases')assert.deepEqual(header.actions[0],{id:'db-refresh-catalog',label:'refresh',icon:'icons/rotate-cw.svg',disabled:true});
    assert.equal(actions[0].id,`${prefix}-connections-menu`);
    assert.deepEqual(actions.map(action=>action.icon),[packageName==='ssh'?'icons/network.svg':'icons/hard-drive.svg','icons/house.svg'],
      'Header actions pass complete asset paths to the native Icon');
    assert.deepEqual(actions[1],{id:`${prefix}-home`,label:'home',icon:'icons/house.svg',disabled:true});
    const menu=setup.nodes.get(`${prefix}-connection-second`);
    assert.equal(menu.type,'NativeContextMenu');
    assert.deepEqual(plain(menu.props.items),['edit','remove'].map(action=>({id:`connection:${JSON.stringify({id:'second',revision:7,action})}`,label:action==='edit'?'connection_action_edit':'settings_delete',enabled:true})));
  });

  test(`${packageName} Home keeps resource tabs and shows saved connections`,async()=>{
    const setup=await fixture(packageName),owner=view(packageName,profiles);
    owner.home=true;owner.opened='first';owner.terminals=[{id:'terminal',ssh:'first'}];owner.terminal='terminal';
    setup.functions.render(owner);
    assert.ok(setup.nodes.has(`${prefix}_workspace_empty`));assert.ok(setup.nodes.has(`${prefix}-connection-first`));
    assert.equal(setup.nodes.has(packageName==='ssh'?'ssh-terminal-terminal':'database-results'),false);
    assert.equal(setup.nodes.has(packageName==='ssh'?'ssh-assistant-first':'database-assistant-first'),false);
    const header=JSON.parse(setup.nodes.get(packageName==='ssh'?'ssh-header':'database-header').props.content);
    assert.equal(header.tabs.selected,null);assert.equal(header.tabs.items[0].id,packageName==='ssh'?'terminal':'first');
  });

  test(`${packageName} picker uses searchable native Command rows and a separate dialog lifecycle`,async()=>{
    const setup=await fixture(packageName),owner=view(packageName,profiles);
    owner.picker=true;owner.pickerRevision=4;
    setup.functions.render(owner);
    let picker=setup.nodes.get(`${prefix}-picker-4`);
    assert.equal(picker.props.mode,'command');assert.equal(picker.props.title,'connection_search');assert.equal(picker.props.empty,'connection_search_empty');
    assert.deepEqual(plain(picker.props.items.map(item=>item.id)),['connection-choice-0','connection-choice-1','connection-add']);
    assert.equal(picker.props.items[1].submenu,true);
    assert.deepEqual(plain(picker.props.items[1].value),{kind:'profile',id:'second'});
    assert.equal(setup.nodes.get(`${prefix}-dialog-0`).props.open,false,'native Picker must not be nested in Modal');
    owner.pickerProfile='second';owner.pickerRevision++;
    setup.functions.picker(owner);picker=setup.nodes.get(`${prefix}-picker-5`);
    assert.deepEqual(plain(picker.props.items.map(item=>item.id)),['connection-action-open','connection-action-edit','connection-action-check',...(packageName==='ssh'?['connection-action-transfer','connection-action-run']:[]),'connection-action-remove','connection-back']);
  });

  test(`${packageName} editor uses native titled form body and footer slots`,async()=>{
    for(const revision of [0,3]) {
      const setup=await fixture(packageName),owner=view(packageName,profiles);
      owner.editor={original:{revision}};owner.dialog=7;
      setup.functions.render(owner);
      const modal=setup.nodes.get(`${prefix}-dialog-7`);
      assert.deepEqual(plain(modal.props),{open:true,form:true,title:`${prefix}_${revision?'edit':'new'}`,width:560});
      assert.equal(modal.nodes.length,2);
      assert.equal(modal.nodes[0].nodes[0].selector,`${prefix}-editor`);
      assert.equal(modal.nodes[1].selector,`${prefix}-editor-footer`);
      assert.equal('w' in modal.nodes[0].style,false,'native form owns the width');
      assert.equal('pt_4' in modal.nodes[0].style,false,'native form owns body top padding');
    }
  });

  test(`${packageName} confirmations use native titled form body and footer slots`,async()=>{
    const setup=await fixture(packageName),owner=view(packageName,profiles);
    owner.confirmation={label:'settings_delete',text:'Delete the retained connection'};owner.dialog=8;
    setup.functions.render(owner);
    const modal=setup.nodes.get(`${prefix}-dialog-8`);
    assert.deepEqual(plain(modal.props),{open:true,form:true,title:'settings_delete',width:packageName==='databases'?568:528});
    assert.equal(modal.nodes.length,2);
    assert.equal(modal.nodes[0].selector,`${prefix}-confirm`);
    assert.equal(modal.nodes[1].nodes.length,2);
    assert.equal(modal.nodes[0].nodes.includes('Delete the retained connection'),false);
    assert.equal(modal.nodes[0].nodes[0].nodes[0],'Delete the retained connection');
  });
}

test('database catalog failures retain trees and header refresh without duplicate retry',async()=>{
  const setup=await fixture('databases'),owner=view('databases',profiles);
  owner.opened='first';owner.catalog.failed.add('');
  setup.functions.render(owner);
  assert.ok(setup.nodes.has('database-tree'));
  const header=JSON.parse(setup.nodes.get('database-header').props.content);
  assert.deepEqual(header.actions.map(action=>action.id),['db-refresh-catalog','db-connections-menu','db-home']);
  assert.equal(header.actions[0].disabled,false);
  assert.equal(setup.nodes.has('database-catalog-header'),false);
  assert.equal(setup.nodes.has('db-refresh-catalog'),false,'refresh belongs to the shared header');
  assert.equal(setup.nodes.has('db-catalog-retry'),false);
});

test('SSH transfer, trust, and command dialogs use native titled form slots',async()=>{
  for(const kind of ['transfer','trust','command']) {
    const setup=await fixture('ssh'),owner=view('ssh',profiles);
    owner.dialog=5;
    if(kind==='transfer') {
      owner.transfer={worktree:'tree',inputs:['source','destination']};
      owner.worktrees=[{id:'tree',project:'project',path:'/fixture/tree'}];owner.projects=[{id:'project',name:'Fixture'}];
    } else if(kind==='trust')owner.outcome={kind:'host_key_required',changed:true,key:{algorithm:'fixture',fingerprint:'Fixture fingerprint'}};
    else {owner.commandOpen=true;owner.command='pwd';}
    setup.functions.render(owner);
    const modal=setup.nodes.get('ssh-dialog-5');
    assert.deepEqual(plain(modal.props),{open:true,form:true,title:{transfer:'ssh_transfer',trust:'ssh_trust_title',command:'ssh_run'}[kind],width:560});
    assert.equal(modal.nodes.length,2);
    assert.equal('w' in modal.nodes[0].style,false);
    assert.equal('max_h' in modal.nodes[0].style,false,'native form owns scrolling and maximum height');
    assert.equal('pt_4' in modal.nodes[0].style,false);
    assert.ok(setup.nodes.has({transfer:'ssh-transfer-editor',trust:'ssh-host-key',command:'ssh-command'}[kind]));
    assert.ok(setup.buttons.has({transfer:'ssh-upload',trust:'ssh-trust',command:'ssh-run'}[kind]));
  }
});

test('SSH file editors use localized modal titles and separate form slots',async()=>{
  for(const [action,title] of [['rename','files_rename'],['new_file','files_new_file'],['new_directory','files_new_directory']]) {
    const setup=await fixture('ssh'),owner=view('ssh',profiles);
    owner.dialog=6;owner.fileEdit={action,input:`${action}-input`};
    setup.functions.render(owner);
    const modal=setup.nodes.get('ssh-dialog-6');
    assert.deepEqual(plain(modal.props),{open:true,form:true,title,width:408});
    assert.equal(modal.nodes.length,2);
    assert.equal(modal.nodes[0].selector,'ssh-file-editor');
    assert.equal(modal.nodes[0].nodes[0].selector,`${action}-input`);
    assert.equal('w' in modal.nodes[0].style,false,'native form owns the width');
    assert.ok(setup.buttons.has('ssh-file-cancel'));
    assert.ok(setup.buttons.has('ssh-file-submit'));
  }
});

test('SSH Transfer worktree selects preserve selection and pending guards',async()=>{
  for(const state of ['ready','pending','empty']) {
    const setup=await fixture('ssh'),owner=view('ssh',profiles);
    owner.dialog=6;owner.transfer={worktree:'tree',inputs:['source','destination']};owner.projects=[{id:'project',name:'Fixture'}];
    owner.worktrees=state==='empty'?[]:[{id:'tree',project:'project',path:'/fixture/tree'}];owner.locked=()=>state==='pending';
    setup.functions.render(owner);
    const menu=setup.nodes.get('ssh-worktree-6');assert.equal(menu.type,'SelectField');
    assert.deepEqual(plain(menu.props),{label:'ssh_worktree',placeholder:'ssh_no_project',selected:'tree',disabled:state!=='ready',
      items:state==='empty'?[]:[{id:'tree',label:'Fixture · /fixture/tree'}]});
  }
});

test('SSH profile removal preserves terminals but unmounts assistants and file navigation',async()=>{
  const setup=await fixture('ssh'),owner=view('ssh',profiles);
  owner.terminals=[{id:'retained',ssh:'removed',status:{kind:'running'}}];owner.terminal='retained';
  setup.functions.render(owner);
  assert.ok(setup.nodes.has('ssh-terminal-retained'));
  assert.equal(setup.nodes.get('ssh-workspace').props.has_details,false);
  assert.equal(setup.nodes.get('ssh-workspace').props.navigation,'none');
  assert.equal(setup.nodes.has('ssh-assistant-removed'),false);
});

test('SSH assistants use active profiles rather than selected saved connections',async()=>{
  const setup=await fixture('ssh'),owner=view('ssh',profiles);
  owner.selected='first';owner.terminals=[{id:'active',ssh:'second'}];owner.terminal='active';
  setup.functions.render(owner);
  const assistant=setup.nodes.get('ssh-assistant-second');
  assert.deepEqual(plain(assistant.props),{assistant:'ssh',resource:{kind:'ssh',id:'second'},heading:'ssh_assistant'});
  assert.equal(setup.nodes.get('ssh-workspace').props.has_details,true);
  assert.equal(setup.nodes.get('ssh-workspace').props.navigation,'resource');
});

test('SSH Files native context and tree fill navigation height using flex columns',async()=>{
  const setup=await fixture('ssh'),owner=view('ssh',profiles);
  owner.terminals=[{id:'active',ssh:'first'}];owner.terminal='active';owner.jobs=[];owner.fileMenu=()=>[];
  const edits=[];owner.editPath=cx=>edits.push(cx);
  owner.files=()=>({input:'path-input',path:'/',segments:()=>[{label:'/',path:'/'}],pending:false,profile:profiles[0],entries:[{name:'fixture.txt',kind:'file'}],selected:[],current:null,items:()=>[]});
  setup.functions.render(owner);
  const panel=setup.nodes.get('ssh-files-panel'),body=panel.nodes.find(node=>node.nodes?.[0]?.selector==='ssh-files-root');
  assert.equal(setup.nodes.has('ssh-files-header'),false);assert.equal(setup.nodes.has('ssh-files-parent'),false);
  const pathbar=setup.nodes.get('ssh-files-pathbar');
  assert.equal(pathbar.type,'PathBar');
  assert.deepEqual(plain(pathbar.props),{segments:['/'],disabled:false});
  assert.equal(setup.nodes.has('ssh-files-edit-path'),false);
  assert.equal(setup.nodes.get('ssh-files').props.guides,true);
  assert.ok(body);
  for(const method of ['v_flex','flex_1','min_h_0','min_w_0'])assert.deepEqual(plain(body.style[method]),[]);
  const tree=body.nodes[0].nodes[0];
  for(const method of ['flex_1','min_h_0','p_2'])assert.deepEqual(plain(tree.style[method]),[]);
  assert.equal(tree.nodes[0].selector,'ssh-files');
  assert.equal(panel.nodes[0].selector,'ssh-files-path');
  assert.equal(panel.nodes[0].type,'PanelHeader');
  assert.equal(setup.nodes.get('ssh-workspace').nodes[2],panel);
  setup.layout();setup.click('ssh-files-path-row');
  assert.equal(edits.length,1,'the path header opens its editor once');
  assert.equal(edits[0],setup.cx,'the path editor receives the event context');
});

test('SSH directory states fill their viewport without cards',async()=>{
  for(const state of ['empty','pending','failed','unloaded']) {
    const setup=await fixture('ssh'),owner=view('ssh',profiles);
    owner.terminals=[{id:'active',ssh:'first'}];owner.terminal='active';owner.jobs=[];
    owner.fileMenu=()=>[{id:'refresh',label:'Refresh',enabled:true}];
    const items=[{id:`placeholder-${state}`,label:state,disabled:true}];
    owner.files=()=>({input:'retained-path',path:'/',segments:()=>[{label:'/',path:'/'}],pending:state==='pending',error:state==='failed'?'ssh_files_failed':null,
      revision:state==='unloaded'?0:1,profile:profiles[0],entries:[],selected:[],current:null,items:()=>items});
    setup.functions.render(owner);
    const panel=setup.nodes.get('ssh-files-panel'),context=setup.nodes.get('ssh-files-root');
    const body=panel.nodes.find(node=>node.nodes?.[0]===context);
    assert.ok(body);
    assert.deepEqual(plain(context.props.items),[{id:'refresh',label:'Refresh',enabled:true}]);
    assert.equal(setup.nodes.get('ssh-workspace').nodes[2],panel);
    assert.equal(setup.nodes.has('ssh-files-empty'),false);
    const expected=state==='empty'?'files_directory_empty':state==='failed'?'ssh_files_failed':'ssh-files-loading';
    assert.equal(context.nodes[0].selector,expected);
    assert.equal(setup.nodes.has('ssh-files'),false,'empty and unavailable states replace the tree');
    if(state==='empty'||state==='failed')assert.deepEqual(plain(context.nodes[0].props),{
      variant:'list',fill_height:true,icon:'folder',label:expected,
    });
    else {
      for(const method of ['flex_1','h_flex','items_center','justify_center'])assert.deepEqual(plain(context.nodes[0].style[method]),[]);
      assert.equal(context.nodes[0].nodes[0].type,'Spinner');
    }
    assert.equal(setup.nodes.has('ssh-files-retry'),state==='failed');
    assert.equal(setup.nodes.has('ssh-files-busy'),state==='pending');
    assert.equal(panel.nodes[0].selector,'ssh-files-path');
    assert.equal(setup.nodes.has('ssh-files-header'),false);assert.equal(setup.nodes.has('ssh-files-parent'),false);
  }
});

test('database catalogs and assistants use native slots and result tables',async()=>{
  const setup=await fixture('databases'),owner=view('databases',profiles);owner.opened='first';
  setup.functions.render(owner);
  assert.equal(setup.nodes.get('database-workspace').props.navigation,'resource');
  assert.equal(setup.nodes.get('database-workspace').nodes[2].selector,'db-catalog');
  assert.equal(setup.nodes.has('database-catalog-header'),false);
  assert.equal(setup.nodes.get('db-catalog').nodes.length,1,'catalog contains only its tree viewport');
  // The resource tree draws each database and table with its icon.
  assert.equal(setup.nodes.get('database-tree').props.guides,true);
  assert.equal(setup.nodes.get('database-tree').props.variant,undefined);
  assert.equal(setup.nodes.get('database-tree').props.current,null);
  assert.deepEqual(plain(setup.nodes.get('database-assistant-first').props),{assistant:'database',resource:{kind:'database',id:'first'},heading:'connection_assistant'});
  assert.equal(setup.nodes.has('db_data_empty'),false);
  assert.deepEqual(plain(setup.nodes.get('database-results').props),{revision:'0',columns:[],rows:[],page:0,menu:[],stripe:true,header:false,bordered:false,empty:'db_data_empty',empty_icon:'table'});
});

test('database catalog renders the retained UI selection through the native tree',async()=>{
  const setup=await fixture('databases'),owner=view('databases',profiles);owner.opened='first';owner.catalog.current='table:0:1';
  setup.functions.render(owner);
  assert.equal(setup.nodes.get('database-tree').props.current,'table:0:1');
});

test('an open database uses one native closeable header tab, disabled while pending',async()=>{
  for(const pending of [null,{request:{unknown:false}},{request:{unknown:true}}]) {
    const setup=await fixture('databases'),owner=view('databases',profiles);owner.opened='first';owner.pending=pending;
    setup.functions.render(owner);
    const header=JSON.parse(setup.nodes.get('database-header').props.content);
    assert.equal(header.title,'database');
    assert.deepEqual(header.tabs,{id:'db-tabs',selected:'first',max_width:180,close_label:'close',
      items:[{id:'first',label:'First',icon:'hard-drive',closable:true,close_disabled:!!pending}]});
  }
});

test('database result pagination uses Kit page numbers',async()=>{
  const setup=await fixture('databases'),owner=view('databases',profiles);
  owner.result={columns:['value'],rows:Array.from({length:123},(_,value)=>[{kind:'text',value}]),truncated:false};
  owner.page=1;setup.functions.results(owner);
  const pages=setup.nodes.get('db-pages');assert.equal(pages.type,'Pagination');
  assert.deepEqual(plain(pages.style.current_page),[2]);assert.deepEqual(plain(pages.style.total_pages),[3]);assert.deepEqual(plain(pages.style.visible_pages),[3]);
  pages.on_change(3,setup.cx);assert.equal(owner.page,2);
  assert.equal(setup.nodes.get('database-results').props.stripe,true);
  assert.equal(setup.nodes.get('database-results').props.header,true);
});

test('database zero-row results preserve columns and a localized empty state',async()=>{
  const setup=await fixture('databases'),owner=view('databases',profiles);
  owner.result={columns:['id','value'],rows:[],truncated:false};
  setup.functions.results(owner);
  const table=setup.nodes.get('database-results');
  assert.deepEqual(plain(table.props.columns),['id','value']);
  assert.deepEqual(plain(table.props.rows),[]);
  assert.equal(table.props.header,true);
  assert.equal(table.props.empty,'db_data_empty');
  assert.equal(table.props.empty_icon,'table');
  assert.equal(setup.nodes.has('db_data_empty'),false);
});

test('database history bar opens SQL in a dialog instead of tabs',async()=>{
  const setup=await fixture('databases'),owner=view('databases',profiles);owner.opened='first';
  setup.functions.historyPanel(owner);
  assert.equal(setup.nodes.has('db-bottom-tabs'),false,'history and SQL are not tabs');
  assert.deepEqual(plain(setup.nodes.get('db-new-query').props),{icon:'plus',label:'db_new_query',show_label:true,size:'small',disabled:false});
  assert.equal(setup.nodes.has('db-stop'),false);
  for(const unknown of [false,true]) {
    const running=await fixture('databases'),state=view('databases',profiles);state.opened='first';
    state.pending={request:{unknown,command:{kind:'query_database'}}};state.locked=()=>true;
    running.functions.historyPanel(state);
    // A query run from a closed dialog keeps its Stop or Check result in the bar.
    assert.equal(running.nodes.get(unknown?'db-inspect':'db-stop').props.size,'small');
    assert.equal(running.nodes.get('db-new-query').props.disabled,true);
  }
  for(const sqlOpen of [false,true]) {
    const page=await fixture('databases'),state=view('databases',profiles);state.opened='first';state.sqlOpen=sqlOpen;
    page.functions.render(state);
    const dialog=page.nodes.get('db-dialog-0');
    assert.equal(dialog.props.open,sqlOpen);
    if(sqlOpen) {
      assert.deepEqual(plain({title:dialog.props.title,width:dialog.props.width,form:dialog.props.form}),{title:'db_new_query',width:720,form:true});
      assert.equal(dialog.nodes[0].selector,'db-sql');
    } else assert.equal(dialog.nodes.length,0);
  }
});

test('database results end in a footer with the row count',async()=>{
  const setup=await fixture('databases'),owner=view('databases',profiles);
  owner.result={columns:['value'],rows:[[{kind:'text',value:'a'}]],truncated:true};
  setup.functions.results(owner);
  assert.equal(setup.nodes.get('db-results-count').nodes[0],'db_rows:{"count":1}');
  assert.equal(setup.nodes.get('db-results-truncated').nodes[0],'db_truncated');
  assert.equal(setup.nodes.has('db-pages'),false);
  const empty=await fixture('databases'),none=view('databases',profiles);
  empty.functions.results(none);
  assert.equal(empty.nodes.has('db-results-footer'),false);
});

test('SQL dialog provides a code editor, Cancel and one query action',async()=>{
  for(const state of ['ready','running','unknown']) {
    const setup=await fixture('databases'),owner=view('databases',profiles);owner.opened='first';owner.sql='SELECT 1';owner.catalog.database='main';
    if(state!=='ready')owner.pending={request:{unknown:state==='unknown'}};
    setup.functions.sqlEditor(owner);setup.functions.sqlFooter(owner);
    assert.equal(setup.nodes.get('db-sql').nodes[0].type,'TextField');
    assert.equal(setup.nodes.has('db-execution-status'),false,'no idle status text');
    assert.equal(setup.nodes.get('db-execution-target').nodes[1].nodes[0],'main');
    const id=state==='ready'?'db-run':state==='unknown'?'db-inspect':'db-stop';
    assert.deepEqual(plain(setup.nodes.get(id).props),{icon:state==='ready'?'play':state==='unknown'?'rotate-cw':'square',
      label:state==='ready'?'db_run':state==='unknown'?'db_inspect':'db_stop',disabled:false,show_label:true,size:'medium',
      ...(state==='ready'?{variant:'primary',shortcut:'secondary-enter'}:{})});
    assert.deepEqual([...setup.buttons.keys()],['db-sql-cancel']);
  }
});

test('query history lists newest first with Copy and Open row actions',async()=>{
  const setup=await fixture('databases'),owner=view('databases',profiles);owner.opened='first';
  owner.logs=Array.from({length:100},(_,index)=>({id:`entry-${index}`,sql:`SELECT '${index} 中文 🙂'`,database:`db-${index}`,timestamp:index,elapsed:0.25,result:{rows:{columns:['value'],rows:[],affected_rows:0}}}));
  owner.logs[99].result={error:{message:'Fixture database failure'}};owner.historyPage=1;
  setup.functions.log(owner);setup.layout();assert.equal(setup.buttons.size,0,'history actions belong in the table menu');
  const table=setup.nodes.get('database-history-table');
  assert.equal(table.type,'DataTable');
  const props=plain(table.props);
  assert.deepEqual(props.columns,['db_sql_tab','db_log_result','db_log_duration']);
  assert.equal(props.rows.length,100);assert.deepEqual(props.rows[0],[`SELECT '99 中文 🙂'`,'db_failed','0.25s']);
  assert.deepEqual(props.rows[1],[`SELECT '98 中文 🙂'`,'db_rows:{"count":0}','0.25s']);
  assert.equal(props.cells[0][1].primary.text_tone,'warning');assert.equal(props.cells[1][1].primary.text_tone,undefined);
  assert.equal(props.details[0][0],`${new Date(99).toLocaleTimeString()}\nSELECT '99 中文 🙂'\nFixture database failure`);
  assert.deepEqual(props.row_ids.slice(0,2),['entry-99','entry-98']);
  assert.equal(props.revision,'first:100:entry-99');assert.equal(props.page,1);
  assert.deepEqual({stripe:props.stripe,header:props.header,bordered:props.bordered,alignments:props.alignments},{stripe:true,header:true,bordered:false,alignments:['start','start','end']});
  assert.deepEqual(props.menu,[{id:'copy',label:'db_copy_query',enabled:true},{id:'open',label:'db_open_sql',enabled:true}]);
  owner.locked=()=>true;setup.functions.log(owner);
  assert.deepEqual(plain(setup.nodes.get('database-history-table').props.menu.map(item=>item.enabled)),[true,false]);
  for(const removed of ['database-history-heading','database-history-rows','db-history-entry-99'])assert.equal(setup.nodes.has(removed),false);
  const panel=await fixture('databases');panel.functions.historyPanel(owner);
  const pages=panel.nodes.get('db-history-pages');assert.deepEqual(plain(pages.style.total_pages),[2]);assert.deepEqual(plain(pages.style.current_page),[2]);
  pages.on_change(1,panel.cx);assert.equal(owner.historyPage,0);
});

test('empty database history is an empty table state',async()=>{
  const setup=await fixture('databases'),owner=view('databases',profiles);owner.historyPage=0;
  setup.functions.log(owner);
  const props=plain(setup.nodes.get('database-history-table').props);
  assert.deepEqual(props.rows,[]);assert.equal(props.empty,'db_history_empty');assert.equal(props.empty_icon,'clock');
  const panel=await fixture('databases');panel.functions.historyPanel(owner);assert.equal(panel.nodes.has('db-history-pages'),false);
});

test('query history fills the lower pane and SQL fills its dialog',async()=>{
  const setup=await fixture('databases'),owner=view('databases',profiles);owner.opened='first';owner.sqlOpen=true;
  setup.functions.render(owner);
  const split=setup.nodes.get('db-editor-results');
  assert.deepEqual(plain(split.style.axis),['vertical']);assert.equal(split.nodes.length,2);
  const lower=split.nodes[1];assert.deepEqual(plain(lower.style.size),[200]);assert.deepEqual(plain(lower.style.size_range),[120,480]);
  const panel=lower.nodes[0];
  assert.equal(panel,setup.nodes.get('database-history-panel'));assert.deepEqual(plain(panel.style.size_full),[]);
  const editor=setup.nodes.get('db-sql');
  assert.deepEqual(plain(editor.style.w_full),[]);assert.equal(editor.nodes[0].type,'TextField');
  assert.equal(setup.nodes.get('db-dialog-0').nodes[1].selector,'db-sql-footer');
  assert.equal('w' in lower.style,false);assert.equal('w' in panel.style,false);
});

test('database results and history labels are localized in English and Chinese',async()=>{
  const {messages}=await import('../dev.sailry.platform/desktop/locales.js');
  for(const locale of ['en','zh-CN']) {
    const text=messages(locale);
    for(const key of ['db_data_empty','db_log_result','db_log_duration','db_history_empty','db_new_query']) {
      assert.equal(typeof text[key],'string');assert.ok(text[key]);assert.equal(/[.!?。！？]$/.test(text[key]),false);
    }
  }
  assert.notEqual(messages('en').db_log_result,messages('zh-CN').db_log_result);
});

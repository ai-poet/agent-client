import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import {lanes,labels,sections,clock,plain as readable,statusColors,statusColor,dotColor} from '../dev.sailry.platform/desktop/state.js';
import {messages} from '../dev.sailry.platform/desktop/locales.js';

const source=(await readFile(new URL('../dev.sailry.platform/desktop/view.js',import.meta.url),'utf8'))
  .replace(/^import .*;\n/gm,'').replace(/^export /gm,'');
const plain=value=>JSON.parse(JSON.stringify(value));

function fixture(width=1440) {
  const nodes=new Map();
  function element(kind='div',id=null,props={}) {
    const value={kind,selector:id,props,items:[],contentSlot:null,style:{}};
    const proxy=new Proxy(value,{get(target,key) {
      if(key==='id')return id=>{target.selector=id;nodes.set(id,proxy);return proxy;};
      if(key==='child')return child=>{target.items.push(child);return proxy;};
      if(key==='content')return child=>{target.contentSlot=child;return proxy;};
      if(key==='children')return children=>{target.items.push(...children);return proxy;};
      if(key in target)return target[key];
      return (...args)=>{target.style[key]=args;return proxy;};
    }});
    if(id)nodes.set(id,proxy);
    return proxy;
  }
  const adapters=Object.fromEntries(['EmptyState','Appearance','CardButton','ClampedText','ThinkingIcon','ScrollRegion','Collapse','CollapseSlot','Loading']
    .map(kind=>[kind,{new:(id,props)=>element(kind,id,props)}]));
  const {render,BOARD_MAX_WIDTH}=vm.runInNewContext(`${source}\n({render,BOARD_MAX_WIDTH})`,{
    div:()=>element(),Icon:function(id){return element('Icon',id);},
    ...adapters,lanes,labels,sections,clock,plain:readable,statusColors,statusColor,dotColor,pageInsets:()=>({rail_width:56}),
    theme:()=>({colors:{muted_foreground:'muted',border:'border',primary:'primary',warning:'warning',chart_3:'reply',
      success:'success',destructive:'failure'}}),window:{rem_size:()=>16,viewport_size:()=>({width})},
  });
  const view={text:messages('en'),board:{connected:true,error:false,previews:new Map(),
    catalog:{node:'captured',projects:[{id:'project',name:'Project'}],worktrees:[{id:'tree',project:'project',path:'/fixture'}],
      sessions:[],terminals:[],unread_terminals:[],session_lanes:{},terminal_lanes:{}}}};
  return {view,nodes,BOARD_MAX_WIDTH,render(){nodes.clear();return render(view,{theme:()=>({radius:{lg:11}})});}};
}

test('empty lanes fill their viewport without cards',()=>{
  const setup=fixture();setup.render();
  for(const lane of lanes) {
    const key=labels[lane],items=setup.nodes.get(`${key}-items`),empty=setup.nodes.get(`${key}-empty`);
    assert.deepEqual(plain(empty.props),{variant:'list',fill_height:true,vertical_align:'start',icon:'inbox',label:setup.view.text.activity_board_empty});
    assert.equal(items.items[0],empty);
    assert.deepEqual(plain(items.style.overflow_y_scrollbar),[]);
    const heading=setup.nodes.get(`${key}-heading`);
    assert.deepEqual(plain(heading.style.rounded),[11]);
    assert.deepEqual(plain(heading.style.border_1),[]);
    assert.deepEqual(plain(heading.style.border_color),['border']);
    assert.equal(heading.style.rounded_full,undefined);
  }
  assert.equal([...setup.nodes.values()].filter(node=>node.kind==='EmptyState').length,4);
});

test('initial reads preserve lane headings without claiming empty',()=>{
  const setup=fixture(),catalog=setup.view.board.catalog;
  setup.view.board.catalog=null;
  for(const error of [false,true]) {
    setup.view.board.error=error;setup.render();
    for(const lane of lanes) {
      const key=labels[lane];
      assert.ok(setup.nodes.has(`${key}-heading`));
      assert.equal(setup.nodes.has(`${key}-empty`),false);
      const placeholder=setup.nodes.get(`${key}-loading`);
      assert.equal(placeholder.kind,'Loading');
    }
    assert.equal([...setup.nodes.values()].filter(node=>node.kind==='EmptyState').length,0);
  }
  setup.view.board.catalog=catalog;setup.render();
  assert.equal([...setup.nodes.values()].filter(node=>node.kind==='Loading').length,0);
  assert.equal([...setup.nodes.values()].filter(node=>node.kind==='EmptyState').length,4);
});

test('refresh failures retain the loaded lane cards',()=>{
  const setup=fixture(),catalog=setup.view.board.catalog;
  catalog.sessions=[{id:'retained',project:'project',worktree:'tree',archived:false,delegation:null,
    activity:{title:'Retained task',queued:0,waiting:null,run:null}}];
  catalog.session_lanes.retained='idle';setup.render();
  const card=setup.nodes.get('activity-retained');
  setup.view.board.connected=false;setup.view.board.error=true;setup.render();
  assert.ok(setup.nodes.has('activity-retained'));
  assert.equal(setup.nodes.get('activity-retained').kind,card.kind);
  assert.equal(setup.nodes.has(`${labels.idle}-empty`),false);
  assert.equal([...setup.nodes.values()].filter(node=>node.kind==='Loading').length,0);
});

test('populated lanes retain their cards and host disclosures without empty content',()=>{
  const setup=fixture(),catalog=setup.view.board.catalog;
  catalog.sessions=[{id:'retained',project:'project',worktree:'tree',archived:false,delegation:null,
    activity:{title:'Retained task',queued:0,waiting:null,run:{turn:'turn',status:'running'}}}];
  catalog.session_lanes.retained='running';setup.render();
  const key=labels.running,group=`activity-project-${key}-project`;
  assert.equal(setup.nodes.has(`${key}-empty`),false);
  assert.ok(setup.nodes.has('activity-retained'));
  assert.deepEqual(plain(setup.view.targets.get('activity-retained')),{kind:'session',id:'retained'});
  assert.equal(setup.view.targets.has(group),false);
  assert.equal(setup.nodes.get(`captured-${group}`).kind,'Collapse');
  assert.equal([...setup.nodes.values()].filter(node=>node.kind==='EmptyState').length,3);
});

test('read failures keep all lanes without a retry bar',()=>{
  const setup=fixture();setup.view.board.error=true;setup.render();
  assert.equal(setup.nodes.has('activity-retry'),false);
  for(const lane of lanes)assert.ok(setup.nodes.has(`${labels[lane]}-items`));
});

function containing(root,child) {
  if(!root?.items)return;
  if(root.items.includes(child))return root;
  for(const nested of [...root.items,...(root.contentSlot ? [root.contentSlot] : [])]) {
    const parent=containing(nested,child);
    if(parent)return parent;
  }
}

test('project headings and content use host slots with a natural scrolling ceiling',()=>{
  const setup=fixture(),catalog=setup.view.board.catalog;
  catalog.sessions=Array.from({length:7},(_,index)=>({id:`retained-${index}`,project:'project',worktree:'tree',
    archived:false,delegation:null,activity:{title:'Retained task',queued:0,waiting:null,run:null}}));
  catalog.session_lanes=Object.fromEntries(catalog.sessions.map(session=>[session.id,'idle']));
  {
    const root=setup.render(),selector=`activity-project-${labels.idle}-project`;
    const header=setup.nodes.get(selector),group=containing(root,header),reveal=group.items[1];
    assert.equal(group.items[0],header);
    assert.equal(group.kind,'Collapse');
    assert.deepEqual(plain(group.props),{label:'Project',default_open:true,variant:'plain'});
    assert.equal(header.kind,'CollapseSlot');assert.equal(header.props.variant,'header');
    assert.equal(reveal.kind,'CollapseSlot');assert.equal(reveal.props.variant,'content');
    const viewport=setup.nodes.get(`${selector}-viewport`),area=viewport.items[0];
    assert.equal(containing(reveal,viewport).kind,'div');
    assert.equal(area.kind,'ScrollRegion');
    assert.deepEqual(plain(area.props),{max_height:520});
    assert.equal(area.selector,'captured-activity-project-activity_idle-project-cards');
    assert.equal(area.items.length,7);
    const card=setup.nodes.get('activity-retained-0');
    assert.equal(card.kind,'CardButton');
    assert.equal(card.props.fill_height,undefined);
    assert.equal(setup.nodes.has('activity-retained-0-content'),false);
    assert.deepEqual(plain(setup.nodes.get('activity-retained-0-title').style.truncate),[]);
    assert.ok(setup.nodes.has('activity-retained-0-location'));
    assert.equal(setup.view.targets.has(selector),false);
    assert.deepEqual(plain(setup.view.targets.get('activity-retained-0')),{kind:'session',id:'retained-0'});
  }
});

test('native project scroll identity includes the captured node and lane',()=>{
  const setup=fixture(),catalog=setup.view.board.catalog;
  catalog.sessions=[{id:'retained',project:'project',worktree:'tree',archived:false,delegation:null,
    activity:{title:'Retained task',queued:0,waiting:null,run:null}}];
  catalog.session_lanes.retained='idle';setup.render();
  const key='captured-activity-project-activity_idle-project-cards';
  setup.render();assert.equal(setup.nodes.get(key).kind,'ScrollRegion');
  const sessions=catalog.sessions;
  catalog.sessions=[];setup.render();
  assert.equal(setup.nodes.has(key),false);
  catalog.sessions=sessions;setup.render();
  assert.equal(setup.nodes.get(key).kind,'ScrollRegion');
  catalog.session_lanes.retained='running';setup.render();
  assert.equal(setup.nodes.has(key),false);
  assert.equal(setup.nodes.get('captured-activity-project-activity_board_running-project-cards').kind,'ScrollRegion');
  catalog.session_lanes.retained='idle';
  catalog.node='replacement';setup.render();
  assert.equal(setup.nodes.has(key),false);
  assert.equal(setup.nodes.get('replacement-activity-project-activity_idle-project-cards').kind,'ScrollRegion');
  catalog.node='captured';setup.render();assert.equal(setup.nodes.get(key).kind,'ScrollRegion');
});

test('cards show project, title, plain preview, status and timing',()=>{
  const setup=fixture(),catalog=setup.view.board.catalog;
  const started=new Date(2026,9,3,20,30).getTime();
  catalog.sessions=[{id:'retained',project:'project',worktree:'tree',archived:false,delegation:null,
    activity:{title:'Retained task',queued:0,waiting:null,run:{turn:'turn',status:'running',started_ms:started}}}];
  catalog.session_lanes.retained='running';
  setup.view.board.previews.set('retained',{session:'retained',turn:'turn',text:'## Summary\n\n- **Done**: `cargo test`'});
  setup.render();
  const card=setup.nodes.get('activity-retained'),body=card.items[0];
  assert.deepEqual(body.items.map(item=>item.selector),
    ['activity-retained-header','activity-retained-title','activity-retained-content','activity-retained-footer']);
  const location=setup.nodes.get('activity-retained-location');
  assert.equal(setup.nodes.get('activity-retained-project').items[0],'Project');
  assert.ok(containing(location,setup.nodes.get('activity-retained-project-icon')));
  const title=setup.nodes.get('activity-retained-title');
  assert.deepEqual(plain(title.style.truncate),[]);
  assert.deepEqual(plain(title.style.text_sm),[]);assert.equal(title.style.text_base,undefined);
  assert.equal(setup.nodes.get('activity-retained-content').props.text,'Summary Done: cargo test');
  const state=setup.nodes.get('activity-retained-state');
  assert.equal(state.items[0].kind,'ThinkingIcon');
  assert.equal(state.items[0].props.phase,'turn_tools_running');
  const status=state.items[1],dot=setup.nodes.get('activity-retained-dot');
  assert.equal(status,setup.nodes.get('activity-retained-status'));
  assert.equal(status.items[0],dot);assert.equal(status.items[1].items[0],setup.view.text.activity_running);
  assert.deepEqual(plain(status.style.text_color),['primary']);assert.deepEqual(plain(dot.style.bg),['primary']);
  assert.deepEqual(plain(status.style.text_xs),[]);
  assert.deepEqual(plain(setup.nodes.get('activity-retained-time').style.text_xs),[]);
  assert.equal(status.style.border_1,undefined);assert.equal(status.style.rounded_full,undefined);
  assert.equal([...setup.nodes.values()].filter(node=>node.kind==='Tag').length,0);
  assert.equal(setup.nodes.get('activity-retained-time').items[1],clock(started));
  assert.equal(setup.nodes.has('activity-retained-duration'),false);
  catalog.sessions[0].activity.run={turn:'turn',status:'completed',started_ms:started,finished_ms:started+94000};
  catalog.session_lanes.retained='completed';setup.render();
  assert.equal(setup.nodes.has('activity-retained-loading'),false);
  assert.deepEqual(plain(setup.nodes.get('activity-retained-status').style.text_color),['success']);
  assert.equal(setup.nodes.get('activity-retained-duration').items[1],'1m34s');
  catalog.sessions[0].activity.run=null;setup.render();
  assert.equal(setup.nodes.has('activity-retained-time'),false);
  assert.equal(setup.nodes.has('activity-retained-content'),false);
});

test('status dots share the plaza colors and neutral idle tone',()=>{
  const setup=fixture(),catalog=setup.view.board.catalog;
  const cases=[['running',null,'running','primary'],['waiting','approval','running','warning'],['waiting','input','running','reply'],
    ['completed',null,'completed','success'],['failed',null,'failed','failure'],['idle',null,'cancelled','muted']];
  for(const [lane,waiting,status,color] of cases) {
    catalog.sessions=[{id:'retained',project:'project',worktree:'tree',archived:false,delegation:null,
      activity:{title:'Retained task',queued:0,waiting,run:{turn:'turn',status}}}];
    catalog.session_lanes.retained=lane;setup.render();
    assert.deepEqual(plain(setup.nodes.get('activity-retained-dot').style.bg),[color]);
    assert.deepEqual(plain(setup.nodes.get('activity-retained-status').style.text_color),[color]);
  }
});

test('read results keep their outcome dot beside a neutral label',()=>{
  const setup=fixture(),catalog=setup.view.board.catalog;
  const cases=[['failed',0,'failure'],['completed',0,'success'],['interrupted',0,'muted'],['cancelled',0,'muted'],['completed',2,'muted']];
  for(const [status,queued,color] of cases) {
    catalog.sessions=[{id:'retained',project:'project',worktree:'tree',archived:false,delegation:null,
      activity:{title:'Retained task',queued,waiting:null,run:{turn:'turn',status}}}];
    catalog.session_lanes.retained='idle';setup.render();
    assert.deepEqual(plain(setup.nodes.get('activity-retained-dot').style.bg),[color]);
    assert.deepEqual(plain(setup.nodes.get('activity-retained-status').style.text_color),['muted']);
  }
  catalog.sessions=[];
  catalog.terminals=[{id:'shell',worktree:'tree',title:'Shell'}];
  for(const [lane,color] of [['failed','failure'],['completed','success']]) {
    catalog.terminal_lanes.shell=lane;setup.render();
    assert.deepEqual(plain(setup.nodes.get('activity-terminal-shell-dot').style.bg),[color]);
    assert.deepEqual(plain(setup.nodes.get('activity-terminal-shell-status').style.text_color),['muted']);
  }
});

test('wide windows center the board at its maximum width',()=>{
  for(const [width,expected] of [[1200,(1200-56-32-36)/4],[1440,(1280-36)/4],[2400,(1280-36)/4]]) {
    const setup=fixture(width);setup.render();
    const board=setup.nodes.get('activity-board');
    assert.deepEqual(plain(board.style.max_w),[setup.BOARD_MAX_WIDTH]);assert.deepEqual(plain(board.style.mx_auto),[]);
    for(const lane of lanes)assert.ok(Math.abs(setup.nodes.get(`${labels[lane]}-column`).style.w[0]-expected)<1e-9);
  }
});

test('cards follow the shared activity order within a project group',()=>{
  const setup=fixture(),catalog=setup.view.board.catalog;
  catalog.sessions=['a','b','c'].map(id=>({id,project:'project',worktree:'tree',archived:false,delegation:null,
    activity:{title:id,queued:0,waiting:null,run:null}}));
  catalog.session_lanes={a:'idle',b:'idle',c:'idle'};catalog.activity_order=['session:b','session:c','session:a'];
  setup.render();
  const area=setup.nodes.get('captured-activity-project-activity_idle-project-cards');
  assert.deepEqual(area.items.map(card=>card.selector),['activity-b','activity-c','activity-a']);
});

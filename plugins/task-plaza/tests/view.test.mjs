import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';

const root=new URL('../dev.sailry.platform/desktop/',import.meta.url);
const sources=await Promise.all(['locales','state','floor','geometry','art','scene','ground','details','view'].map(async name =>
  (await readFile(new URL(`${name}.js`,root),'utf8')).replace(/^import .*;\n/gm,'').replace(/^export /gm,'')));
const plain=value=>JSON.parse(JSON.stringify(value));

function fixture(count=0) {
  const nodes=new Map(),opened=[],viewport={width:1440,height:1000};
  function element(kind='div',id=null,props={}) {
    const value={kind,selector:id,props,items:[],style:{}};
    const proxy=new Proxy(value,{get(target,key) {
      if(key==='id')return id=>{target.selector=id;nodes.set(id,proxy);return proxy;};
      if(key==='child')return child=>{target.items.push(child);return proxy;};
      if(key==='children')return children=>{target.items.push(...children);return proxy;};
      if(key in target)return target[key];
      return (...args)=>{target.style[key]=args;return proxy;};
    }});
    if(id)nodes.set(id,proxy);return proxy;
  }
  const native=Object.fromEntries(['Header','Workspace','Image','Conversation','ComposedButton','CardButton','PanelHeader','IconButton','EmptyState','Loading','ClampedText','StatusRing'].map(kind=>[kind,{new:(id,props)=>{
    const result=element(kind,id,props);
    if(kind==='ClampedText')result.child(props.text);
    return result;
  }}]));
  const api=vm.runInNewContext(`${sources.join('\n')}\n({render,ground,layout,Floor,messages,WORLD_WIDTH,WORLD_HEIGHT,ROOMS,PERSON_SIZE,BOARD,PAGER,EDGE,edgeColor,boardPadding,boardClearance,pagerWidth})`,{
    div:()=>element(),Button:function(id){return element('Button',id);},Icon:function(id){return element('Icon',id);},
    Resizable:function(id){return element('Resizable',id);},ResizablePanel:function(){return element('ResizablePanel');},...native,pageInsets:()=>({rail_width:56}),
    theme:()=>({colors:{background:'background',surface:'surface',foreground:'foreground',border:'border',muted_foreground:'#808080ff',primary:'primary',warning:'warning',chart_3:'reply',success:'success',destructive:'failure',group_box:'group'}}),
    PathBuilder:{stroke:width=>{
      const path={width,commands:[],build(){return this;}};
      for(const method of ['move_to','line_to','curve_to','dash_array'])path[method]=(...args)=>{path.commands.push([method,...args]);return path;};
      return path;
    }},Background:{solid:color=>element('Background',null,{color})},
    window:{viewport_size:()=>viewport,rem_size:()=>16,paint_path:(path,fill)=>element('NativePath',null,{path,fill})},openActivityTarget:(...args)=>opened.push(args),
  });
  const sessions=Array.from({length:count},(_,index)=>({id:`session-${index}`,project:'project',worktree:'tree',archived:false,delegation:null,
    activity:{title:`Task ${index}`,queued:0,waiting:null,run:{turn:`turn-${index}`,status:'running'}}}));
  const view={text:api.messages('en'),floor:new api.Floor(),panelOpen:false,selected:null,
    board:{connected:true,previews:new Map(),catalog:{node:'captured',projects:[{id:'project',name:'Project'}],
      worktrees:[{id:'tree',project:'project',path:'/fixture'}],sessions,terminals:[],unread_terminals:[],
      session_lanes:Object.fromEntries(sessions.map(session=>[session.id,'running'])),terminal_lanes:{}}},
    inspect(item){this.selected={kind:'session',id:item.id,title:item.title};this.panelOpen=true;},
    inspectRoom(id){this.selected={kind:'room',id};this.panelOpen=true;},
    inspectApprovals(){this.selected={kind:'approvals'};this.panelOpen=true;},closeDetails(){this.panelOpen=false;},
  };
  return {view,nodes,opened,viewport,...api,render(){nodes.clear();return api.render(view);},
    click(id){if(['ComposedButton','CardButton'].includes(nodes.get(id).kind))view.inspect(view.targets.get(id));
      else if(nodes.get(id).kind==='IconButton') {
        if(id==='plaza-open-conversation')opened.push(['session',view.selected.id]);
        view.closeDetails();
      }else nodes.get(id).style.on_click[0]({}, {notify(){}});}};
}

test('eight rooms surround a plaza with six desks each',()=>{
  const setup=fixture(4);setup.render();
  assert.equal([...setup.nodes.keys()].filter(id=>id.startsWith('plaza-room-outline-')).length,8);
  assert.equal([...setup.nodes.keys()].filter(id=>/^plaza-desk-/.test(id)).length,48);
  const plaza=setup.nodes.get('plaza-atrium');
  assert.equal(plaza.style.bg,undefined);assert.equal(plaza.style.border_1,undefined);
  assert.ok(Math.abs(setup.nodes.get('plaza-world').style.w[0]/setup.nodes.get('plaza-world').style.h[0]-setup.WORLD_WIDTH/setup.WORLD_HEIGHT)<1e-12);
  assert.ok([...setup.nodes.values()].filter(node=>node.kind==='Image').every(node=>node.props.fit==='fill'));
  for(let index=0;index<4;index++)assert.equal(setup.nodes.get(`plaza-session-${index}`).kind,'ComposedButton');
  assert.equal([...setup.nodes.values()].filter(node=>node.kind==='Conversation').length,0);
  assert.equal(setup.nodes.get('plaza-room-project').kind,'Button');
  assert.ok(setup.nodes.get('plaza-room-project-board').style.left[0]>0);
  assert.ok(setup.nodes.has('plaza-viewport'));
});

test('watermark dashes stay aligned across repeated rows',()=>{
  const setup=fixture(),result=setup.ground(1603,947),path=result.items[1].props.path;
  assert.ok(path.commands.length<200,'watermark cost must not grow with the number of tiles');
  assert.deepEqual(plain(path.commands[0]),['dash_array',[4,28]]);
  for(let index=1;index<path.commands.length;index+=2) {
    const [,x,y]=path.commands[index], [,right,bottom]=path.commands[index+1];
    assert.equal(((right-x)+(bottom-y))%32,0,'each row must end on a full dash period');
  }
});

test('shared sidebar releases conversations on close or replacement',()=>{
  const setup=fixture(2);setup.render();
  const full=setup.nodes.get('plaza-world').style.w[0];
  setup.click('plaza-session-0');setup.render();
  assert.deepEqual(plain(setup.nodes.get('plaza-detail-chat-session-0').props),{active:true,session:'session-0'});
  assert.ok(setup.nodes.has('plaza-detail-panel'));
  assert.equal(setup.nodes.get('plaza-detail-header').kind,'PanelHeader');
  assert.equal(setup.nodes.get('plaza-workspace').kind,'Workspace');
  assert.equal(setup.nodes.get('plaza-workspace').props.has_details,true);
  assert.equal(setup.nodes.get('plaza-workspace').items[1],setup.nodes.get('plaza-detail-panel'));
  assert.equal(setup.nodes.has('plaza-columns'),false);
  assert.ok(setup.nodes.get('plaza-world').style.w[0]<full);
  assert.equal([...setup.nodes.values()].filter(node=>node.kind==='Modal').length,0);
  setup.click('plaza-session-1');setup.render();
  assert.equal(setup.nodes.get('plaza-detail-chat-session-0').props.active,false);
  assert.deepEqual(plain(setup.nodes.get('plaza-detail-chat-session-1').props),{active:true,session:'session-1'});
  setup.click('plaza-detail-close');setup.render();
  assert.equal(setup.nodes.get('plaza-detail-chat-session-1').props.active,false);
  assert.equal(setup.nodes.has('plaza-detail-panel'),false);
  assert.equal(setup.nodes.get('plaza-workspace').props.has_details,false);
  setup.click('plaza-session-0');setup.render();setup.click('plaza-open-conversation');setup.render();
  assert.deepEqual(setup.opened,[['session','session-0']]);
  assert.equal(setup.nodes.get('plaza-detail-chat-session-0').props.active,false);
});

test('approval switches directly to the plaza and returns to its desk',()=>{
  const setup=fixture(1),session=setup.view.board.catalog.sessions[0];
  setup.render();const seated=plain(setup.nodes.get('plaza-session-0-position').style);
  session.activity.waiting='approval';setup.view.board.catalog.session_lanes[session.id]='waiting';setup.render();
  const waiting=setup.nodes.get('plaza-session-0-position');
  assert.ok(waiting.style.left[0] > seated.left[0]);
  setup.click('plaza-session-0');setup.render();
  assert.equal(setup.nodes.get('plaza-detail-chat-session-0').props.session,session.id);
  session.activity.waiting=null;setup.view.board.catalog.session_lanes[session.id]='running';setup.render();
  setup.click('plaza-detail-close');setup.render();
  assert.deepEqual(plain(setup.nodes.get('plaza-session-0-position').style),seated);
});

test('overflow retains access to every session',()=>{
  const setup=fixture(8);setup.render();
  assert.equal([...setup.nodes.keys()].filter(id=>/^plaza-session-\d+$/.test(id)).length,6);
  setup.click('plaza-room-next-project');setup.render();
  assert.equal([...setup.nodes.keys()].filter(id=>/^plaza-session-\d+$/.test(id)).length,2);
  setup.click('plaza-room-project');setup.render();
  assert.equal([...setup.nodes.values()].filter(node=>node.kind==='CardButton').length,8);
});

test('room lists show current previews without replacing the scene',()=>{
  const setup=fixture(1);setup.view.board.previews.set('session-0',{turn:'turn-0',text:'## Done\n**Checked**'});
  setup.render();setup.click('plaza-room-project');const result=setup.render();
  assert.ok(setup.nodes.has('plaza-world'));
  function content(node){return typeof node==='string'?node:(node?.items??[]).map(content).join(' ');}
  assert.match(content(result),/Done Checked/);
});

test('only active work lights the computer screen',()=>{
  const setup=fixture(1),session=setup.view.board.catalog.sessions[0];
  setup.render();
  const width=setup.nodes.get('plaza-desk-project-0').style.w[0];
  const monitor=()=>setup.nodes.get('plaza-monitor-project-0-clip').items[0].style.left[0];
  const unlit=-53*(width/216),lit=-366*(width/216);
  assert.ok(Math.abs(monitor()-lit)<1e-8);
  session.activity.run.status='queued';setup.render();assert.ok(Math.abs(monitor()-unlit)<1e-8);
  session.activity.run.status='running';session.activity.waiting='input';
  setup.view.board.catalog.session_lanes[session.id]='waiting';setup.render();assert.ok(Math.abs(monitor()-unlit)<1e-8);
  session.activity.waiting='approval';setup.render();assert.ok(Math.abs(monitor()-unlit)<1e-8);
});

test('each active seat has one ring in its legend color',()=>{
  const setup=fixture(1),session=setup.view.board.catalog.sessions[0];
  for(const [lane,waiting,state] of [['running',null,'running'],['waiting','approval','approval'],['waiting','input','input'],
    ['completed',null,'completed'],['failed',null,'failed']]) {
    setup.view.board.catalog.session_lanes[session.id]=lane;session.activity.waiting=waiting;
    setup.render();
    const ring=setup.nodes.get('plaza-session-0-status'),legend=setup.nodes.get(`plaza-legend-${state}`);
    assert.equal(ring.kind,'StatusRing');
    assert.equal(ring.props.color,legend.style.bg[0]);
    assert.deepEqual(plain(ring.props),{color:legend.style.bg[0],opacity:1,line:1.5,running:lane==='running'});
    setup.click('plaza-session-0');setup.render();
    const rings=[...setup.nodes.values()].filter(node=>node.kind==='StatusRing'&&node.selector.startsWith('plaza-session-0'));
    assert.equal(rings.length,1,'selection must not add a second ring');
    setup.view.panelOpen=false;
  }
});

test('idle seats and read terminals have no ring',()=>{
  const setup=fixture(1),catalog=setup.view.board.catalog;
  catalog.session_lanes['session-0']='idle';catalog.sessions[0].activity.run=null;
  catalog.terminals=[{id:'shell',worktree:'tree',title:'Shell'}];catalog.terminal_lanes={shell:'completed'};
  setup.render();
  assert.ok(setup.nodes.has('plaza-session-0-position'));assert.ok(setup.nodes.has('plaza-terminal-shell-position'));
  assert.equal([...setup.nodes.values()].filter(node=>node.kind==='StatusRing').length,0);
  catalog.unread_terminals=['shell'];setup.render();
  assert.equal(setup.nodes.get('plaza-terminal-shell-status').props.color,'success');
});

test('room task dots follow the run status in their label',()=>{
  const setup=fixture(1),catalog=setup.view.board.catalog,session=catalog.sessions[0];
  for(const [lane,waiting,status,color] of [['running',null,'running','primary'],['waiting','approval','running','warning'],
    ['waiting','input','running','reply'],['completed',null,'completed','success'],['idle',null,'failed','failure'],
    ['idle',null,'completed','success'],['idle',null,'cancelled','#808080ff']]) {
    catalog.session_lanes[session.id]=lane;session.activity.waiting=waiting;session.activity.run.status=status;
    setup.view.inspectRoom('project');setup.render();
    const dot=setup.nodes.get('plaza-detail-session-session-0-state').items[0];
    assert.deepEqual(plain(dot.style.bg),[color]);
  }
});

test('seat tooltips show only the title',()=>{
  const setup=fixture(1);setup.render();
  const seat=setup.nodes.get('plaza-session-0');
  assert.equal(seat.props.tooltip,'Task 0');
  assert.equal([...setup.nodes.values()].filter(node=>node.kind==='Tag').length,0);
});

test('seats follow the shared activity order before paging',()=>{
  const setup=fixture(8),catalog=setup.view.board.catalog;
  catalog.activity_order=['session-6','session-1','session-4','session-3','session-0','session-2','session-7','session-5'].map(id=>`session:${id}`);
  setup.render();
  const seated=()=>[...setup.nodes.keys()].filter(id=>/^plaza-session-\d+-position$/.test(id))
    .map(id=>setup.nodes.get(id).style).map(style=>[style.top[0],style.left[0]]);
  const order=[...setup.nodes.keys()].filter(id=>/^plaza-session-\d+-position$/.test(id))
    .sort((left,right)=>{
      const [a,b]=[setup.nodes.get(left).style,setup.nodes.get(right).style];
      return a.top[0]-b.top[0]||a.left[0]-b.left[0];
    }).map(id=>id.replace(/^plaza-|-position$/g,''));
  assert.deepEqual(order,['session-6','session-1','session-4','session-3','session-0','session-2']);
  assert.equal(seated().length,6);
  setup.click('plaza-room-next-project');setup.render();
  assert.ok(setup.nodes.has('plaza-session-7-position')&&setup.nodes.has('plaza-session-5-position'));
});

test('whiteboards stack each total above its label',()=>{
  const setup=fixture(12),catalog=setup.view.board.catalog;
  for(const [index,session] of catalog.sessions.entries()) {
    catalog.session_lanes[session.id]=['running','waiting','completed','idle','waiting','failed'][index%6];
    session.activity.waiting=index%6===1?'approval':index%6===4?'input':null;
  }
  const tones={running:'primary',approval:'warning',completed:'success',idle:'foreground'};
  for(const locale of ['en','zh-CN']) {
    setup.view.text=setup.messages(locale);setup.render();
    const name=setup.nodes.get('plaza-room-project-name'),grid=setup.nodes.get('plaza-room-project-statistics');
    assert.deepEqual(plain(name.items),['Project']);
    assert.ok(name.style.truncate&&name.style.font_semibold&&name.style.text_center);
    assert.deepEqual(grid.items,Object.keys(tones).map(state=>setup.nodes.get(`plaza-room-project-${state}`)));
    for(const state of Object.keys(tones)) {
      const cell=setup.nodes.get(`plaza-room-project-${state}`),[count,label]=cell.items;
      assert.ok(cell.style.flex_1&&cell.style.items_center);
      assert.equal(count,setup.nodes.get(`plaza-room-project-${state}-count`));
      assert.equal(count.items[0],'2');assert.equal(count.style.text_color[0],tones[state]);
      assert.ok(count.style.font_semibold);
      assert.equal(label.style.text_color[0],'#808080ff');assert.ok(label.style.text_xs&&label.style.truncate);
    }
  }
});

test('empty totals use the muted foreground',()=>{
  const setup=fixture();setup.render();
  for(const state of ['running','approval','completed','idle']) {
    const count=setup.nodes.get(`plaza-room-project-${state}-count`);
    assert.equal(count.items[0],'0');assert.equal(count.style.text_color[0],'#808080ff');
  }
});

test('whiteboards stay opaque and padded behind the hover tint',()=>{
  const setup=fixture(1);setup.render();
  const backing=setup.nodes.get('plaza-room-project-board'),sign=setup.nodes.get('plaza-room-project'),label=sign.items[0];
  assert.equal(backing.items[0],sign);
  assert.equal(backing.style.bg[0],'background');assert.equal(sign.style.bg[0],'background');
  const edge=setup.nodes.get('plaza-room-outline-0').items[0].props.fill.props.color;
  assert.equal(edge,'#8080808c');assert.equal(sign.style.border_color[0],edge,'the border matches the room edge');
  assert.deepEqual(plain(sign.style.rounded),[setup.BOARD.radius]);assert.deepEqual(plain(backing.style.rounded),[setup.BOARD.radius]);
  assert.deepEqual([sign.style.w[0],sign.style.h[0]],[backing.style.w[0],backing.style.h[0]]);
  for(const node of [backing,sign])assert.equal(node.style.opacity,undefined);
  assert.equal(sign.style.hover,undefined);
  const scale=setup.nodes.get('plaza-world').style.w[0]/setup.WORLD_WIDTH;
  assert.deepEqual(plain(label.style.px),[setup.BOARD.padding_x]);assert.deepEqual(plain(label.style.gap),[setup.BOARD.row_gap]);
  assert.ok(Math.abs(label.style.py[0]-setup.boardPadding(scale))<1e-9);
});

test('whiteboards cover the opening in the top edge',()=>{
  const setup=fixture(1);setup.render();
  const scale=setup.nodes.get('plaza-world').style.w[0]/setup.WORLD_WIDTH,{bounds}=setup.ROOMS[0];
  const board=setup.nodes.get('plaza-room-project-board').style;
  const commands=setup.nodes.get('plaza-room-outline-0').items[0].props.path.commands;
  const left=board.left[0]-bounds.x*scale,top=board.top[0]-bounds.y*scale;
  assert.ok(top<0&&top+board.h[0]>0,'the whiteboard straddles the top edge');
  const [start,end]=[commands[0],commands.at(-1)];
  assert.deepEqual(plain([start[0],start[2],end[0],end[2]]),['move_to',0,'line_to',0]);
  assert.ok(Math.abs(start[1]-left-board.w[0])<1e-9&&Math.abs(end[1]-left)<1e-9);
  const desk=setup.nodes.get('plaza-desk-project-0').style;
  assert.ok(desk.top[0]-board.top[0]-board.h[0]>=setup.boardClearance(scale)-1e-9);
});

test('pagers sit in an opening of the bottom edge',()=>{
  const sizes=[[{width:900,height:1000},true,0.65],[{width:1600,height:1078},false,1]];
  for(const [size,sidebar,expected] of sizes) {
    const setup=fixture(8);
    Object.assign(setup.viewport,size);setup.view.panelOpen=sidebar;setup.view.selected={kind:'room',id:'project'};setup.render();
    const scale=setup.nodes.get('plaza-world').style.w[0]/setup.WORLD_WIDTH;
    assert.ok(Math.abs(scale-expected)<1e-9);
    const label=setup.nodes.get('plaza-room-project').items[0].style.py[0];
    assert.ok(Math.abs(label-setup.BOARD.padding_y*Math.min(1,expected)**2)<1e-9);
    const pager=setup.nodes.get('plaza-room-pages-project').style,outline=setup.nodes.get('plaza-room-outline-0').style;
    const bottom=outline.top[0]+outline.h[0],next=setup.nodes.get('plaza-room-3-board').style;
    assert.ok(Math.abs(pager.left[0]+pager.w[0]/2-(outline.left[0]+outline.w[0]/2))<1e-9);
    assert.ok(Math.abs(pager.top[0]+pager.h[0]/2-bottom)<1e-9,'the pager is centered on the bottom edge');
    assert.ok(pager.top[0]+pager.h[0]<=next.top[0]);
    // The bottom edge stops short of the pager on both sides.
    const commands=setup.nodes.get('plaza-room-outline-0').items[0].props.path.commands;
    const opening=commands.findIndex(([method],index)=>index>0&&method==='move_to');
    const [,gapRight,gapY]=commands[opening-1],[,gapLeft,resumeY]=commands[opening];
    assert.ok(Math.abs(gapY-outline.h[0])<1e-9&&Math.abs(resumeY-outline.h[0])<1e-9);
    const middle=outline.w[0]/2;
    assert.ok(Math.abs(gapRight-middle-(pager.w[0]/2+setup.PAGER.notch))<1e-9);
    assert.ok(Math.abs(middle-gapLeft-(pager.w[0]/2+setup.PAGER.notch))<1e-9);
    const people=[...setup.nodes.values()].filter(node=>/^plaza-session-\d+-position$/.test(node.selector));
    assert.ok(people.length>0);
    for(const {style:person} of people) {
      assert.ok(person.top[0]+person.h[0]<=pager.top[0],'seats clear the pager');
    }
    setup.click('plaza-room-next-project');setup.render();
    assert.deepEqual(plain(setup.nodes.get('plaza-room-pages-project').style.w),plain(pager.w));
  }
  const single=fixture(4);single.render();
  assert.equal(single.nodes.has('plaza-room-pages-project'),false);
  for(const [setup,opened] of [[fixture(8),1],[single,0]]) {
    setup.render();
    for(let index=0;index<8;index++) {
      const commands=setup.nodes.get(`plaza-room-outline-${index}`).items[0].props.path.commands;
      assert.equal(commands.filter(([method])=>method==='move_to').length,index===0?1+opened:1);
    }
  }
});

test('room edges and neutral rings share one token',()=>{
  const setup=fixture(1);setup.render();
  assert.deepEqual(plain(setup.EDGE),{color:'muted_foreground',opacity:0.55});
});

test('room padding clears the whiteboard and the bottom edge',()=>{
  for(const width of [900,1440]) {
    const setup=fixture(1);setup.viewport.width=width;setup.render();
    const scale=setup.nodes.get('plaza-world').style.w[0]/setup.WORLD_WIDTH;
    const board=setup.nodes.get('plaza-room-project-board').style,outline=setup.nodes.get('plaza-room-outline-0').style;
    const person=setup.nodes.get('plaza-session-0-position').style;
    assert.ok(person.top[0]-(board.top[0]+board.h[0])>=12*scale,'seats keep room below the whiteboard');
    for(let seat=0;seat<6;seat++) {
      const desk=setup.nodes.get(`plaza-desk-project-${seat}`).style;
      assert.ok(desk.top[0]-(board.top[0]+board.h[0])>=setup.boardClearance(scale)-1e-9);
    }
    const lowest=setup.ROOMS[0].bounds.y+140+68;
    assert.ok(outline.top[0]+outline.h[0]-lowest*scale>=30*scale,'the last row keeps room above the bottom edge');
  }
});

test('taller whiteboards leave desks and page controls clear',()=>{
  const setup=fixture(8);
  const catalog=setup.view.board.catalog,original=[...catalog.sessions];
  for(let index=1;index<8;index++) {
    const project=`p${index}`,tree=`tree-${index}`;
    catalog.projects.push({id:project,name:project});catalog.worktrees.push({id:tree,project,path:'/fixture'});
    for(const session of original) {
      const id=`${project}-${session.id}`;
      catalog.sessions.push({...session,id,project,worktree:tree});catalog.session_lanes[id]='running';
    }
  }
  for(const width of [900,1440])for(const sidebar of [false,true]) {
    setup.viewport.width=width;setup.view.panelOpen=sidebar;setup.view.selected={kind:'room',id:'project'};setup.render();
    const board=setup.nodes.get('plaza-room-project-board').style,desk=setup.nodes.get('plaza-desk-project-0').style;
    assert.ok(board.top[0]+board.h[0]<=desk.top[0]);
    const pager=setup.nodes.get('plaza-room-pages-project').style,next=setup.nodes.get('plaza-room-p3-board').style;
    assert.ok(pager.top[0]+20<=next.top[0]);
    const last=setup.nodes.get('plaza-room-pages-p7').style,world=setup.nodes.get('plaza-world').style;
    assert.ok(setup.nodes.get('plaza-scene-frame').style.pb_8);
    assert.ok(last.top[0]+20<=world.h[0]+32);
  }
});

test('all furniture shares a uniform scale inside disjoint room boundaries',()=>{
  const setup=fixture(4);setup.render();
  const world=setup.nodes.get('plaza-world'),scale=world.style.w[0]/setup.WORLD_WIDTH;
  for(let index=0;index<8;index++) {
    const bounds=setup.ROOMS[index].bounds;
    for(let seat=0;seat<6;seat++) {
      const key=index===0?'project':index;
      const desk=setup.nodes.get(`plaza-desk-${key}-${seat}`).style;
      assert.ok(desk.left[0]>=bounds.x*scale);
      assert.ok(desk.top[0]>=bounds.y*scale);
      assert.ok(desk.left[0]+desk.w[0]<=(bounds.x+bounds.width)*scale);
      assert.ok(desk.top[0]+desk.h[0]<=(bounds.y+bounds.height)*scale);
      assert.ok(Math.abs(desk.w[0]/desk.h[0]-216/185)<1e-8);
    }
    for(let other=index+1;other<8;other++) {
      const second=setup.ROOMS[other].bounds;
      assert.ok(bounds.x+bounds.width<=second.x||second.x+second.width<=bounds.x||
        bounds.y+bounds.height<=second.y||second.y+second.height<=bounds.y);
    }
  }
  assert.ok(setup.nodes.has('plaza-ground'));
  assert.ok([...setup.nodes.values()].filter(node=>node.kind==='Image').every(node=>!node.props.path.includes('environment')));
});

test('approval groups fit without touching adjacent rooms',()=>{
  for(const count of [1,3,6]) {
    const setup=fixture(count);
    for(const session of setup.view.board.catalog.sessions) {
      session.activity.waiting='approval';setup.view.board.catalog.session_lanes[session.id]='waiting';
    }
    setup.render();
    const plaza=setup.nodes.get('plaza-atrium').style;
    const people=Array.from({length:count},(_,index)=>setup.nodes.get(`plaza-session-${index}-position`).style);
    for(const person of people) {
      assert.ok(person.left[0]>=plaza.left[0]);assert.ok(person.top[0]>=plaza.top[0]);
      assert.ok(person.left[0]+person.w[0]<=plaza.left[0]+plaza.w[0]);
      assert.ok(person.top[0]+person.h[0]<=plaza.top[0]+plaza.h[0]);
    }
    assert.ok(setup.nodes.get('plaza-room-6-board').style.top[0]>plaza.top[0]+plaza.h[0]);
    for(let index=0;index<people.length;index++)for(let other=index+1;other<people.length;other++) {
      const first=people[index],second=people[other];
      assert.ok(first.left[0]+first.w[0]<=second.left[0]||second.left[0]+second.w[0]<=first.left[0]||
        first.top[0]+first.h[0]<=second.top[0]||second.top[0]+second.h[0]<=first.top[0]);
    }
  }
});

test('pending catalogs keep room selection unavailable',()=>{
  const setup=fixture();setup.view.board.catalog=null;setup.render();
  assert.equal(setup.nodes.has('plaza-loading'),false);
  assert.ok(setup.nodes.has('plaza-header'));assert.ok(setup.nodes.has('plaza-toolbar'));
  assert.equal(setup.nodes.has('plaza-world'),false);
  setup.view.board.error=true;setup.render();
  assert.equal(setup.nodes.get('plaza-retry').kind,'Button');
  assert.equal(setup.nodes.has('plaza-loading'),false);
});

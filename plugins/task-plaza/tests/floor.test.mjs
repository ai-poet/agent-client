import assert from 'node:assert/strict';
import test from 'node:test';
import {Floor,layout,SEAT_COUNT,ROOM_COUNT} from '../dev.sailry.platform/desktop/floor.js';
import {messages} from '../dev.sailry.platform/desktop/locales.js';
const text=messages('en');
function catalog(count=4) {
  const sessions=Array.from({length:count},(_,index)=>({id:`s${index}`,project:'project',worktree:'tree',archived:false,delegation:null,
    activity:{title:`Task ${index}`,queued:0,waiting:null,run:{turn:`t${index}`,status:'running'}}}));
  return {node:'node',projects:[{id:'project',name:'Project'}],worktrees:[{id:'tree',project:'project',path:'/project'}],
    sessions,session_lanes:Object.fromEntries(sessions.map(session=>[session.id,'running'])),terminals:[],terminal_lanes:{},unread_terminals:[]};
}
const render=(data,floor)=>layout(data,new Map(),text,floor);

test('seats follow the shared activity order',()=>{
  const data=catalog(),floor=new Floor();
  data.activity_order=['s2','s0','s3','s1'].map(id=>`session:${id}`);
  const seats=()=>Object.fromEntries(render(data,floor).rooms[0].items.map(item=>[item.id,item.seat]));
  assert.deepEqual(seats(),{s2:0,s0:1,s3:2,s1:3});
  data.sessions.reverse();
  assert.deepEqual(seats(),{s2:0,s0:1,s3:2,s1:3});
  data.activity_order.reverse();
  assert.deepEqual(seats(),{s1:0,s3:1,s0:2,s2:3});
});

test('ordering applies across lanes before pagination',()=>{
  const data=catalog(8),floor=new Floor();
  data.session_lanes.s7='idle';data.session_lanes.s0='completed';
  data.sessions[3].activity.waiting='approval';data.session_lanes.s3='waiting';
  data.activity_order=['s3','s0','s1','s2','s4','s5','s6','s7'].map(id=>`session:${id}`);
  let room=render(data,floor).rooms[0];
  assert.deepEqual(room.visible.map(item=>item.id),['s3','s0','s1','s2','s4','s5']);
  floor.turnRoom(room.key,1,room.pages);room=render(data,floor).rooms[0];
  assert.deepEqual(room.visible.map(item=>item.id),['s6','s7']);
});

test('running overflow fills consecutive pages without holes',()=>{
  const data=catalog(14),floor=new Floor();
  for(const session of data.sessions)data.session_lanes[session.id]='idle';
  render(data,floor);
  for(let index=0;index<8;index++)data.session_lanes[`s${index}`]='running';
  let room=render(data,floor).rooms[0];
  assert.ok(room.visible.every(item=>item.lane==='running'));
  floor.turnRoom(room.key,1,room.pages);room=render(data,floor).rooms[0];
  assert.deepEqual(room.visible.map(item=>item.lane),['running','running','idle','idle','idle','idle']);
  floor.turnRoom(room.key,1,room.pages);room=render(data,floor).rooms[0];
  assert.equal(room.visible.length,2);
  data.sessions=data.sessions.slice(0,2);room=render(data,floor).rooms[0];
  assert.equal(room.page,0);assert.equal(room.pages,1);
});

test('more than six sessions and eight projects remain reachable',()=>{
  const data=catalog(14),floor=new Floor();
  for(let index=0;index<9;index++)data.projects.push({id:`p${index}`,name:`Project ${index}`});
  let model=render(data,floor);assert.equal(model.visible.length,ROOM_COUNT);assert.equal(model.pages,2);
  const room=model.rooms[0];assert.equal(room.visible.length,SEAT_COUNT);assert.equal(room.pages,3);
  const ids=new Set(room.visible.map(item=>item.id));
  for(let page=1;page<3;page++) {
    floor.turnRoom(room.key,1,room.pages);model=render(data,floor);
    for(const item of model.rooms[0].visible)ids.add(item.id);
  }
  assert.equal(ids.size,14);floor.page=1;model=render(data,floor);assert.equal(model.visible.length,2);
  data.sessions[0].activity.waiting='approval';data.session_lanes.s0='waiting';
  assert.equal(render(data,floor).approvals[0].id,'s0');
});

test('removed sessions close gaps and another node clears placement',()=>{
  const data=catalog(),floor=new Floor();
  data.sessions=data.sessions.filter(session=>session.id!=='s0');
  assert.deepEqual(render(data,floor).rooms[0].items.map(item=>item.seat),[0,1,2]);
  floor.page=5;floor.roomPages.set('project',3);data.node='replacement';
  render(data,floor);assert.equal(floor.page,0);assert.equal(floor.roomPages.get('project'),0);
});

test('stale approval flags and hidden sessions stay outside the plaza',()=>{
  const data=catalog();data.sessions[0].activity.waiting='approval';data.session_lanes.s0='completed';
  data.sessions[1].activity.waiting='approval';data.session_lanes.s1='waiting';data.sessions[1].archived=true;
  data.sessions[2].activity.waiting='approval';data.session_lanes.s2='waiting';data.sessions[2].delegation={session:'parent'};
  const model=render(data,new Floor());assert.equal(model.approvals.length,0);assert.equal(model.rooms[0].items.length,2);
});

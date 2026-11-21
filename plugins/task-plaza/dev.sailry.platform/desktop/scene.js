import {div,PathBuilder,Background} from 'gpui-kit';
import {Button,Icon} from 'gpui-component';
import {theme} from 'sailry';
import {ComposedButton,StatusRing} from 'sailry/ui';
import {sprite} from './art.js';
import {statusColors,statusColor} from './state.js';
import {ROOMS,WORLD_WIDTH,WORLD_HEIGHT,PERSON_SIZE,DESK_WIDTH,DESK_HEIGHT,SEATED,STANDING,ATRIUM,seatPosition,approvalPosition} from './geometry.js';

// Room chrome stays at theme size; the scene geometry follows the scale.
export const BOARD = {padding_x:12,padding_y:10,row_gap:8,clearance:14,hang:0.65,radius:4,line:1.2};
// The pager sits in an opening of the bottom edge with `notch` clearance on each side.
export const PAGER = {button:22,height:20,gap:4,notch:6};
// Room edges and neutral rings share one theme token.
export const EDGE = {color:'muted_foreground',opacity:0.55};
// Theme colors are `#rrggbbaa`; borders take one color, so the edge opacity folds into its alpha.
export function edgeColor(colors) {
  const color = colors[EDGE.color], alpha = color.length === 9 ? parseInt(color.slice(7),16) : 255;
  return color.slice(0,7) + Math.round(alpha * EDGE.opacity).toString(16).padStart(2,'0');
}
const CORNER = 4;
const deskTop = index => seatPosition(index,0).y - 14 - DESK_HEIGHT - ROOMS[index].bounds.y;
const shrink = scale => Math.min(1,scale);

// Small scenes trim the board's vertical padding and desk clearance, so each
// pager below its room still clears the next row's whiteboard.
export function boardPadding(scale) {
  return BOARD.padding_y * shrink(scale) ** 2;
}
export function boardClearance(scale) {
  return BOARD.clearance * shrink(scale);
}

export function boardWidth(index,scale) {
  return Math.max(ROOMS[index].board.width * scale,220);
}
// Name, count and label lines at their theme sizes, plus padding and the 1px border.
export function boardHeight(rem,scale) {
  return rem * BOARD.line * (0.875 + 0.875 + 0.75) + BOARD.row_gap + boardPadding(scale) * 2 + 2;
}
// The board straddles the room's top edge and keeps a gap above the desks.
export function boardTop(index,scale,rem) {
  const height = boardHeight(rem,scale);
  return ROOMS[index].bounds.y * scale + Math.min(deskTop(index) * scale - boardClearance(scale),height * BOARD.hang) - height;
}
// The widest label reserves a stable width, so paging never shifts the controls.
export function pagerWidth(value,rem) {
  if (!value || value.pages < 2) return 0;
  return PAGER.button * 2 + PAGER.gap * 2 + Math.max(2,`${value.pages}/${value.pages}`.length * 0.5) * rem;
}
function board(view,value,index,scale) {
  const {bounds} = ROOMS[index], {text} = view, colors = theme().colors, rem = window.rem_size();
  const counts = {running:0,approval:0,completed:0,idle:0};
  for (const item of value?.items ?? []) {
    const state = item.state === 'idle' ? 'idle' : item.lane;
    if (item.approval) counts.approval += 1;
    else if (state in counts) counts[state] += 1;
  }
  const id = `plaza-room-${value?.key ?? index}`, width = boardWidth(index,scale), height = boardHeight(rem,scale);
  const tone = state => !counts[state] ? colors.muted_foreground : state === 'idle' ? colors.foreground : colors[statusColors[state]];
  const entries = [['running',text.room_running],['approval',text.room_approvals],['completed',text.room_completed],['idle',text.room_idle]];
  const label = div().v_flex().size_full().min_w_0().justify_center().gap(BOARD.row_gap).px(BOARD.padding_x).py(boardPadding(scale))
    .text_color(colors.foreground).line_height(BOARD.line)
    .child(div().id(`${id}-name`).w_full().min_w_0().truncate().text_center().text_sm().font_semibold().child(value?.name ?? text.empty_room))
    .children(value ? [div().id(`${id}-statistics`).h_flex().w_full().min_w_0()
      .children(entries.map(([state,title]) => div().id(`${id}-${state}`).v_flex().flex_1().min_w_0().items_center()
        .child(div().id(`${id}-${state}-count`).text_sm().font_semibold().text_color(tone(state)).child(String(counts[state])))
        .child(div().max_w_full().truncate().text_xs().text_color(colors.muted_foreground).child(title))))] : []);
  // Kit's outline hover tints with a translucent fill. The opaque backing keeps
  // the sign solid over the room edge and the ground in every state.
  const sign = new Button(id).outline().secondary().p_0().w(width).h(height).rounded(BOARD.radius)
    .border_color(edgeColor(colors)).bg(colors.background).disabled(!value).tooltip(value?.name ?? text.empty_room).child(label);
  if (value) sign.on_click((_event,cx) => view.inspectRoom(value.key,cx));
  return div().id(`${id}-board`).absolute().left((bounds.x + bounds.width / 2) * scale - width / 2).top(boardTop(index,scale,rem))
    .w(width).h(height).rounded(BOARD.radius).bg(colors.background).child(sign);
}
function outline(value,index,scale) {
  const {bounds} = ROOMS[index], width = bounds.width * scale, height = bounds.height * scale;
  const middle = width / 2, cover = boardWidth(index,scale) / 2, pager = pagerWidth(value,window.rem_size());
  const left = Math.max(middle - cover,CORNER), right = Math.min(middle + cover,width - CORNER), path = PathBuilder.stroke(1);
  // The whiteboard covers the top edge between its borders; a paged room also
  // opens its bottom edge around the pager, like a legend on a border.
  const notch = pager ? Math.min(pager / 2 + PAGER.notch,middle - CORNER) : 0;
  path.move_to(right,0);
  if (right < width - CORNER) path.line_to(width - CORNER,0);
  path.curve_to(width,CORNER,width,0).line_to(width,height - CORNER).curve_to(width - CORNER,height,width,height);
  if (notch) path.line_to(middle + notch,height).move_to(middle - notch,height);
  path.line_to(CORNER,height).curve_to(0,height - CORNER,0,height).line_to(0,CORNER).curve_to(CORNER,0,0,0);
  if (left > CORNER) path.line_to(left,0);
  return div().id(`plaza-room-outline-${index}`).absolute()
    .left(bounds.x * scale).top(bounds.y * scale).w(width).h(height)
    .child(window.paint_path(path.build(),Background.solid(edgeColor(theme().colors))).w(width).h(height));
}
function paging(view,value,index,scale) {
  const width = pagerWidth(value,window.rem_size());
  if (!width) return null;
  const {bounds} = ROOMS[index], colors = theme().colors, label = width - PAGER.button * 2 - PAGER.gap * 2;
  // The pager is centered on the room's bottom edge, inside its opening.
  return div().id(`plaza-room-pages-${value.key}`).absolute()
    .left((bounds.x + bounds.width / 2) * scale - width / 2).top((bounds.y + bounds.height) * scale - PAGER.height / 2)
    .w(width).h(PAGER.height).h_flex().items_center().gap(PAGER.gap).text_xs().text_color(colors.muted_foreground)
    .child(new Button(`plaza-room-prev-${value.key}`).ghost().size('small').p_0().w(PAGER.button).h(PAGER.height)
      .disabled(value.page === 0).tooltip(view.text.previous).child(new Icon('icons/chevron-left.svg').size('xsmall'))
      .on_click((_event,cx) => {view.floor.turnRoom(value.key,-1,value.pages);cx.notify();}))
    .child(div().id(`plaza-room-page-${value.key}`).w(label).text_center().child(`${value.page + 1}/${value.pages}`))
    .child(new Button(`plaza-room-next-${value.key}`).ghost().size('small').p_0().w(PAGER.button).h(PAGER.height)
      .disabled(value.page + 1 === value.pages).tooltip(view.text.next).child(new Icon('icons/chevron-right.svg').size('xsmall'))
      .on_click((_event,cx) => {view.floor.turnRoom(value.key,1,value.pages);cx.notify();}));
}
// One ring per seat in its state color; idle seats have none and selection adds none.
export function ring(item) {
  if (item.lane === 'idle' || item.state === 'idle') return null;
  const token = statusColor(item);
  return {color:token === statusColors.idle ? EDGE.color : token,
    opacity:token === statusColors.idle ? EDGE.opacity : 1,running:item.lane === 'running'};
}
function person(view,item,position,scale,approval = false) {
  const id = item.kind === 'terminal' ? `plaza-terminal-${item.id}` : `plaza-${item.id}`;
  const dimension = approval ? 80 : PERSON_SIZE, size = dimension * scale, index = approval ? STANDING[item.character] : SEATED[item.character];
  const status = ring(item);
  view.targets.set(id,item);
  return div().id(`${id}-position`).absolute().left((position.x - dimension / 2) * scale)
    .top((position.y - dimension) * scale).w(size).h(size)
    .child(ComposedButton.new(id,{size,label:`${item.title}, ${item.status}, ${item.project}`,tooltip:item.title})
      .child(div().relative().w(size).h(size).h_flex().justify_center()
        .children(status ? [div().id(`${id}-ring`).absolute().bottom(0).left(size * 0.07).w(size * 0.86).h(14 * scale)
          .child(StatusRing.new(`${id}-status`,{color:theme().colors[status.color],opacity:status.opacity,
            line:1.5,running:status.running}))] : [])
        .child(sprite(id,index,size))));
}
function plaza(view,model,scale) {
  const colors = theme().colors, width = ATRIUM.width * scale, height = ATRIUM.height * scale;
  return div().id('plaza-atrium').absolute().left(ATRIUM.x * scale).top(ATRIUM.y * scale).w(width).h(height)
    .children([0,22].map(inset => div().absolute().inset(inset * scale).rounded_full().border_1().border_color(colors.border)))
    .child(div().absolute().bottom(6 * scale).w_full().text_center().text_sm().text_color(colors.muted_foreground)
      .child(model.approvals.length ? view.text.approval_count(model.approvals.length) : view.text.approval_plaza));
}
export function scene(view,model,scale) {
  const world = div().id('plaza-world').relative().flex_shrink(0).w(WORLD_WIDTH * scale).h(WORLD_HEIGHT * scale)
    .children(ROOMS.map((_room,index) => outline(model.visible[index],index,scale))).child(plaza(view,model,scale));
  const furnishings = [], people = [];
  for (let room = 0;room < 8;room += 1) {
    const value = model.visible[room];
    for (let seat = 0;seat < 6;seat += 1) {
      const position = seatPosition(room,seat), item = value?.visible.find(item => item.seat % 6 === seat);
      const lit = item && !item.approval && item.phase === 'turn_tools_running';
      const identity = `${value?.key ?? room}-${seat}`;
      furnishings.push({depth:position.y,element:div().id(`plaza-desk-${identity}`).absolute()
        .left((position.x - DESK_WIDTH / 2) * scale).top((position.y - 14 - DESK_HEIGHT) * scale)
        .w(DESK_WIDTH * scale).h(DESK_HEIGHT * scale)
        .child(sprite(`plaza-monitor-${identity}`,lit ? 1 : 0,DESK_HEIGHT * scale,DESK_WIDTH * scale))});
      if (item && !item.approval) people.push({item,position});
      else furnishings.push({depth:position.y + 0.1,element:div().id(`plaza-chair-${identity}`).absolute()
        .left((position.x - 17) * scale).top((position.y - 36) * scale).w(34 * scale).h(36 * scale)
        .child(sprite(`plaza-empty-chair-${identity}`,2,36 * scale))});
    }
  }
  model.approvals.slice(0,6).forEach((item,index) => people.push({item,position:approvalPosition(index,model.approvals.length),approval:true}));
  return world.children([...furnishings,...people.map(value => ({depth:value.position.y + 0.1,
      element:person(view,value.item,value.position,scale,value.approval)}))]
      .sort((left,right) => left.depth - right.depth).map(value => value.element))
    .children(ROOMS.map((_room,index) => board(view,model.visible[index],index,scale)))
    .children(ROOMS.map((_room,index) => paging(view,model.visible[index],index,scale)).filter(Boolean));
}

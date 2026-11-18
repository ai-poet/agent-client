import {div} from 'gpui-kit';
import {Button,Icon} from 'gpui-component';
import {Header,theme} from 'sailry';
import {Workspace,pageInsets} from 'sailry/ui';
import {layout} from './floor.js';
import {WORLD_WIDTH,WORLD_HEIGHT} from './geometry.js';
import {scene} from './scene.js';
import {ground} from './ground.js';
import {details,DETAIL_WIDTH} from './details.js';
import {statusColors} from './state.js';

export function render(view) {
  view.targets = new Map();
  const {text,board,floor} = view, colors = theme().colors;
  const model = layout(board.catalog,board.previews,text,floor), viewport = window.viewport_size();
  const side = details(view,model), width = viewport.width - pageInsets().rail_width;
  // Keep the unscaled theme labels readable when the sidebar narrows the scene.
  const scale = Math.max(0.65,Math.min((width - 96 - (side.panel ? DETAIL_WIDTH : 0)) / WORLD_WIDTH,
    (viewport.height - 178) / WORLD_HEIGHT));
  const header = Header.new('plaza-header',{content:JSON.stringify({
    items:[{label:text.activity_board_running,value:String(model.counts.running)},
      {label:text.activity_board_waiting,value:String(model.approvals.length)},
      {label:text.activity_idle,value:String(model.counts.idle)}],actions:[],
  })});
  const changeFloor = direction => (_event,cx) => {
    floor.page = Math.max(0,Math.min(model.pages - 1,floor.page + direction));cx.notify();
  };
  const legend = [['running',text.activity_board_running],['idle',text.activity_idle],
    ['approval',text.activity_board_waiting],['input',text.activity_input],['completed',text.activity_completed],['failed',text.activity_failed]];
  const canvas = div().id('plaza-canvas').v_flex().size_full().min_w_0().min_h_0()
    .child(div().id('plaza-toolbar').h_flex().flex_none().w_full().items_center().gap_2().px_4().py_2()
      .child(div().flex_1().min_w_0().text_sm().text_color(colors.muted_foreground).child(text.floor(floor.page + 1,model.pages)))
      .child(new Button('plaza-floor-prev').ghost().size('small').disabled(floor.page === 0)
        .tooltip(text.previous).child(new Icon('icons/chevron-left.svg').size('small')).on_click(changeFloor(-1)))
      .child(new Button('plaza-floor-next').ghost().size('small').disabled(floor.page + 1 === model.pages)
        .tooltip(text.next).child(new Icon('icons/chevron-right.svg').size('small')).on_click(changeFloor(1))))
    .child(div().id('plaza-viewport').relative().flex_1().min_w_0().min_h_0().overflow_hidden()
      .child(ground(width,viewport.height))
      .child(div().id('plaza-scroll').absolute().inset(0).overflow_x_scrollbar().overflow_y_scrollbar()
        .child(div().id('plaza-scene-frame').w_full().min_w(WORLD_WIDTH * scale + 32).min_h('100%').v_flex().items_center().p_4().pb_8()
          .child(board.catalog ? scene(view,model,scale).my_auto() : board.error
            ? div().v_flex().items_center().justify_center().w(WORLD_WIDTH * scale).h(WORLD_HEIGHT * scale)
              .child(new Button('plaza-retry').ghost().label(text.retry).disabled(!!board.pending)
                .on_click((_event,cx) => cx.spawn(async () => view.board.refresh())))
            : div().w(WORLD_WIDTH * scale).h(WORLD_HEIGHT * scale)))))
    .child(div().id('plaza-legend').h_flex().flex_none().items_center().flex_wrap().gap_4().px_4().py_2().border_t_1().border_color(colors.border)
      .children(legend.map(([state,label]) => div().h_flex().items_center().gap_2().text_xs()
        .child(div().id(`plaza-legend-${state}`).w(9).h(9).rounded_full().bg(colors[statusColors[state]])).child(label)))
      .children(model.approvals.length > 6 ? [new Button('plaza-approvals').ghost().size('small')
        .label(text.approval_count(model.approvals.length)).on_click((_event,cx) => view.inspectApprovals(cx))] : [])
      .children(board.connected ? [] : [div().flex_1().text_right().text_xs().text_color(colors.muted_foreground).child(text.live_disconnected)]));
  return div().id('plaza-board').v_flex().size_full().min_w_0().min_h_0().text_color(colors.foreground)
    .child(header).child(Workspace.new('plaza-workspace',{has_details:!!side.panel,details_label:text.tasks,
      default_details_width:DETAIL_WIDTH,min_details_width:320,navigation:'none'})
      .children([canvas,...(side.panel ? [side.panel] : [])]))
    .children(side.released ? [div().absolute().w(0).h(0).child(side.released)] : []);
}

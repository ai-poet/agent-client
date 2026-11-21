import {div} from 'gpui-kit';
import {Icon} from 'gpui-component';
import {Conversation,theme} from 'sailry';
import {PanelHeader,IconButton,CardButton,ClampedText,EmptyState} from 'sailry/ui';
import {plain,clock,dotColor} from './state.js';
export const DETAIL_WIDTH = 360;

function card(view,item) {
  const colors = theme().colors, id = `plaza-detail-${item.kind}-${item.id}`;
  view.targets.set(id,item);
  return CardButton.new(id,{label:`${item.title}, ${item.status}`,tooltip:item.title})
    .child(div().id(`${id}-body`).v_flex().w_full().min_w_0().items_start().gap_3().text_left()
      .child(div().id(`${id}-title`).w_full().min_w_0().whitespace_normal().text_sm().font_semibold().line_height('1.25rem').child(item.title))
      .children(item.preview ? [ClampedText.new(`${id}-preview`,{text:plain(item.preview),lines:3})] : [])
      .child(div().id(`${id}-state`).h_flex().w_full().min_w_0().items_center().gap_2().text_sm()
        .child(div().size(8).flex_shrink(0).rounded_full().bg(colors[dotColor(item)]))
        .child(div().flex_1().min_w_0().whitespace_normal().child(item.status))
        .children(item.duration ? [div().flex_none().text_color(colors.muted_foreground).child(item.duration)] : []))
      .child(div().id(`${id}-metadata`).h_flex().w_full().min_w_0().items_center().gap_2().text_sm().text_color(colors.muted_foreground)
        .child(new Icon(item.kind === 'terminal' ? 'icons/square-terminal.svg' : 'icons/folder.svg').size('small'))
        .child(div().flex_1().min_w_0().truncate().child(item.project || view.text.sessions_unassigned))
        .children(item.time === null ? [] : [div().flex_none().child(clock(item.time))])));
}
function list(view,items) {
  return div().id('plaza-detail-list').v_flex().flex_1().w_full().min_w_0().min_h_0().p_3().gap_2().overflow_y_scrollbar()
    .child(div().text_sm().text_color(theme().colors.muted_foreground).child(view.text.task_count(items.length)))
    .children(items.length ? items.map(item => card(view,item))
      : [EmptyState.new('plaza-tasks-empty',{variant:'list',fill_height:true,icon:'inbox',label:view.text.no_tasks})]);
}
export function details(view,model) {
  const {text,selected} = view, session = view.panelOpen && selected?.kind === 'session';
  const current = session ? selected.id : null;
  // A different native instance preserves the SDK's captured-session contract.
  // Release the prior instance in the same render before mounting the selection.
  const released = view.chatSession && view.chatSession !== current
    ? Conversation.new(`plaza-detail-chat-${view.chatSession}`,{active:false}) : null;
  view.chatSession = current;
  if (!view.panelOpen) return {panel:null,released};
  const room = selected?.kind === 'room' ? model.rooms.find(room => room.key === selected.id) : null;
  const title = session ? selected.title : room?.name ?? text.approval_plaza;
  const header = PanelHeader.new('plaza-detail-header',{draggable:false,bordered:true})
    .child(div().h_flex().w_full().min_w_0().items_center().gap_2()
      .child(div().flex_1().min_w_0().truncate().text_sm().font_medium().child(title))
      .children(session ? [IconButton.new('plaza-open-conversation',{icon:'external-link',label:text.open_conversation})] : [])
      .child(IconButton.new('plaza-detail-close',{icon:'close',label:text.close})));
  return {released,panel:div().id('plaza-detail-panel').v_flex().size_full().min_w_0().min_h_0()
    .child(header)
    .child(session ? Conversation.new(`plaza-detail-chat-${selected.id}`,{session:selected.id,active:true})
      : list(view,room?.items ?? model.approvals))};
}

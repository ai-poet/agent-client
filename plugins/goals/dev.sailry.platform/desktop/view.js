import {div} from 'gpui-kit';
import {Button} from 'gpui-component';
import {theme} from 'sailry';
import {ScrollRegion} from 'sailry/ui';
import {labels,reasons,duration,count} from '../labels.js';

const pick=(label,locale)=>label.locales?.[locale] ?? label.label;
const of=(used,limit,format)=>limit ? `${format(used)} / ${format(limit)}` : format(used);

/** The goal's state line: its state and, when paused, the reason. */
export function stateLine(goal,locale) {
  const state=pick(labels[goal.state],locale);
  return goal.reason ? `${state} · ${pick(reasons[goal.reason],locale)}` : state;
}
/** The counters row: unattended turns of the limit, time and tokens. */
export function counters(goal,text) {
  return [`${text.turns} ${goal.automatic_turns} / ${goal.turn_limit}`,
    `${text.time} ${of(goal.elapsed_ms,goal.time_limit_ms,duration)}`,
    `${text.tokens} ${of(goal.tokens,goal.token_budget,count)}`];
}
function body(view) {
  const {text,goal}=view,colors=theme().colors,locale=view.scope.locale;
  if (!goal) return [div().id('goal-empty').text_sm().text_color(colors.muted_foreground).child(view.draft() || text.empty)];
  const tone=goal.state==='completed' ? colors.success : ['paused','blocked'].includes(goal.state) ? colors.warning : colors.muted_foreground;
  const note=goal.state==='completed' ? goal.evidence : goal.detail;
  return [
    div().id('goal-objective').text_sm().selectable().child(goal.objective),
    div().id('goal-state').text_xs().text_color(tone).child(stateLine(goal,locale)),
    div().id('goal-counters').h_flex().flex_wrap().gap_3().text_xs().text_color(colors.muted_foreground)
      .children(counters(goal,text).map((line,index)=>div().id(`goal-counter-${index}`).child(line))),
    ...(note ? [div().id('goal-note').v_flex().gap_1().text_xs()
      .children(goal.state==='completed' ? [div().id('goal-note-label').text_color(colors.muted_foreground).child(text.evidence)] : [])
      .child(div().id('goal-note-text').selectable().child(note))] : []),
  ];
}
function earlier(view) {
  const {text,history}=view,colors=theme().colors;
  if (!history.length) return [];
  return [div().id('goal-earlier').v_flex().gap_1().pt_2().text_xs().text_color(colors.muted_foreground)
    .child(div().id('goal-earlier-title').font_medium().child(text.earlier))
    .children(history.map(item=>div().id(`goal-earlier-${item.id}`).h_flex().gap_2().min_w_0()
      .child(div().id(`goal-earlier-text-${item.id}`).flex_1().min_w_0().truncate().child(item.objective))
      .child(div().id(`goal-earlier-outcome-${item.id}`).child(`${text[item.outcome]} · ${duration(item.elapsed_ms)}`))))];
}
function buttons(view) {
  const {text,goal}=view;
  const idle=view.busy || view.loading,disabled=idle || !view.state.connected || view.state.readonly || !!view.pending;
  const button=(id,label,action)=>new Button(id).size('small').label(label).disabled(disabled).on_click(action);
  // An unconfirmed action keeps its request; Retry completes that request.
  const result=view.pending ? [new Button('goal-retry').size('small').label(text.retry).disabled(idle).on_click((_,cx)=>view.execute(cx))] : [];
  if (!goal) return result;
  if (goal.status==='active') result.push(button('goal-pause',text.pause,(_,cx)=>view.perform('pause',cx)));
  else result.push(button('goal-resume',goal.status==='completed' ? text.reopen : text.resume,(_,cx)=>view.perform('resume',cx)));
  result.push(button('goal-end',text.end,(_,cx)=>view.perform('end',cx)).ghost());
  return result;
}

export function render(view) {
  return div().id('goal-panel').v_flex().gap_2().w_full()
    .child(ScrollRegion.new('goal-scroll',{max_height:360}).children([...body(view),...earlier(view)]))
    .child(div().id('goal-actions').h_flex().justify_end().gap_2().children(buttons(view)));
}

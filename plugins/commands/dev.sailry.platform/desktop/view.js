import {div} from 'gpui-kit';
import {Button, Tab, TabBar, Spinner} from 'gpui-component';
import {Terminal, theme} from 'sailry';
import {Menu} from 'sailry/ui';
import {choices} from './access.js';

export function render(view) {
  const {text} = view, disabled = view.busy || !!view.pending;
  const selected = view.items.find(item => item.id === view.selected);
  const closing = view.closingTerminals?.has(selected?.id);
  const attached = selected && selected.status.kind !== 'stopped' && !closing;
  const stoppedShell = selected?.status.kind === 'stopped' && !selected.tool && !selected.ssh;
  const waiting = !selected && (!view.loaded || view.loading || disabled || !!view.resource);
  const spinning = waiting && !view.error && !view.resource;
  const status = closing ? null : selected ? (opening(view, selected) ? null : text.stopped) : waiting ? null : text.empty;
  return div().id('commands-page').v_flex().size_full().min_w_0().min_h_0()
    .children(!view.resource ? [div().h_flex().flex_shrink(0).gap_1().px_2().py_1()
      .child(new Button('terminal-new').ghost().label(text.create).disabled(disabled)
        .on_click((_, cx) => view.perform('create', null, cx)))
      .child(Menu.new('terminal-access', {label:text.access, disabled, items:[
        {id:'unrestricted',label:`${text.create} · ${text.full}`,enabled:true},
        {id:'workspace',label:`${text.create} · ${text.workspace}`,enabled:true},
      ]}))
      .child(Menu.new('terminal-tools', {label:text.cli, disabled,
        items:choices(view.tools,text)}))
      .child(div().flex_1())
      .children(selected ? [new Button('terminal-close').ghost().label(text.close).disabled(disabled)
        .on_click((_, cx) => view.perform('close', selected.id, cx))] : [])] : [])
    .children(!view.resource && view.items.length > 1 ? [new TabBar('terminal-tabs').variant('underline')
      .selected_index(view.items.findIndex(item => item.id === view.selected))
      .children(view.items.map((item, index) => new Tab().label(view.label(item, index))))
      .on_change((index, cx) => view.select(index, cx))] : [])
    .children(view.error && (view.pending || view.openRetry) ? [div().h_flex().gap_2().px_3().py_2()
      .child(new Button('terminal-retry').ghost().label(text.retry).disabled(view.busy)
        .on_click((_, cx) => view.retry(cx)))] : [])
    .children(!attached && (!stoppedShell || !view.error) ? [div().flex_1().v_flex().items_center().justify_center()
      .children(spinning ? [new Spinner()] : status ? [status] : [])] : [])
    .child(div().id(`terminal-content-${selected?.status.kind ?? 'empty'}`)
      .flex_grow(attached ? 1 : 0).min_h_0().min_w_0().children([
      Terminal.new('commands-terminal', {terminal:attached ? selected.id : null}),
    ]));
}

// A stopped shell that is being opened, or whose open awaits the refreshed
// inventory, is transient: render the plain terminal background.
export function opening(view, item) {
  if (item?.status.kind !== 'stopped' || item.tool || item.ssh || view.error) return false;
  const attempt = view.openAttempts?.get(item.id);
  if (attempt && attempt.revision === item.revision) return !attempt.failed;
  const intent = view.openIntent;
  return !!intent && (intent.target === null || intent.target === item.id);
}

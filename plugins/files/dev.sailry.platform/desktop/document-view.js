// Document composition retained from Sailry 116aab0f; the surfaces own editing.
import {div} from 'gpui-kit';
import {StatusBar} from 'gpui-component';
import {Header,theme} from 'sailry';
import {DocumentSurface} from 'sailry/documents';
import {PanelHeader, NavigationTabs, IconButton, Menu, SegmentedTabs, Measure, NativeContextMenu, EmptyState} from 'sailry/ui';

export function header(view) {
  const {documents,text} = view;
  const tabs = {
      id:'file-tabs',
      items:documents.items().map(document => ({id:document.id,
        label:document.path.split('/').at(-1),dirty:document.dirty,closable:true,
        close_label:`${text.close} ${document.path}`})),
      selected:documents.selected,max_width:180,close_label:text.close
    };
  if (view.mode === 'main') return Header.new('files-header',{content:JSON.stringify({title:text.files,tabs})});
  return PanelHeader.new('file-tabs-header',{padding:8}).child(div().w_full().min_w_0()
    .child(NavigationTabs.new(tabs.id,tabs)));
}

function separator(colors) {
  return div().w_px().h_4().mx_1().flex_shrink(0).bg(colors.border);
}

// Approximate pixel widths for collapsing; scripts cannot measure each control.
// SWITCH is Kit's icon-only segmented bar (two 30px segments, 2px gap, 4px padding) plus the row gap.
const BUTTON = 32, SEPARATOR = 10, BLOCK = 132, SWITCH = 74, PADDING = 24;
export const toolbarMenus = new Set(['file-block-menu','file-insert-menu','file-more-menu']);

function blockLabel(text, block) {
  if (!block) return text.files_block;
  if (block.startsWith('heading')) return text.files_block_heading.replace('{level}',block.slice(7));
  return text[`files_block_${block}`];
}

function blockItems(text, block) {
  const item = (id,shortcut) => ({id:`block:${id}`,label:blockLabel(text,id),shortcut,enabled:true,checked:id === block});
  return [item('paragraph','secondary-alt-0'),
    ...[1,2,3,4,5,6].map(level => item(`heading${level}`,`secondary-alt-${level}`)),
    {separator:true},item('code'),item('callout')];
}

function insertItems(text, disabled) {
  const item = (id,icon) => ({id:`insert:${id}`,label:text[`files_insert_${id}`],icon,enabled:!disabled});
  return [item('image','image'),item('rule','separator-horizontal'),item('code','square-code'),{separator:true},
    {header:true,label:text.files_insert_diagrams},item('mermaid','workflow'),item('math','sigma'),item('chart','chart-bar'),
    {separator:true},item('html','file-code')];
}

// Groups that collapse right to left into the overflow menu.
export function groups(view, document) {
  const {text} = view, format = document.format ?? {}, readonly = !document.can_edit;
  const button = (id,icon,options = {}) => ({id,icon,label:options.label ?? text[id],shortcut:options.shortcut,
    selected:!!options.selected,disabled:options.disabled ?? readonly});
  const find = button('files_find','search',{disabled:false});
  if (document.notebook && document.mode === 'document') return [[find]];
  const wrap = [button('files_wrap','text-wrap',{selected:document.wrap,disabled:false})];
  if (!document.markdown) return [[find,button('files_replace','replace')],wrap];
  const block = (id,icon) => button(`block:${id}`,icon,{label:blockLabel(text,id),selected:format.block === id});
  return [
    [button('files_italic','italic',{shortcut:'secondary-i',selected:format.italic}),
      button('files_strike','strikethrough',{shortcut:'secondary-shift-x',selected:format.strike}),
      button('files_inline_code','code',{shortcut:'secondary-e',selected:format.code}),
      button('files_link','link',{shortcut:'secondary-k',selected:format.link})],
    [block('bullet','list'),block('ordered','list-ordered'),block('task','list-todo'),block('quote','quote'),
      button('insert:table','table',{label:text.files_insert_table}),
      {...button('files_insert','plus'),menu:true}],
    [find],
    ...(document.mode === 'source' ? [wrap] : []),
  ];
}

// Splits groups into visible ones and the overflow, keeping whole controls.
export function fit(groups, room) {
  const items = groups.flatMap((group,index) => group.map((item,position) =>
    ({item,group:index,width:BUTTON + (position === 0 ? SEPARATOR : 0)})));
  let used = items.reduce((sum,entry) => sum + entry.width,0), count = items.length;
  if (room !== undefined && used > room) {
    used += BUTTON;
    while (count > 0 && used > room) used -= items[--count].width;
  }
  const split = entries => entries.reduce((parts,entry) => {
    if (parts.at(-1)?.index !== entry.group) parts.push({index:entry.group,items:[]});
    parts.at(-1).items.push(entry.item);
    return parts;
  },[]).map(part => part.items);
  return {visible:split(items.slice(0,count)),hidden:split(items.slice(count))};
}

function overflow(view, hidden) {
  const items = hidden.flatMap((group,index) => [...(index ? [{separator:true}] : []),
    ...group.map(item => item.menu
      ? {label:item.label,icon:item.icon,children:insertItems(view.text,item.disabled)}
      : {id:item.id,label:item.label,icon:item.icon,shortcut:item.shortcut,enabled:!item.disabled,checked:item.selected})]);
  return Menu.new('file-more-menu',{label:view.text.files_more,icon:'ellipsis',items});
}

function control(view, item) {
  if (item.menu) return Menu.new('file-insert-menu',{label:item.label,icon:item.icon,disabled:item.disabled,
    items:insertItems(view.text,item.disabled)});
  return IconButton.new(item.id,{icon:item.icon,label:item.label,shortcut:item.shortcut,
    selected:item.selected,disabled:item.disabled});
}

export function toolbar(view, document, colors) {
  const {text} = view, readonly = !document.can_edit;
  const preview = document.notebook && document.mode === 'document';
  const button = (id,icon,shortcut) => IconButton.new(id,{icon,label:text[id],shortcut,disabled:readonly});
  const fixed = [
    IconButton.new('files_save',{icon:'circle-check',label:text.files_save,shortcut:'secondary-s',
      disabled:!document.can_save,tone:document.dirty && !document.saving ? 'success' : 'muted'}),
    ...(preview ? [] : [separator(colors),button('files_undo','undo','secondary-z'),button('files_redo','redo','secondary-shift-z')]),
    ...(document.markdown ? [separator(colors),
      Menu.new('file-block-menu',{label:blockLabel(text,document.format?.block),caret:true,small:true,disabled:readonly,
        items:blockItems(text,document.format?.block)}),
      button('files_bold','bold','secondary-b')] : []),
  ];
  const mode = document.markdown || document.notebook;
  const reserved = PADDING + BUTTON + (preview ? 0 : SEPARATOR + 2 * BUTTON)
    + (document.markdown ? SEPARATOR + BLOCK + BUTTON : 0) + (mode ? SEPARATOR + SWITCH : 0);
  const all = groups(view,document);
  const {visible,hidden} = fit(all,view.toolbarWidth === undefined ? undefined : view.toolbarWidth - reserved);
  // Formatting groups stay left; Find and wrap sit after the spacer.
  const trailing = document.markdown ? 2 : 0;
  const render = list => list.flatMap(group => [separator(colors),...group.map(item => control(view,item))]);
  // Italic continues Bold's group, so the first formatting group has no separator.
  const leading = render(visible.slice(0,trailing)).slice(document.markdown ? 1 : 0);
  return div().id('file-toolbar').v_flex().w_full().flex_shrink(0).border_b_1().border_color(colors.border)
    .child(Measure.new('file-toolbar-width'))
    .child(div().h_flex().w_full().h_12().px_3().gap_1().overflow_hidden()
      .children(fixed)
      .children(leading)
      .child(div().flex_1())
      .children(render(visible.slice(trailing)).slice(visible.length > trailing ? 1 : 0))
      .children(hidden.length ? [overflow(view,hidden)] : [])
      // Kit's tab bar scrolls when squeezed, which clips its padding; keep the switch at its own width.
      .children(mode ? [separator(colors),div().h_flex().flex_shrink(0).child(SegmentedTabs.new('file-editor-mode',{
        selected:document.mode,items:[
          {id:'document',label:document.notebook ? text.content_preview : text.content_document,icon:'eye'},
          {id:'source',label:text.content_source,icon:'code-xml'}]}))] : []));
}

function footer(view, document, colors) {
  const {text} = view;
  const status = document?.saving ? 'files_saving' : document?.truncated ? 'files_content_partial' : null;
  const path = div().id('document-path-label').flex_1().min_w_0().truncate().child(document?.path ?? '');
  const details = document ? div().id('document-status-details').h_flex().flex_shrink(0).gap_2()
    .child(div().id('document-cursor-position').child(text.files_cursor_position
      .replace('{line}',String(document.position.line)).replace('{column}',String(document.position.column))))
    .child(div().id('document-encoding').child('UTF-8'))
    .child(div().id('document-language').child(document.language)) : div();
  return div().id('document-path').flex_shrink(0).min_w_0()
    .child(new StatusBar().h_8().flex_shrink(0).border_t_0().bg('#00000000')
      .left_content(document ? NativeContextMenu.new(`document-path-${document.id}`,{items:view.pathItems(document)}).child(path) : path)
      .children(status ? [div().truncate().child(text[status])] : [])
      .right_content(details));
}

function empty(text) {
  return EmptyState.new('file_empty',{icon:'file',label:text.file_empty});
}

export function content(view) {
  const document = view.documents.current(), colors = theme().colors;
  return div().id('file-document').v_flex().size_full().min_h_0().min_w_0()
    .children(document ? [toolbar(view,document,colors)] : [])
    .child(div().flex_1().min_h_0().min_w_0()
      .child(document ? DocumentSurface.new(`file-editor-${document.id}`,{document:document.id}) : empty(view.text)))
    .children(document ? [footer(view,document,colors)] : []);
}

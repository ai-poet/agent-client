import {div} from 'gpui-kit';
import {Button,Icon,Pagination,Resizable,ResizablePanel} from 'gpui-component';
import {Conversation,Header,theme} from 'sailry';
import {Workspace,EmptyState,Loading,InputSurface,Picker,SelectableRow,IconButton,Tooltip,ResourceTree,DataTable,Modal,NativeContextMenu} from 'sailry/ui';
import {TextField,readText} from 'sailry/forms';
import {command,cell,historyEntries,historyRevision} from './main.js';
import {format} from './locales.js';
import {editor,editorFooter} from './editor-view.js';

function description(profile,text) {
  const connection=profile.connection;
  if(connection.kind==='sqlite')return `SQLite · ${connection.path}`;
  if(connection.kind==='socket')return `${text.db_transport_socket} · ${connection.path}`;
  return `${text[connection.engine==='mysql'?'db_mysql':'db_postgres']} · ${connection.username}@${connection.host}:${connection.port}`;
}
function picker(view) {
  const {text}=view,profile=view.profiles.find(profile=>profile.id===view.pickerProfile),disabled=view.locked();
  const actions=[['open','connection_action_open','arrow-right'],['edit','connection_action_edit','settings-2'],['check','connection_action_check','check'],['remove','settings_delete','delete']];
  const items=profile?[...actions.map(([action,label,icon])=>({id:`connection-action-${action}`,label:text[label],icon,disabled,value:{kind:'action',action}})),
    {id:'connection-back',label:text.reference_back,icon:'chevron-left',value:{kind:'back'}}]:[
    ...view.profiles.map((profile,index)=>({id:`connection-choice-${index}`,label:profile.name,detail:description(profile,text),icon:'hard-drive',submenu:true,disabled,value:{kind:'profile',id:profile.id}})),
    {id:'connection-add',label:text.db_new,icon:'plus',disabled,value:{kind:'add'}}];
  return Picker.new(`db-picker-${view.pickerRevision}`,{mode:'command',open:view.picker,title:text.connection_search,empty:text.connection_search_empty,items});
}
function landing(view) {
  const {text}=view;
  return EmptyState.new('db_workspace_empty',{icon:'hard-drive',label:text.db_workspace_empty})
    .child(div().w_full().max_w_96().child(IconButton.new('db-new',{icon:'plus',label:text.db_new,show_label:true,full_width:true,variant:'primary',size:'medium',disabled:view.locked()})))
    .child(div().id('db-saved-connections').v_flex().w_full().h_auto().max_w_96().min_h_0().max_h(320).flex_initial()
      .child(div().id('db-saved-connections-scroll').v_flex().w_full().flex_1().min_h_0().overflow_y_scrollbar().gap_2()
        .children(view.profiles.map(profile=>NativeContextMenu.new(`db-connection-${profile.id}`,{fill:false,items:[['edit','connection_action_edit'],['remove','settings_delete']].map(([action,label])=>({id:`connection:${JSON.stringify({id:profile.id,revision:profile.revision,action})}`,label:text[label],enabled:!view.locked()}))})
          .child(SelectableRow.new(`db-open-${profile.id}`,{variant:'card',label:profile.name,disabled:view.locked(),selected:false})
          .child(div().h_flex().w_full().gap_3().child(new Icon('icons/hard-drive.svg').size_5())
            .child(div().v_flex().flex_1().min_w_0().gap_0p5().child(div().text_sm().truncate().child(profile.name))
              .child(div().text_sm().truncate().text_color(theme().colors.muted_foreground).child(description(profile,text))))
            .child(new Icon('icons/chevron-right.svg').size_4())))))));
}
function feedback(view,form=false) {
  const {pending,text}=view;
  const content=div().v_flex().min_w_0();
  if(form)content.gap_3();else content.gap_2().px_3().py_1();
  return content
    .children(pending?[div().id('db-pending').h_flex().gap_2().items_center()
      .child(div().flex_1().text_sm().child(pending.request.unknown?'':text.db_running))
      .children(pending.request.unknown?[new Button('db-check-result').label(text.db_check_result).disabled(pending.request.running).on_click((_,cx)=>view.run(cx=>view.check(cx),cx))]:[])
      .children(['query_database','check_database'].includes(pending.request.command?.kind)?[new Button('db-stop').label(text.db_stop).on_click((_,cx)=>view.run(()=>pending.request.cancel(),cx))]:[])]:[]);
}
function results(view) {
  const {result,text}=view,colors=theme().colors;
  const count=result?.rows.length??0,pages=Math.max(1,Math.ceil(count/50));
  // The table fills the page, whose edges bound it, so it draws no frame of its own.
  return div().id('db-results').v_flex().size_full().min_h_0().min_w_0()
    .child(div().flex_1().min_h_0().min_w_0().child(DataTable.new('database-results',{revision:String(view.epoch),columns:result?.columns??[],rows:(result?.rows??[]).map(row=>row.map(cell)),page:view.page,menu:view.rowMenu(),stripe:true,header:!!result?.columns.length,bordered:false,empty:text.db_data_empty,empty_icon:'table'})))
    .children(result?.columns.length?[div().id('db-results-footer').h_flex().h_9().flex_none().px_3().gap_3().items_center().text_sm().text_color(colors.muted_foreground).border_t_1().border_color(colors.border)
      .child(div().id('db-results-count').flex_none().child(format(text.db_rows,{count})))
      .children(result.truncated?[div().id('db-results-truncated').flex_none().child(text.db_truncated)]:[])
      .child(div().flex_1())
      .children(pages>1?[new Pagination('db-pages').size('small').current_page(view.page+1).total_pages(pages).visible_pages(3).on_change((page,cx)=>{view.page=page-1;cx.notify();})]:[])]:[]);
}
// The history is the results' table: newest first, one row a query, its
// tooltip the time, the full SQL and any error. Selection, arrows, Enter or
// a double click (open in SQL), ⌘C (copy the query) and the row menu are
// the table's own.
function log(view) {
  const {text}=view,entries=historyEntries(view.logs);
  const status=entry=>{
    const result=entry.result?.rows??entry.request?.output?.data?.data,error=entry.result?.error??entry.request?.error;
    return {error,label:entry.request?.unknown?text.db_unknown:error?text.db_failed:result?format(text.db_rows,{count:result.columns.length?result.rows.length:result.affected_rows}):text.db_running};
  };
  const rows=entries.map(entry=>[entry.sql,status(entry).label,entry.elapsed==null?'':`${entry.elapsed.toFixed(2)}s`]);
  return div().id('database-history').v_flex().flex_1().min_h_0().min_w_0()
    .child(DataTable.new('database-history-table',{revision:historyRevision(view.opened,view.logs),columns:[text.db_sql_tab,text.db_log_result,text.db_log_duration],rows,
      cells:entries.map((entry,index)=>{const {error}=status(entry);return rows[index].map((value,column)=>({primary:{text:value,...(column===1&&error?{text_tone:'warning'}:{})}}));}),
      details:entries.map(entry=>{const {error}=status(entry),tip=`${new Date(entry.timestamp).toLocaleTimeString()}\n${entry.sql}${error?.message?`\n${error.message}`:''}`;return [tip,tip,tip];}),
      row_ids:entries.map(entry=>entry.id),widths:[200,160,80],alignments:['start','start','end'],fill:0,page:view.historyPage,
      menu:[{id:'copy',label:text.db_copy_query,enabled:true},{id:'open',label:text.db_open_sql,enabled:!view.locked()}],
      stripe:true,header:true,bordered:false,empty:text.db_history_empty,empty_icon:'clock'}));
}
// The action that runs, stops or checks the query: Run while idle, Stop
// while it runs, Check result when its outcome is unknown.
function queryButton(view,props) {
  const {text,pending}=view,unknown=pending?.request.unknown;
  return IconButton.new(pending?unknown?'db-inspect':'db-stop':'db-run',{
    icon:pending?unknown?'rotate-cw':'square':'play',label:text[pending?unknown?'db_inspect':'db_stop':'db_run'],
    disabled:!pending&&(view.locked()||!readText(view.sql).trim()),...props,
  });
}
// The SQL dialog's body: a SQL editor with line numbers, ⌘↵ running it.
function sqlEditor(view) {
  return div().id('db-sql').v_flex().w_full().child(TextField.new(view.sql,{disabled:view.locked()}));
}
// The SQL dialog's footer: the database it runs in, then Cancel and Run,
// Run becoming Stop while the query runs and Check result when its outcome
// is unknown.
function sqlFooter(view) {
  const {text}=view,colors=theme().colors;
  return div().id('db-sql-footer').h_flex().w_full().min_w_0().gap_2().items_center()
    .child(div().id('db-execution-target').h_flex().flex_1().min_w_0().gap_2().items_center().text_sm().text_color(colors.muted_foreground)
      .child(new Icon('icons/database.svg').size_4())
      .child(div().min_w_0().truncate().child(view.catalog.database??view.profile().name)))
    .child(new Button('db-sql-cancel').label(text.settings_cancel).on_click((_,cx)=>view.close(cx)))
    .child(queryButton(view,view.pending?{show_label:true,size:'medium'}:{show_label:true,size:'medium',variant:'primary',shortcut:'secondary-enter'}));
}
function historyPanel(view) {
  const {text,pending}=view,colors=theme().colors;
  // A query run from the SQL dialog keeps its Stop here once the dialog is closed.
  const running=pending&&!view.sqlOpen&&pending.request.command?.kind==='query_database';
  return div().id('database-history-panel').v_flex().size_full().min_h_0().min_w_0().border_t_1().border_color(colors.border)
    .child(div().id('database-history-bar').h_flex().h_10().flex_none().items_center().pl_3().pr_2().gap_1()
      .child(div().flex_1().min_w_0().truncate().text_sm().font_medium().child(text.db_history))
      .children(view.logs.length>50?[new Pagination('db-history-pages').size('small').current_page(view.historyPage+1).total_pages(Math.ceil(view.logs.length/50)).visible_pages(2).on_change((page,cx)=>{view.historyPage=page-1;cx.notify();})]:[])
      .children(running?[queryButton(view,{size:'small'})]:[])
      .child(IconButton.new('db-new-query',{icon:'plus',label:text.db_new_query,show_label:true,size:'small',disabled:view.locked()})))
    .child(log(view));
}
function content(view) {
  if(view.home||!view.profile())return div().id('db-landing').v_flex().size_full().min_h_0().children(view.pending?[feedback(view)]:[]).child(landing(view));
  const panels=new Resizable('db-editor-results').axis('vertical')
    .child(new ResizablePanel().size_range(100,10000).child(results(view)))
    .child(new ResizablePanel().size(200).size_range(120,480).flex_none().child(historyPanel(view)));
  return div().id('db-page').v_flex().size_full().min_h_0().min_w_0().child(div().flex_1().min_h_0().overflow_hidden().child(panels));
}
function navigation(view) {
  const {text,catalog}=view;
  if(view.home||!view.profile())return div();
  return div().id('db-catalog').v_flex().size_full().min_h_0().min_w_0()
    .child(div().flex_1().min_h_0().p_1().child(ResourceTree.new('database-tree',{guides:true,items:JSON.stringify(catalog.items(text,(table,database)=>view.tableMenu(table,database))),current:catalog.current,selected:[]})));
}

export function render(view) {
  const {text}=view,current=view.profile(),profile=view.home?null:current,query=view.sqlOpen&&!!profile&&!view.editor&&!view.confirmation,opened=!!view.editor||!!view.confirmation||query;
  const form=view.editor?{form:true,title:text[view.editor.original.revision?'db_edit':'db_new'],width:560}
    :view.confirmation?{form:true,title:text[view.confirmation.label],width:568}:query?{form:true,title:text.db_new_query,width:720}:{};
  const editorBody=view.editor?div().v_flex().gap_4().child(editor(view))
    .children(view.pending?[feedback(view,true)]:[]):null;
  const confirmationBody=view.confirmation?div().id('db-confirm').v_flex().gap_3()
    .child(div().whitespace_normal().text_sm().child(view.confirmation.text)):null;
  const confirmationFooter=view.confirmation?div().h_flex().justify_end().gap_2()
    .child(new Button('db-confirm-cancel').label(text.settings_cancel).on_click((_,cx)=>view.close(cx)))
    .child(new Button('db-confirm-submit').danger().label(text[view.confirmation.label]).disabled(view.locked()).on_click((_,cx)=>view.confirm(cx))):null;
  const connecting=!opened&&(!!view.pending&&!view.pending.request.unknown&&view.pending.request.command?.kind==='check_database'||profile&&view.catalog.loading.size&&!view.catalog.names.length);
  return div().size_full().min_h_0()
    .child(Header.new('database-header',{content:JSON.stringify({title:text.database,
      tabs:current?{id:'db-tabs',selected:view.home?null:current.id,max_width:180,close_label:text.close,items:[{id:current.id,label:current.name,icon:'hard-drive',closable:true,close_disabled:!!view.pending}]}:undefined,
      actions:[{id:'db-refresh-catalog',label:text.refresh,icon:'icons/rotate-cw.svg',disabled:!profile||!!view.catalog?.loading.size||view.locked()},
        {id:'db-connections-menu',label:text.db_connections,icon:'icons/hard-drive.svg',disabled:view.locked()},
        {id:'db-home',label:text.home,icon:'icons/house.svg',disabled:view.locked()||!profile}]})}))
    .child(Workspace.new('database-workspace',{has_details:!!profile,details_label:text.connection_assistant,min_navigation_width:200,min_details_width:320,navigation:profile?'resource':'none'}).children([
      div().relative().size_full().min_h_0().child(content(view)).children(connecting?[Loading.new('db-connecting')]:[]),
      profile?div().id('db-assistant-panel').size_full().min_h_0().child(Conversation.new(`database-assistant-${view.opened}`,{assistant:'database',resource:{kind:'database',id:view.opened},heading:text.connection_assistant})):div(),
      navigation(view)
    ])).child(picker(view)).child(Modal.new(`db-dialog-${view.dialog}`,{open:opened,...form})
      .children(view.editor?[editorBody,editorFooter(view)]:view.confirmation?[confirmationBody,confirmationFooter]:query?[sqlEditor(view),sqlFooter(view)]:[]));
}

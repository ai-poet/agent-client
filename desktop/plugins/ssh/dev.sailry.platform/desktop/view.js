import {div} from 'gpui-kit';
import {Button,VForm,Field,Progress,Icon,Spinner} from 'gpui-component';
import {Conversation,Terminal,Header,theme} from 'sailry';
import {Workspace,PanelHeader,PathBar,EmptyState,Loading,Picker,SelectableRow,IconButton,SelectField,ResourceTree,NativeContextMenu,Modal,ActionScope} from 'sailry/ui';
import {TextField} from 'sailry/forms';
import {format} from './locales.js';
import {editor,editorFooter} from './editor-view.js';
const description=profile=>`${profile.username}@${profile.host}:${profile.port}`;
function picker(view){const{text}=view,profile=view.profiles.find(profile=>profile.id===view.pickerProfile),disabled=view.locked();
  const actions=[['open','connection_action_open','arrow-right'],['edit','connection_action_edit','settings-2'],['check','connection_action_check','check'],['transfer','ssh_transfer','replace'],['run','ssh_run','play'],['remove','settings_delete','delete']];
  const items=profile?[...actions.map(([action,label,icon])=>({id:`connection-action-${action}`,label:text[label],icon,disabled,value:{kind:'action',action}})),
    {id:'connection-back',label:text.reference_back,icon:'chevron-left',value:{kind:'back'}}]:[
    ...view.profiles.map((profile,index)=>({id:`connection-choice-${index}`,label:profile.name,detail:description(profile),icon:'network',submenu:true,disabled,value:{kind:'profile',id:profile.id}})),
    {id:'connection-add',label:text.ssh_new,icon:'plus',disabled,value:{kind:'add'}}];
  return Picker.new(`ssh-picker-${view.pickerRevision}`,{mode:'command',open:view.picker,title:text.connection_search,empty:text.connection_search_empty,items});}
function landing(view){const{text}=view;return EmptyState.new('ssh_workspace_empty',{icon:'square-terminal',label:text.ssh_workspace_empty})
  .child(div().w_full().max_w_96().child(IconButton.new('ssh-new',{icon:'plus',label:text.ssh_new,show_label:true,full_width:true,variant:'primary',size:'medium',disabled:view.locked()})))
  .child(div().id('ssh-saved-connections').v_flex().w_full().h_auto().max_w_96().min_h_0().max_h(320).flex_initial()
    .child(div().id('ssh-saved-connections-scroll').v_flex().w_full().flex_1().min_h_0().overflow_y_scrollbar().gap_2()
      .children(view.profiles.map(profile=>NativeContextMenu.new(`ssh-connection-${profile.id}`,{fill:false,items:[['edit','connection_action_edit'],['remove','settings_delete']].map(([action,label])=>({id:`connection:${JSON.stringify({id:profile.id,revision:profile.revision,action})}`,label:text[label],enabled:!view.locked()}))})
        .child(SelectableRow.new(`ssh-open-${profile.id}`,{variant:'card',label:profile.name,disabled:view.connectionLocked(profile),selected:false})
        .child(div().h_flex().w_full().gap_3().child(new Icon('icons/network.svg').size_5())
          .child(div().v_flex().flex_1().min_w_0().gap_0p5().child(div().text_sm().truncate().child(profile.name))
            .child(div().text_sm().truncate().text_color(theme().colors.muted_foreground).child(description(profile))))
          .child(new Icon('icons/chevron-right.svg').size_4())))))));}
function feedback(view,form=false){const{text,outcome,pending}=view,content=div().v_flex().min_w_0();if(form)content.gap_3();else content.gap_2().px_3().py_2();return content
  .children(outcome?.kind==='completed'?[
    ...[outcome.stdout,outcome.stderr].flatMap((value,index)=>value?[new Field().label(text[index?'command_stderr':'command_stdout']).child(TextField.new(view.output[index],{readonly:true}))]:[]),
    ...(outcome.truncated?[div().text_sm().child(text.command_truncated)]:[])]:[])
  .children(pending?[div().id('ssh-pending').h_flex().gap_2().items_center().child(div().text_sm().flex_1().child(pending.request.unknown?'':text.ssh_running))
    .children(pending.request.unknown?[new Button('ssh-inspect').label(text.ssh_inspect).disabled(pending.request.running).on_click((_,cx)=>view.run(cx=>view.inspect(cx),cx))]:[])
    .child(new Button('ssh-stop').label(text.ssh_stop).on_click((_,cx)=>view.run(()=>pending.request.cancel(),cx)))]:[]);}
// The remote files as the Files package shows its tree: the path and its
// actions in a PanelHeader, a centered spinner until the first listing, an
// empty or failed directory said in the panel, and the tree otherwise. A
// reload keeps the rows while the header's refresh turns into a spinner.
// The header's row draws the divider so it stays inside the header padding,
// and a click on its blank space, which no segment or button takes, opens
// the path editor.
function files(view){const files=view.files(),{text}=view;if(!files)return div();
  const busy=files.pending,empty=!files.entries?.length,loaded=files.revision>0;
  const body=empty&&!loaded&&!files.error?div().id('ssh-files-loading').flex_1().h_flex().items_center().justify_center().child(new Spinner())
    :empty&&files.error?EmptyState.new('ssh_files_failed',{variant:'list',fill_height:true,icon:'folder',label:text[files.error]??text.ssh_files_failed})
      .child(new Button('ssh-files-retry').ghost().label(text.refresh).disabled(busy).on_click((_,cx)=>view.run(cx=>files.load(files.path,cx),cx)))
    :empty&&!busy?EmptyState.new('files_directory_empty',{variant:'list',fill_height:true,icon:'folder',label:text.files_directory_empty})
    :empty?div().id('ssh-files-loading').flex_1().h_flex().items_center().justify_center().child(new Spinner())
    :div().flex_1().min_h_0().p_2().child(ResourceTree.new('ssh-files',{guides:true,open:'double',external_files:!view.locked(),items:JSON.stringify(files.items(path=>view.fileMenu(path),text)),selected:files.selected,current:files.current,menu:view.fileMenu('')}));
  const row=div().id('ssh-files-path-row').h_flex().size_full().min_w_0().gap_1().items_center().border_b_1().border_color(theme().colors.border);
  if(!files.editing)row.on_click((_,cx)=>view.editPath(cx));
  return div().id('ssh-files-panel').v_flex().size_full().min_h_0().min_w_0()
  .child(PanelHeader.new('ssh-files-path')
    .child(row
      .child(div().flex_1().min_w_0().on_key_down((event,cx)=>view.pathKey(event,cx)).child(files.editing
        ?TextField.new(files.input,{disabled:busy,size:'small'})
        :PathBar.new('ssh-files-pathbar',{segments:files.segments().map(segment=>segment.label),disabled:busy})))
      .children(files.editing?[]:[busy?div().id('ssh-files-busy').size_8().h_flex().items_center().justify_center().child(new Spinner())
        :IconButton.new('ssh-files-refresh',{icon:'rotate-cw',label:text.refresh,size:'medium',disabled:!loaded&&!files.error})])))
  .child(div().v_flex().flex_1().min_h_0().min_w_0().child(NativeContextMenu.new('ssh-files-root',{items:view.fileMenu('')}).child(body)))
  .children(view.jobs.filter(job=>job.profile===files.profile.id).map(job=>transferJob(view,job)));}
function transferJob(view,job){const{text}=view,progress=job.progress?job.progress.size?job.progress.copied/job.progress.size*100:100:null;return div().id(`ssh-transfer-${job.id}`).v_flex().p_2().gap_1().text_xs()
  .child(div().truncate().child(job.source))
  .children(progress!==null?[new Progress(`ssh-progress-${job.id}`).value(progress).accessibility_label(text.ssh_transfer)]:[])
  .children(job.can_replace?[div().child(text.ssh_files_exists)]:[])
  .child(div().h_flex().gap_1()
    .children(job.can_replace?[new Button(`ssh-replace-${job.id}`).size('small').label(text.ssh_files_replace).on_click((_,cx)=>view.transferAction(job,'replace',cx))]:[])
    .children(job.can_check?[new Button(`ssh-transfer-check-${job.id}`).size('small').label(text.ssh_inspect).on_click((_,cx)=>view.transferAction(job,'check',cx))]:[])
    .children(job.can_start?[new Button(`ssh-transfer-start-${job.id}`).size('small').label(text.retry).on_click((_,cx)=>view.transferAction(job,'start',cx))]:[])
    .child(new Button(`ssh-transfer-close-${job.id}`).ghost().size('small').label(job.can_cancel?text.settings_cancel:text.close).on_click((_,cx)=>view.transferAction(job,job.can_cancel?'cancel':'dismiss',cx))));}
function transfer(view){const{text,transfer:value}=view,items=view.worktrees.filter(tree=>tree.project).map(tree=>({id:tree.id,label:`${view.projects.find(project=>project.id===tree.project)?.name??''} · ${tree.path}`}));
  const trees=SelectField.new(`ssh-worktree-${view.dialog}`,{label:text.ssh_worktree,placeholder:text.ssh_no_project,selected:value.worktree,
    disabled:view.locked()||!items.length,items});
  return div().id('ssh-transfer-editor').v_flex().gap_4().child(new VForm().size('large').gap_y(8)
    .child(new Field().label(text.ssh_worktree).child(trees))
    .child(new Field().label(text.ssh_project_path).child(TextField.new(value.inputs[0],{disabled:view.locked()})))
    .child(new Field().label(text.ssh_remote_path).child(TextField.new(value.inputs[1],{disabled:view.locked()}))))
    .children(view.pending?[feedback(view,true)]:[]);}
function transferFooter(view){const{text,transfer:value}=view;return div().h_flex().justify_end().gap_2().flex_wrap()
      .child(new Button('ssh-transfer-cancel').label(text.settings_cancel).on_click((_,cx)=>view.close(cx)))
      .child(new Button('ssh-download').label(text.ssh_download).disabled(view.locked()||!value.worktree).on_click((_,cx)=>view.transferFile('download',cx)))
      .child(new Button('ssh-upload').primary().label(text.ssh_upload).disabled(view.locked()||!value.worktree).on_click((_,cx)=>view.transferFile('upload',cx)));}
function titled(title,body,footer,width=560){return {props:{form:true,title,width},children:[body,footer]};}
function dialog(view){const{text}=view;if(view.outcome?.kind==='host_key_required')return titled(text.ssh_trust_title,
  div().v_flex().gap_4().child(div().id('ssh-host-key').v_flex().gap_2()
    .child(div().text_color(theme().colors[view.outcome.changed?'destructive':'foreground']).child(text[view.outcome.changed?'ssh_key_changed':'ssh_key_unknown']))
    .child(div().whitespace_normal().text_sm().text_color(theme().colors.muted_foreground).child(view.outcome.key.fingerprint)))
    .children(view.pending?[feedback(view,true)]:[]),
  div().h_flex().justify_end().gap_2().child(new Button('ssh-trust-cancel').label(text.settings_cancel).on_click((_,cx)=>view.close(cx)))
    .child(new Button('ssh-trust').primary().label(text.ssh_trust).disabled(view.locked()).on_click((_,cx)=>view.trust(cx))));
  if(view.confirmation)return titled(text[view.confirmation.label],
    div().id('ssh-confirm').v_flex().gap_3().child(div().whitespace_normal().text_sm().child(view.confirmation.text)),
    div().h_flex().justify_end().gap_2()
      .child(new Button('ssh-confirm-cancel').label(text.settings_cancel).on_click((_,cx)=>view.close(cx)))
      .child(new Button('ssh-confirm-submit').danger().label(text[view.confirmation.label]).disabled(view.locked()).on_click((_,cx)=>view.confirm(cx))),528);
  if(view.editor)return titled(text[view.editor.original.revision?'ssh_edit':'ssh_new'],
    div().v_flex().gap_4().child(editor(view)).children(view.pending?[feedback(view,true)]:[]),editorFooter(view));
  if(view.fileEdit)return titled(text[{rename:'files_rename',new_file:'files_new_file',new_directory:'files_new_directory'}[view.fileEdit.action]],
    div().id('ssh-file-editor').v_flex().gap_3().child(TextField.new(view.fileEdit.input,{disabled:view.locked()})),
    div().h_flex().justify_end().gap_2().child(new Button('ssh-file-cancel').label(text.settings_cancel).on_click((_,cx)=>view.close(cx)))
      .child(new Button('ssh-file-submit').primary().label(text.settings_save).disabled(view.locked()).on_click((_,cx)=>view.submitFile(cx))),408);
  if(view.transfer)return titled(text.ssh_transfer,transfer(view),transferFooter(view));
  if(view.commandOpen)return titled(text.ssh_run,div().id('ssh-command').v_flex().gap_4()
    .child(TextField.new(view.command,{disabled:view.locked()})).children(view.pending||view.outcome?.kind==='completed'?[feedback(view,true)]:[]),
    div().h_flex().justify_end().gap_2().child(new Button('ssh-command-close').label(text.settings_cancel).on_click((_,cx)=>view.close(cx)))
      .child(new Button('ssh-run').primary().label(text.ssh_run).disabled(view.locked()).on_click((_,cx)=>view.runCommand(cx))));
  return {props:{},children:[]};}
export function render(view){const{text}=view,active=view.home?null:view.terminals.find(info=>info.id===view.terminal),profile=view.home?null:view.activeProfile(),opened=!!view.editor||!!view.confirmation||!!view.fileEdit||!!view.transfer||view.commandOpen||view.outcome?.kind==='host_key_required';
  const modal=opened?dialog(view):{props:{},children:[]};
  const opening=view.pending?.request.command?.kind==='open_ssh_terminal';
  const connecting=!opened&&!!view.pending&&!view.pending.request.unknown&&['check_ssh','open_ssh_terminal'].includes(view.pending.request.command?.kind);
  const body=div().id(!view.loaded?'ssh-loading':active?'ssh-page':'ssh-landing').relative().v_flex().size_full().min_h_0().min_w_0()
    .children(!opened&&!connecting&&!opening&&view.pending?[feedback(view)]:[])
    .child(!view.loaded||connecting?div().flex_1().min_h_0()
      :active?div().flex_1().min_h_0().child(view.closingTerminals?.has(active.id)?div():Terminal.new(`ssh-terminal-${active.id}`,{terminal:active.id})):landing(view))
    .children(connecting?[Loading.new('ssh-connecting')]:[]);
  return ActionScope.new('ssh-actions',{close:'ssh-close-current'})
    .child(Header.new('ssh-header',{content:JSON.stringify({title:text.ssh,tabs:view.terminals.length?{id:'ssh-tabs',selected:view.home?null:view.terminal,close_label:text.close,max_width:180,items:view.terminals.map(info=>({id:info.id,label:view.profiles.find(profile=>profile.id===info.ssh)?.name??text.ssh,closable:true,icon:'terminal'}))}:undefined,
      actions:[{id:'ssh-connections-menu',label:text.ssh_connections,icon:'icons/network.svg',disabled:view.locked()},
        {id:'ssh-home',label:text.home,icon:'icons/house.svg',disabled:view.locked()||!active}]})}))
    .child(Workspace.new('ssh-workspace',{has_details:!!profile,details_label:text.ssh_assistant,min_navigation_width:200,min_details_width:320,navigation:profile?'resource':'none'}).children([
      body,
      profile?div().id('ssh-assistant-panel').size_full().min_h_0().child(Conversation.new(`ssh-assistant-${profile.id}`,{assistant:'ssh',resource:{kind:'ssh',id:profile.id},heading:text.ssh_assistant})):div(),
      profile?files(view):div()
    ])).child(picker(view)).child(Modal.new(`ssh-dialog-${view.dialog}`,{open:opened,...modal.props}).children(modal.children));}

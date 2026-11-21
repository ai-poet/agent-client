import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import {Menus} from '../dev.sailry.platform/desktop/menus.js';
import {messages} from '../dev.sailry.platform/desktop/locales.js';

const source = (await readFile(new URL('../dev.sailry.platform/desktop/actions.js',import.meta.url),'utf8'))
  .replace(/^import .*;\n/gm,'').replace(/^export /gm,'');

function fixture(receipts = [],confirmed = true) {
  const prompts=[],fields=new Map(),drafts=new Set(),prepared=[],completed=[],forgotten=[],released=[],reported=[];
  let serial=0,refreshes=0;
  const Actions=vm.runInNewContext(`${source}\nActions`,{
    createText:value=>{const id=`field-${++serial}`;fields.set(id,value);return id;},
    readText:id=>fields.get(id),releaseText:id=>released.push(id),focusText:()=>{},
    prepareGitChange:draft=>{if(drafts.size>=16)throw Error('plugin request draft capacity exhausted');prepared.push(JSON.parse(JSON.stringify(draft)));const id=prepared.length===1?'captured-request':`captured-request-${prepared.length-1}`;drafts.add(id);return id;},
    completeRequest:async id=>{completed.push(id);const receipt=receipts.shift();if(receipt instanceof Error)throw receipt;return typeof receipt==='function'?receipt():receipt;},
    forgetRequest:id=>{forgotten.push(id);drafts.delete(id);},faultCode:()=> 'outcome_unknown',
    confirmAction:async prompt=>{prompts.push(prompt);return confirmed;},
  });
  const view={text:{},notify:()=>{},report:key=>reported.push(key),repo:{
    status:{head:'captured-head',branch:'main'},
    revisions:()=>({expected_head:'captured-head',expected_index:'captured-index'}),
    refresh:async()=>{refreshes++;},
  }};
  return {owner:new Actions(view),prompts,fields,drafts,prepared,completed,forgotten,released,reported,refreshes:()=>refreshes};
}

test('opening labels preserve plain dialogs and direct actions',async()=>{
  for(const locale of ['en','zh-CN']) {
    const state=fixture(),view=state.owner.view,switched=[];
    view.text=messages(locale);
    Object.assign(view.repo,{
      writable:()=>true,options:{},navigation:{},
      status:{head:'captured-head',branch:'main',entries:[]},
      branches:{entries:[{name:'topic',commit:'captured-head',current:false,remote:false}],
        sync:{tracking:{},remotes:['origin'],remote_urls:{origin:'https://example.com/repo.git'},
          tags:[{name:'v1',commit:'captured-head'}],stashes:[]}},
      branch:async(operation,target)=>switched.push({operation,target}),
    });
    const menus=new Menus(view),checkout=menus.root().find(item=>item.id==='git_checkout');
    assert.equal(checkout.label.endsWith('…'),true);
    await state.owner.invoke(checkout.value);
    assert.equal(state.owner.picker.title,view.text.git_search_branches);
    state.owner.picker=null;
    const direct=menus.management()[0].value.items.find(item=>item.value.operation==='switch');
    assert.equal(direct.label,view.text.git_checkout);
    assert.equal(direct.label.endsWith('…'),false);
    await state.owner.invoke(direct.value);
    assert.equal(switched.length,1);
    assert.equal(switched[0].operation,'switch');
    assert.equal(state.owner.dialog,null);
    assert.equal(state.owner.picker,null);
    const create=menus.tags().find(item=>item.id==='git_tag_create');
    assert.equal(create.label.endsWith('…'),true);
    await state.owner.invoke(create.value);
    assert.equal(view.text[state.owner.dialog.title].endsWith('…'),false);
    state.owner.close();
    const remove=menus.tags().find(item=>item.id==='git_tag_delete');
    assert.equal(remove.label.endsWith('…'),true);
    await state.owner.invoke(remove.value);
    assert.equal(state.owner.picker.title,view.text.git_tag_delete);
    assert.equal(state.owner.picker.title.endsWith('…'),false);
    await state.owner.invoke(state.owner.picker.items[0].value);
    assert.equal(state.owner.dialog.title,'git_tag_delete');
    assert.equal(view.text[state.owner.dialog.title].endsWith('…'),false);
    assert.equal(state.owner.dialog.action.delete_tag.name,'v1');
    state.owner.close();
  }
});

test('empty names report validation without replacing the form or draft',async()=>{
  for(const form of ['branch','rename','tag']) {
    const state=fixture();
    state.owner.form(form,{name:'existing',commit:'captured-head'});
    const dialog=state.owner.dialog,input=dialog.input;
    state.fields.set(input,'');
    if(dialog.extra)state.fields.set(dialog.extra,'Keep tag message');
    await state.owner.submit();
    assert.equal(state.owner.dialog,dialog);
    assert.equal(dialog.input,input);
    assert.equal(state.fields.get(input),'');
    if(dialog.extra)assert.equal(state.fields.get(dialog.extra),'Keep tag message');
    assert.equal(dialog.error,'git_branch_name_required');
    assert.deepEqual(state.reported,['git_branch_name_required']);
    assert.deepEqual(state.prepared,[]);
    assert.deepEqual(state.completed,[]);
    assert.deepEqual(state.forgotten,[]);
    assert.deepEqual(state.released,[]);
  }
});

test('unconfirmed receipts report and retry the captured request without losing input',async()=>{
  const state=fixture([{}, {Ok:{kind:'git_change',data:{}}}]);
  state.owner.form('branch');
  const dialog=state.owner.dialog;
  state.fields.set(dialog.input,'topic');
  await state.owner.submit();
  assert.equal(state.owner.dialog,dialog);
  assert.equal(state.fields.get(dialog.input),'topic');
  assert.equal(dialog.pending.id,'captured-request');
  assert.equal(dialog.busy,false);
  assert.equal(dialog.error,'git_branch_create_unknown');
  assert.deepEqual(state.reported,['git_branch_create_unknown']);
  assert.deepEqual(state.prepared,[{kind:'create',name:'topic',commit:'captured-head'}]);
  assert.deepEqual(state.completed,['captured-request']);
  assert.deepEqual(state.forgotten,[]);
  assert.deepEqual(state.released,[]);
  await state.owner.submit();
  assert.equal(state.owner.dialog,null);
  assert.equal(state.prepared.length,1);
  assert.deepEqual(state.completed,['captured-request','captured-request']);
  assert.deepEqual(state.forgotten,['captured-request']);
  assert.deepEqual(state.released,[dialog.input]);
  assert.equal(state.refreshes(),1);
});

test('branch submissions retain the selected starting commit',async()=>{
  const state=fixture([{Ok:{kind:'git_branch_created',data:{}}}]);
  state.owner.form('branch',{name:'release',commit:'selected-commit'});
  state.fields.set(state.owner.dialog.input,'topic');
  await state.owner.submit();
  assert.deepEqual(state.prepared,[{kind:'create',name:'topic',commit:'selected-commit'}]);
});

test('confirmed initialization uses the repository owner instead of a dialog draft',async()=>{
  const state=fixture(),initialized=[];
  state.owner.view.repo.initialize=async()=>{initialized.push(true);return true;};
  state.owner.confirm({kind:'confirm',title:'git_initialize',target:'api\nweb',operation:'initialize'});
  await state.owner.submit();
  assert.deepEqual(initialized,[true]);
  assert.equal(state.owner.dialog,null);
  assert.deepEqual(state.prepared,[]);
});

test('closing failed forms releases capacity for later operations',async()=>{
  const state=fixture(Array.from({length:17},()=>({Err:{code:'conflict'}})));
  for(let index=0;index<17;index++) {
    state.owner.form('branch');state.fields.set(state.owner.dialog.input,'existing');
    await state.owner.submit();state.owner.close();
    assert.equal(state.drafts.size,0);
  }
  assert.equal(state.prepared.length,17);assert.equal(state.forgotten.length,17);
});

test('cancelled forms release late completions without changing replacements',async()=>{
  for(const receipt of [{Ok:{kind:'git_branch_created',data:{}}},{Err:{code:'conflict'}},{Err:{code:'outcome_unknown'}},new Error('disconnected')]) {
    let resolve,reject;
    const pending=new Promise((accept,fail)=>{resolve=accept;reject=fail;});
    const state=fixture([()=>pending]);state.owner.form('branch');
    state.fields.set(state.owner.dialog.input,'topic');const task=state.owner.submit();
    state.owner.close();state.owner.form('branch');const replacement=state.owner.dialog;
    assert.equal(state.drafts.size,1);assert.deepEqual(state.forgotten,[]);
    if(receipt instanceof Error)reject(receipt);else resolve(receipt);
    await task;
    assert.equal(state.owner.dialog,replacement);assert.equal(state.refreshes(),0);
    assert.equal(state.drafts.size,0);assert.deepEqual(state.forgotten,['captured-request']);
  }
});

test('uncertain forms retain their request until retry or abandonment',async()=>{
  for(const receipt of [{},{Err:{code:'outcome_unknown'}},new Error('disconnected')]) {
    const state=fixture([receipt,{Ok:{kind:'git_branch_created',data:{}}}]);
    state.owner.form('branch');const dialog=state.owner.dialog;state.fields.set(dialog.input,'topic');
    await state.owner.submit();assert.equal(dialog.pending.uncertain,true);
    state.fields.set(dialog.input,'different');await state.owner.submit();
    assert.equal(state.prepared.length,1);assert.deepEqual(state.completed,['captured-request']);
    assert.deepEqual(state.forgotten,[]);assert.equal(state.drafts.size,1);
    state.fields.set(dialog.input,'topic');await state.owner.submit();
    assert.deepEqual(state.completed,['captured-request','captured-request']);assert.equal(state.drafts.size,0);
  }
  const state=fixture([{Err:{code:'outcome_unknown'}}]);state.owner.form('branch');
  state.fields.set(state.owner.dialog.input,'topic');await state.owner.submit();state.owner.close();
  assert.equal(state.drafts.size,0);assert.deepEqual(state.forgotten,['captured-request']);
});

test('branch switches ask for native confirmation first',async()=>{
  for(const confirmed of [true,false]) {
    const state=fixture([],confirmed),switched=[];
    state.owner.view.text={git_switch_confirm:'Switch to %{branch}',git_switch_confirm_detail:'Detail',git_switch_confirm_action:'Switch'};
    state.owner.view.repo.branch=async(operation,target)=>switched.push({operation,target});
    await state.owner.branch('switch',{name:'dev',commit:'b',current:false,remote:false});
    assert.deepEqual(JSON.parse(JSON.stringify(state.prompts)),[{title:'Switch to dev',detail:'Detail',action:'Switch'}]);
    assert.equal(switched.length,confirmed ? 1 : 0);
    await state.owner.branch('switch',{name:'main',current:true,remote:false});
    assert.equal(state.prompts.length,1);
  }
});

test('the composer start form opens without repository status',()=>{
  const state=fixture();
  state.owner.view.repo.status=null;
  assert.doesNotThrow(()=>state.owner.form('start'));
  assert.equal(state.owner.dialog.form,'start');
  assert.equal(state.owner.dialog.revisions,null);
});

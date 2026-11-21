import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {host} from './host.mjs';

async function fixture(options={}) {
  let reads=0;
  const status={kind:'ready',entries:[{diff:{additions:3,deletions:1}},{diff:{additions:2,deletions:4}}],
    truncated:false,omitted_paths:0,...options.status};
  const api=await host({context:()=>({worktree:'worktree',...options.scope}),inspectGit:async()=>{
    reads++;
    if (options.error) throw options.error;
    return status;
  }});
  return {api,reads:()=>reads};
}

test('repository counts include localized details',async()=>{
  const {api,reads}=await fixture();
  const [state]=await api.badge();
  assert.equal(reads(),1);
  assert.equal(state.id,'changes');assert.equal(state.visible,true);assert.equal(state.enabled,true);
  assert.deepEqual(state.segments,[{label:{label:'+5'},tone:'success'},{label:{label:'-5'},tone:'danger'}]);
  assert.deepEqual(state.details.map(detail=>[detail.label.label,detail.value]),[['Added','5'],['Removed','5'],['Files','2']]);
  assert.deepEqual(state.details.map(detail=>detail.label.locales['zh-CN']),['新增','删除','文件']);
  for (const key of ['value','reply_to','scope','choices']) assert.equal(Object.hasOwn(state,key),false);
});

test('clean repositories retain zero counts',async()=>{
  const {api}=await fixture({status:{entries:[]}});
  const [state]=await api.badge();
  assert.equal(state.visible,true);
  assert.deepEqual(state.segments.map(segment=>segment.label.label),['+0','-0']);
  assert.equal(state.details[2].value,'0');
});

test('directories and missing scope stay hidden',async()=>{
  const directory=await fixture({status:{kind:'directory'}});
  assert.deepEqual(await directory.api.badge(),[{id:'changes',visible:false}]);
  const unscoped=await fixture({scope:{worktree:null}});
  assert.deepEqual(await unscoped.api.badge(),[{id:'changes',visible:false}]);
  assert.equal(unscoped.reads(),0);
});

test('bounded scans include a partial detail',async()=>{
  for (const status of [{truncated:true},{omitted_paths:1},{entries:[{diff:{additions:1,deletions:0,truncated:true}}]}]) {
    const {api}=await fixture({status});
    const [state]=await api.badge();
    const partial=state.details.at(-1);
    assert.equal(partial.label.label,'Partial results');
    assert.equal(partial.label.locales['zh-CN'],'仅显示部分结果');
    assert.equal(partial.value,'');
  }
});

test('invalidated scopes hide the badge',async()=>{
  for (const code of ['revision_conflict','not_found','permission_denied','wrong_target']) {
    const error=Object.assign(new Error('Repository unavailable'),{code});
    const {api}=await fixture({error});
    assert.deepEqual(await api.badge(),[{id:'changes',visible:false}]);
  }
});

test('transient failures preserve the host error',async()=>{
  const error=Object.assign(new Error('Connection lost'),{code:'unavailable'});
  const {api}=await fixture({error});
  await assert.rejects(api.badge(),failure=>failure===error);
});

test('manifest declares Node status ownership',async()=>{
  const manifest=JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  const extension=manifest.extensions['dev.sailry.platform'];
  assert.equal(manifest.version,'0.1.36');
  assert.equal(extension.min_sailry_version,'0.1.6');
  assert.deepEqual(extension.host.read_only,['badge']);
  assert.deepEqual(extension.host.status,{read:'badge',changes:['repository']});
  assert.ok(extension.host.handlers.includes('badge'));
  assert.equal(extension.ui.find(entry=>entry.id==='changes').handler,'changes');
  assert.equal(extension.tools.find(tool=>tool.name==='list_git_changes').handler.name,'changes');
});

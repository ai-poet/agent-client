import test from 'node:test';
import assert from 'node:assert/strict';
import {Repository} from '../dev.sailry.platform/desktop/repository.js';
import {rows,checkbox,selectedId} from '../dev.sailry.platform/desktop/changes.js';

const deferred = () => { let resolve,reject; const promise=new Promise((done,fail) => {resolve=done;reject=fail;}); return {promise,resolve,reject}; };
const settle = async () => { for (let index=0;index<12;index++) await Promise.resolve(); };
const status = () => ({kind:'ready',head:'original',branch:'main',index_revision:'index-1',entries:[{path:'a.txt',staged:'modified',unstaged:null,untracked:false,conflicted:false}]});
function fixture(extra = {}, pause = async () => {}) {
  const prepared=[],completed=[],forgotten=[],reported=[];
  const api={inspectGit:async()=>status(),listGitBranches:async()=>({sync:{head_message:'Previous message'}}),
    prepareGitCommit:draft=>{prepared.push(draft);return 'stable-request';},
    prepareGitIndex:draft=>{prepared.push(draft);return 'stable-request';},
    prepareGitChange:draft=>{prepared.push(draft);return 'stable-request';},
    completeRequest:async id=>{completed.push(id);return {Ok:{kind:'git_index',data:{revision:'index-2'}}};},
    forgetRequest:id=>forgotten.push(id),faultCode:()=> 'outcome_unknown',...extra};
  const owner=new Repository(api,()=>{},key=>reported.push(key),pause);owner.status=status();return {owner,prepared,completed,forgotten,reported};
}

test('branch reads wait for repository applicability',async()=>{
  for (const directory of [false,true]) {
    const inspected=deferred(),branches=deferred();let reads=0;
    const {owner,reported}=fixture({inspectGit:()=>inspected.promise,listGitBranches:()=>{reads++;return branches.promise;}});
    const refresh=owner.refresh();assert.equal(reads,0);
    inspected.resolve(directory ? {...status(),kind:'directory'} : status());
    await settle();
    assert.equal(reads,directory ? 0 : 1);
    if (!directory) branches.reject(Error('branch read failed'));
    await refresh;
    assert.equal(owner.error,directory ? null : 'git_branches_failed');
    assert.deepEqual(reported,directory ? [] : ['git_branches_failed']);
  }
});

test('diff reads overlap while the display budget preserves path order',async()=>{
  const pending=new Map(),calls=[];
  const {owner}=fixture({readGitDiff:path=>{calls.push(path);return new Promise(resolve=>pending.set(path,resolve));}});
  owner.status={...status(),entries:['a','b','c'].map(path=>({path}))};
  const opened=owner.open({kind:'changes'});
  assert.deepEqual(calls,['a','b','c']);
  pending.get('c')({path:'c',text:'last'});
  pending.get('b')({path:'b',text:'b'.repeat(512*1024)});
  pending.get('a')({path:'a',text:'a'.repeat(512*1024)});
  await opened;
  const document=owner.documents.get('changes:');
  assert.deepEqual(document.files.map(file=>file.path),['a','b']);
  assert.equal(document.truncated,true);
});

test('document and history refresh do not wait for each other',async()=>{
  const diff=deferred(),history=deferred(),calls=[];
  const {owner}=fixture({readGitDiff:()=>{calls.push('diff');return diff.promise;},
    readGitLog:()=>{calls.push('history');return history.promise;}});
  owner.documents.set('a.txt',{request:{kind:'file',path:'a.txt'}});owner.tabs=['a.txt'];owner.selected='a.txt';owner.view='history';
  const refreshed=owner.refresh();await settle();
  assert.deepEqual(calls,['diff','history']);
  history.resolve({entries:[]});diff.resolve({path:'a.txt',text:'done'});
  assert.equal(await refreshed,true);
});

test('starts with a tree and preserves explicit navigation choices',async()=>{
  const {owner}=fixture();
  assert.deepEqual(owner.navigation,{hierarchical:true,grouping:'tracked',sort:'path'});
  const nested={...status(),entries:[{...status().entries[0],path:'src/main.rs'}]};
  const text={git_tracked:'Tracked',git_untracked:'Untracked'};
  assert.equal(rows(nested,owner.navigation,text)[0].children[0].id,'dir/tracked/src');
  owner.navigation.hierarchical=false;
  await owner.refresh();
  assert.equal(owner.navigation.hierarchical,false);
  assert.equal(rows(nested,owner.navigation,text)[0].children[0].id,'tracked/src/main.rs');
});

test('initialization retains its request until recovery and then enables normal writes',async()=>{
  let attempt=0,initialized=false;
  const {owner,prepared,completed,forgotten}=fixture({
    inspectGit:async()=>initialized ? {...status(),kind:'unborn',head:null} : {kind:'directory',head:null,branch:null,index_revision:null,entries:[]},
    completeRequest:async id=>{completed.push(id); if (++attempt===1) return {Err:{code:'outcome_unknown'}}; initialized=true;return {Ok:{kind:'git_action_completed'}};},
  });
  await owner.refresh();
  assert.equal(owner.writable(),false); assert.equal(owner.canInitialize(),true);
  assert.equal(await owner.initialize(),false);
  assert.deepEqual(prepared,[{kind:'action',action:'initialize',expected_index:'',expected_head:null,expected_branch:null}]);
  assert.equal(owner.canInitialize(),false);
  assert.equal(await owner.initialize(),false);
  assert.equal(await owner.complete(),true);
  assert.deepEqual(completed,['stable-request','stable-request']);
  assert.deepEqual(forgotten,['stable-request']);
  assert.equal(owner.status.kind,'unborn'); assert.equal(owner.writable(),true);
  assert.equal(owner.canInitialize(),false);
});

test('commit preserves newer input and freezes the submitted revisions',async()=>{
  const receipt=deferred();const {owner,prepared,forgotten,reported}=fixture({completeRequest:()=>receipt.promise});
  owner.message='Submitted';const pending=owner.commit();owner.message='New draft';
  assert.equal(prepared[0].message,'Submitted');assert.equal(prepared[0].expected_index,'index-1');
  assert.equal(owner.writable(),false);
  receipt.resolve({Ok:{kind:'git_commit_created',data:{id:'commit',follow_up:{code:'unavailable',message:'Push failed'}}}});
  assert.equal(await pending,true);assert.equal(owner.message,'New draft');
  assert.equal(owner.followUp.message,'Push failed');assert.deepEqual(forgotten,['stable-request']);assert.equal(owner.pending,null);
  assert.deepEqual(reported,['git_commit_sync_failed']);
});

test('accepted read failures use the ordinary notification channel',async()=>{
  for (const [method,read,key] of [
    ['inspectGit',owner=>owner.refresh(),'git_read_failed'],
    ['listGitBranches',owner=>owner.refresh(),'git_branches_failed'],
    ['readGitLog',owner=>owner.loadHistory(),'git_read_failed'],
    ['readGitDiff',owner=>owner.open({kind:'file',path:'a.txt'}),'git_read_failed'],
  ]) {
    const {owner,reported}=fixture({[method]:async()=>{throw Error('read failed');}});
    await read(owner);assert.equal(owner.error,key);assert.deepEqual(reported,[key]);
  }
});

test('status refresh exposes its typed failure code',async()=>{
  for (const code of ['unavailable','not_found','permission_denied','wrong_target']) {
    const {owner}=fixture({inspectGit:async()=>{throw Error('typed failure');},faultCode:()=>code});
    assert.equal(await owner.refresh(),false);
    assert.equal(owner.readCode,code);
  }
  const {owner}=fixture();await owner.refresh();
  assert.equal(owner.readCode,null);
});

test('stale and released reads cannot report failures',async()=>{
  for (const [method,read] of [
    ['inspectGit',owner=>owner.refresh()],
    ['listGitBranches',owner=>owner.refresh()],
    ['readGitLog',owner=>owner.loadHistory()],
    ['readGitDiff',owner=>owner.open({kind:'file',path:'a.txt'})],
  ]) {
    const pending=deferred();const {owner,reported}=fixture({[method]:()=>pending.promise});
    const readResult=read(owner);await settle();owner.stop();pending.reject(Error('late failure'));
    await readResult;assert.deepEqual(reported,[]);
  }
  const first=deferred();let count=0;
  const {owner,reported}=fixture({readGitDiff:async path=>++count===1 ? first.promise : {path,text:'accepted'}});
  const stale=owner.open({kind:'file',path:'a.txt'});await owner.open({kind:'file',path:'b.txt'});
  first.reject(Error('stale failure'));await stale;
  assert.equal(owner.selected,'b.txt');assert.deepEqual(reported,[]);
});

test('overlapping observations share only their active read',async()=>{
  const pending=deferred();let reads=0;
  const {owner,forgotten}=fixture({inspectGit:()=>{reads++;return reads===1 ? pending.promise : Promise.resolve(status());}});
  owner.pending={id:'unknown',kind:'index',busy:false,reviewing:false};
  const first=owner.refresh(),second=owner.refresh(true);
  assert.equal(first,second);assert.equal(reads,1);
  pending.resolve(status());assert.deepEqual(await Promise.all([first,second]),[true,true]);
  assert.equal(owner.pending,null);assert.deepEqual(forgotten,['unknown']);
  await owner.refresh();assert.equal(reads,2,'completed reads are not cached');
});

test('invalidation follows shared reads with one fresh observation',async()=>{
  for(const failed of [false,true]) {
    const first=deferred(),latest=deferred();let reads=0;
    const {owner,reported}=fixture({inspectGit:()=>++reads===1 ? first.promise : latest.promise});
    const observation=owner.refresh(),changed=owner.invalidate(),again=owner.invalidate();
    assert.equal(observation,changed);assert.equal(changed,again);assert.equal(reads,1);
    if(failed)first.reject(Error('stale failure'));else first.resolve({...status(),branch:'stale'});
    await settle();assert.equal(reads,2);
    assert.equal(owner.status.branch,failed ? 'main' : 'stale','completed data stays available during the follow-up');
    latest.resolve({...status(),branch:'fresh'});
    assert.deepEqual(await Promise.all([observation,changed,again]),[true,true,true]);
    assert.equal(owner.status.branch,'fresh');assert.equal(reads,2);assert.deepEqual(reported,[]);
  }
});

test('continuous changes share one pending slot and retain completed data',async()=>{
  const replies=[deferred(),deferred(),deferred()],pauses=[];let reads=0;
  const {owner}=fixture({inspectGit:()=>replies[reads++].promise},async delay=>pauses.push(delay));
  owner.message='Retained draft';
  const initial=owner.invalidate();
  for(let count=0;count<20;count++)assert.equal(owner.invalidate(),initial);
  replies[0].resolve({...status(),branch:'first'});await settle();
  assert.equal(reads,2);assert.equal(owner.status.branch,'first');
  for(let count=0;count<20;count++)assert.equal(owner.invalidate(),initial);
  replies[1].resolve({...status(),branch:'second'});await settle();
  assert.equal(reads,3);assert.equal(owner.status.branch,'second');
  replies[2].resolve({...status(),branch:'latest'});
  assert.equal(await initial,true);assert.equal(owner.status.branch,'latest');
  assert.equal(owner.message,'Retained draft');assert.equal(reads,3);
  assert.equal(pauses.length,2);assert.equal(pauses.every(delay=>delay>0&&delay<=750),true);
});

test('hidden reads retain one catch-up without background feedback',async()=>{
  let reads=0;const {owner,reported}=fixture({inspectGit:async()=>{reads++;throw Error('unavailable');},faultCode:()=> 'unavailable'});
  const original=owner.status;owner.visible=false;owner.message='Retained draft';
  for(let count=0;count<20;count++)await owner.invalidate();
  assert.equal(reads,0);assert.equal(owner.status,original);
  owner.visible=true;assert.equal(await owner.invalidate(),false);
  assert.equal(reads,1);assert.equal(owner.status,original);assert.equal(owner.message,'Retained draft');
  assert.deepEqual(reported,[]);
  await owner.refresh();assert.deepEqual(reported,['git_read_failed']);
  owner.status=null;await owner.invalidate();assert.equal(reported.length,2);
});

test('visibility changes stop subsequent repository reads',async()=>{
  for(const field of ['visible','connected']) {
    const inspected=deferred();let branches=0,reads=0;
    const {owner}=fixture({
      inspectGit:()=>++reads===1 ? inspected.promise : Promise.resolve(status()),
      listGitBranches:async()=>{branches++;return {};},
    });
    const initial=owner.invalidate();owner[field]=false;
    inspected.resolve({...status(),branch:'retained'});
    assert.equal(await initial,false);assert.equal(branches,0);
    assert.equal(owner.status.branch,'retained');assert.equal(owner.dirty,true);
    owner[field]=true;
    assert.equal(await owner.invalidate(),true);
    assert.equal(reads,2);assert.equal(branches,1);
  }
});

test('disconnect during refresh delay prevents a new query',async()=>{
  const waiting=deferred();let reads=0;
  const {owner}=fixture({inspectGit:async()=>{reads++;return status();}},()=>waiting.promise);
  owner.lastRead=Date.now();
  const refresh=owner.invalidate();owner.connected=false;
  waiting.resolve();
  assert.equal(await refresh,false);assert.equal(reads,0);assert.equal(owner.dirty,true);
});

test('document invalidations share the active repository read',async()=>{
  const document=deferred();let reads=0,diffs=0;
  const {owner}=fixture({inspectGit:async()=>{reads++;return status();},
    readGitDiff:()=>++diffs===1 ? document.promise : Promise.resolve({path:'a.txt',text:'latest'})});
  owner.selected='a.txt';owner.tabs=['a.txt'];owner.documents.set('a.txt',{request:{kind:'file',path:'a.txt'}});
  const initial=owner.invalidate();await settle();
  assert.equal(reads,1);assert.equal(diffs,1);
  for(let count=0;count<20;count++)assert.equal(owner.invalidate(),initial);
  assert.equal(reads,1);
  document.resolve({path:'a.txt',text:'accepted'});
  assert.equal(await initial,true);assert.equal(reads,2);assert.equal(diffs,2);
  assert.equal(owner.documents.get('a.txt').files[0].text,'latest');
});

test('superseded refreshes follow the latest result',async()=>{
  for (const latestFirst of [false,true]) {
    const first=deferred(),second=deferred();let reads=0;
    const {owner}=fixture({inspectGit:()=>++reads===1 ? first.promise : second.promise});
    const older=owner.refresh();++owner.read;const latest=owner.refresh();let finished=false;
    older.then(()=>{finished=true;});
    if (latestFirst) { second.resolve({...status(),branch:'latest'});await latest; }
    first.resolve({...status(),branch:'stale'});await settle();
    if (!latestFirst) { assert.equal(finished,false);second.resolve({...status(),branch:'latest'}); }
    assert.deepEqual(await Promise.all([older,latest]),[true,true]);
    assert.equal(owner.status.branch,'latest');assert.equal(reads,2);
  }
});

test('stale inspect and branch failures follow the accepted refresh',async()=>{
  for (const method of ['inspectGit','listGitBranches']) {
    const first=deferred(),second=deferred();let reads=0;
    const {owner,reported}=fixture({[method]:()=>++reads===1 ? first.promise : second.promise});
    const older=owner.refresh();await settle();++owner.read;const latest=owner.refresh();await settle();
    first.reject(Error('stale failure'));await settle();
    second.resolve(method === 'inspectGit' ? status() : {sync:{head_message:'Latest'}});
    assert.deepEqual(await Promise.all([older,latest]),[true,true]);assert.deepEqual(reported,[]);
  }
});

test('latest failure and release reach every refresh waiter',async()=>{
  for (const released of [false,true]) {
    const first=deferred(),second=deferred();let reads=0;
    const {owner,reported}=fixture({inspectGit:()=>++reads===1 ? first.promise : second.promise});
    const older=owner.refresh();++owner.read;const latest=owner.refresh();
    first.resolve(status());await settle();
    if (released) {owner.stop();second.resolve(status());}
    else second.reject(Error('latest failure'));
    assert.deepEqual(await Promise.all([older,latest]),[false,false]);
    assert.deepEqual(reported,released ? [] : ['git_read_failed']);
  }
});

test('active review joins observation without affecting later refreshes',async()=>{
  for (const reviewFirst of [false,true]) {
    const first=deferred();let reads=0;
    const {owner,forgotten}=fixture({inspectGit:()=>++reads===1 ? first.promise : Promise.resolve(status())});
    owner.pending={id:'unknown',kind:'index',busy:false,reviewing:false};
    const older=owner.refresh(reviewFirst),latest=owner.refresh(!reviewFirst);
    assert.equal(older,latest);assert.equal(owner.pending.id,'unknown');
    first.resolve(status());assert.deepEqual(await Promise.all([older,latest]),[true,true]);
    assert.equal(owner.pending,null);assert.deepEqual(forgotten,['unknown']);
    owner.pending={id:'later',kind:'index',busy:false,reviewing:false};
    await owner.refresh();assert.equal(owner.pending.id,'later');assert.deepEqual(forgotten,['unknown']);
  }
});

test('explicit review is not delayed behind document rendering',async()=>{
  const document=deferred();let reads=0;
  const {owner,forgotten}=fixture({inspectGit:async()=>{reads++;return status();},readGitDiff:()=>document.promise});
  owner.selected='notes';owner.tabs=['notes'];owner.documents.set('notes',{request:{kind:'file',path:'notes',scope:'all'}});
  owner.pending={id:'unknown',kind:'index',busy:false,reviewing:false};
  const observation=owner.refresh();await settle();
  assert.equal(owner.pending.id,'unknown');assert.equal(owner.loading,false);
  owner.selected=null;const review=owner.refresh(true);await review;
  assert.equal(reads,2);assert.equal(owner.pending,null);assert.deepEqual(forgotten,['unknown']);
  document.resolve({path:'notes',text:'accepted'});assert.equal(await observation,true);
});

test('an active document review does not unlock a later uncertain write',async()=>{
  const document=deferred();
  const {owner,forgotten}=fixture({readGitDiff:()=>document.promise,completeRequest:async()=>({Err:{code:'outcome_unknown'}})});
  owner.selected='notes';owner.documents.set('notes',{request:{kind:'file',path:'notes',scope:'all'}});
  owner.pending={id:'earlier',kind:'index',busy:false,reviewing:false};
  const review=owner.refresh(true);await settle();
  assert.equal(owner.pending,null);assert.equal(owner.writable(),true);
  assert.equal(await owner.index(['a.txt'],'stage'),false);assert.equal(owner.pending.id,'stable-request');
  owner.selected=null;await owner.refresh();
  assert.equal(owner.pending.id,'stable-request');assert.deepEqual(forgotten,['earlier']);
  document.resolve({path:'notes',text:'late'});assert.equal(await review,true);
});

test('mutation invalidation does not follow its own refresh promise',async()=>{
  const document=deferred(),receipt=deferred();
  const {owner,reported}=fixture({readGitDiff:()=>document.promise,completeRequest:()=>receipt.promise});
  owner.selected='notes';owner.documents.set('notes',{request:{kind:'file',path:'notes',scope:'all'}});
  const refresh=owner.refresh();await settle();assert.equal(owner.writable(),true);
  const write=owner.index(['a.txt'],'stage');
  document.resolve({path:'notes',text:'late'});assert.equal(await refresh,false);
  receipt.resolve({Err:{code:'outcome_unknown'}});assert.equal(await write,false);
  assert.equal(owner.pending.id,'stable-request');assert.deepEqual(reported,['git_index_unknown']);
});

test('binary and bounded diff states do not produce error notifications',async()=>{
  for (const result of [{binary:true},{truncated:true}]) {
    const {owner,reported}=fixture({readGitDiff:async path=>({path,text:'',...result})});
    await owner.open({kind:'file',path:'a.txt'});
    assert.equal(owner.error,result.binary ? 'git_diff_binary' : 'git_diff_partial');assert.deepEqual(reported,[]);
  }
});

test('unknown writes retain one request until explicit recovery',async()=>{
  const {owner,prepared,completed}=fixture();let attempt=0;
  owner.api.completeRequest=async id=>{completed.push(id);if (++attempt===1) throw new Error('connection lost');return {Ok:{kind:'git_index',data:{revision:'index-2'}}};};
  assert.equal(await owner.index(['a.txt'],'stage'),false);
  assert.equal(owner.pending.id,'stable-request');assert.equal(owner.writable(),false);
  assert.equal(await owner.index(['a.txt'],'stage'),false);assert.equal(prepared.length,1);
  assert.equal(await owner.complete(),true);assert.deepEqual(completed,['stable-request','stable-request']);
  assert.equal(prepared.length,1);assert.equal(owner.pending,null);
});

test('latest read intent opens a tab only when its result is accepted',async()=>{
  const first=deferred(),second=deferred();const {owner}=fixture({readGitDiff:path=>(path==='a'?first:second).promise});
  const a=owner.open({kind:'file',path:'a'}),b=owner.open({kind:'file',path:'b'});
  second.resolve({path:'b',text:'b'});await b;first.resolve({path:'a',text:'a'});await a;
  assert.deepEqual(owner.tabs,['b']);assert.equal(owner.selected,'b');assert.equal(owner.documents.has('a'),false);
});

test('history replaces native pages using the original immutable head',async()=>{
  const reads=[],head='a'.repeat(40),second={head,after:'b'.repeat(40),offset:100},third={head,after:'c'.repeat(40),offset:200};
  let liveHead=head;
  const {owner}=fixture({readGitLog:async(limit,cursor,options)=>{
    reads.push({limit,cursor,options});
    const offset=cursor?.offset ?? 0;
    return {head:cursor?.head ?? options.revision ?? liveHead,offset,
      entries:[{id:String(offset)}],next:offset === 0 ? second : offset === 100 ? third : null};
  }});
  await owner.loadHistory();liveHead='d'.repeat(40);
  for(const page of [2,3,1,2,3]) await owner.historyPage(page);
  assert.deepEqual(reads,[
    {limit:100,cursor:null,options:{revision:null}},
    ...[second,third,null,second,third].map(cursor=>({limit:100,cursor,options:{revision:head}})),
  ]);
  assert.deepEqual(owner.history.entries,[{id:'200'}]);
  await owner.historyPage(4);assert.equal(reads.length,6);
  await owner.loadHistory();assert.equal(owner.history.head,liveHead);
  await owner.historyPage(3);assert.equal(reads.length,7);
});

test('history paging accepts a newer choice while retaining the committed page',async()=>{
  const head='a'.repeat(40),cursor={head,after:'b'.repeat(40),offset:100},replies=[];
  const {owner}=fixture({readGitLog:async()=>({head,offset:0,entries:[{id:'current'}],next:cursor})});
  await owner.loadHistory();
  owner.api.readGitLog=async(_limit,cursor)=>new Promise(resolve=>replies.push({cursor,resolve}));
  const next=owner.historyPage(2);
  assert.deepEqual(owner.history.entries,[{id:'current'}]);
  const current=owner.historyPage(1);
  replies[1].resolve({head,offset:0,entries:[{id:'latest'}],next:cursor});await current;
  replies[0].resolve({head,offset:100,entries:[{id:'stale'}],next:{head,after:'c'.repeat(40),offset:200}});await next;
  assert.deepEqual(owner.history.entries,[{id:'latest'}]);assert.equal(owner.historyLoading,false);
  await owner.historyPage(3);assert.equal(replies.length,2);
});

test('history failures retain the accepted page and retry cursor',async()=>{
  const head='a'.repeat(40),cursor={head,after:'b'.repeat(40),offset:100},calls=[];
  const {owner,reported}=fixture({readGitLog:async()=>({head,offset:0,entries:[{id:'current'}],next:cursor})});
  await owner.loadHistory();
  owner.api.readGitLog=async(_limit,selected)=>{
    calls.push(selected);
    if(calls.length === 1)throw Error('unavailable');
    return {head,offset:100,entries:[{id:'continued'}],next:null};
  };
  await owner.historyPage(2);
  assert.deepEqual(owner.history.entries,[{id:'current'}]);
  assert.deepEqual(reported,['git_read_failed']);
  await owner.historyPage(2);
  assert.deepEqual(calls,[cursor,cursor]);
  assert.deepEqual(owner.history.entries,[{id:'continued'}]);
});

test('branch creation does not acquire unrelated checkout revisions',async()=>{
  const {owner,prepared}=fixture();await owner.branch('create',{name:'topic',commit:'original'});
  assert.deepEqual(prepared[0],{kind:'create',name:'topic',commit:'original'});
});

test('change groups retain staged additions and mixed index selection',()=>{
  const modified={path:'src/a',staged:'modified',unstaged:'modified',untracked:false,conflicted:false};
  const added={path:'new',staged:'added',unstaged:null,untracked:false,conflicted:false};
  const tree=rows({entries:[modified,added]},{grouping:'tracked',hierarchical:false,sort:'path'},{git_tracked:'Tracked',git_untracked:'New'});
  assert.equal(tree[0].children[0].id,'tracked/src/a');assert.equal(tree[1].children[0].id,'untracked/new');
  assert.deepEqual(checkbox([modified,added]),{checked:false,mixed:true,operation:'stage',paths:['src/a']});
  assert.equal(selectedId(added,'tracked','all'),'untracked/new');
});

test('tracking conflicts retain the original action for explicit stash-and-switch',async()=>{
  const {owner,prepared,forgotten}=fixture({completeRequest:async()=>({Err:{code:'conflict'}})});
  const action={switch_tracking:{remote:'origin/topic',local:'topic',stash:false}};
  assert.equal(await owner.action(action),false);
  assert.equal(owner.pending.kind,'switch');assert.equal(owner.pending.error,'git_branch_conflict');
  assert.deepEqual(owner.pending.action,action);assert.deepEqual(forgotten,[]);
  assert.equal(await owner.index(['a.txt'],'stage'),false);
  assert.equal(prepared.length,1);
});

test('known staging conflicts refresh without replay and preserve visible diff',async()=>{
  const {owner,completed}=fixture({completeRequest:async()=>({Err:{code:'revision_conflict'}}),readGitDiff:async path=>({path,text:'updated',additions:1,deletions:0})});
  await owner.open({kind:'file',path:'a.txt',scope:'staged'});
  assert.equal(await owner.index(['a.txt'],'unstage'),false);
  assert.equal(owner.pending,null);assert.equal(owner.selected,'a.txt');
  assert.equal(owner.documents.get('a.txt').text,undefined);
  assert.equal(owner.documents.get('a.txt').files[0].text,'updated');assert.deepEqual(completed,[]);
});

test('explicit review unlocks an unknown result without replaying it',async()=>{
  let attempts=0;const {owner,forgotten}=fixture({completeRequest:async()=>{attempts++;throw Error('lost response');}});
  await owner.index(['a.txt'],'stage');await owner.refresh();
  assert.equal(owner.pending.id,'stable-request');assert.equal(owner.writable(),false);
  await owner.refresh(true);assert.equal(owner.pending,null);assert.equal(attempts,1);assert.deepEqual(forgotten,['stable-request']);
});

test('closing or replacing a controller rejects a late completion',async()=>{
  const receipt=deferred();const {owner,forgotten}=fixture({completeRequest:()=>receipt.promise});
  owner.message='captured';const pending=owner.commit();owner.stop();
  receipt.resolve({Ok:{kind:'git_commit_created',data:{}}});
  assert.equal(await pending,false);assert.equal(owner.message,'captured');assert.deepEqual(forgotten,[]);
});

test('amend toggles restore the draft and commit scope remains explicit',()=>{
  const {owner}=fixture();owner.branches={sync:{head_message:'Previous'}};owner.message='New draft';
  owner.toggleAmend();assert.equal(owner.message,'Previous');owner.toggleAmend();assert.equal(owner.message,'New draft');
  owner.status.entries=[{path:'tracked',staged:null,unstaged:'modified',untracked:false,conflicted:false},{path:'new',staged:null,unstaged:null,untracked:true,conflicted:false}];
  assert.deepEqual(owner.commitChoices(),['tracked','all']);assert.equal(owner.commitChoices('staged'),null);
  owner.status.entries.shift();assert.deepEqual(owner.commitChoices(),['all']);
});


test('branches follow references and explicit refreshes',async()=>{
  let reads=0;
  const {owner}=fixture({listGitBranches:async()=>{reads++;return {sync:{head_message:'Previous'}};}});
  owner.references='1';await owner.invalidate();assert.equal(reads,1);
  await owner.invalidate();await owner.invalidate();assert.equal(reads,1);
  owner.references='2';await owner.invalidate();assert.equal(reads,2);
  owner.status.head='older';await owner.invalidate();assert.equal(reads,3);
  await owner.refresh(true);assert.equal(reads,4);
});

test('reference changes during reads retain invalidation',async()=>{
  const pending=deferred();let reads=0;
  const {owner}=fixture({listGitBranches:async()=>{reads++;return reads===1 ? pending.promise : {sync:{}};}});
  owner.references='1';const first=owner.invalidate();await settle();
  owner.references='2';owner.invalidate();
  pending.resolve({sync:{}});await first;assert.equal(reads,2);
  await owner.invalidate();assert.equal(reads,2);
});

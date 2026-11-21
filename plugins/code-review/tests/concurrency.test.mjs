import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFile} from 'node:fs/promises';
import {file,fixture,receipt,uuid} from './support.mjs';

const source=async name=>(await readFile(new URL(`../dev.sailry.platform/host/${name}.js`,import.meta.url),'utf8'))
  .replace(/^import .*;\n/gm,'').replace(/^export /gm,'');
const settle=async()=>{for(let step=0;step<32;step++)await Promise.resolve();};

test('manifest starts every frozen scope page together',async()=>{
  const held=[],id=uuid(50);
  const manifest=vm.runInNewContext(`${await source('records')}\nmanifest`,{
    identifier:value=>value,getValue:key=>new Promise(resolve=>held.push({key,resolve})),
  });
  const loading=manifest({id,pages:[0,1,2],counts:{files:3,groups:0,skipped:0}});
  assert.equal(held.length,3,'all manifest reads start before any record settles');
  for(const entry of [...held].reverse())entry.resolve({key:entry.key,present:true,revision:'1',value:{v:1,kind:'scope',rows:[{kind:'file',value:{path:entry.key}}]}});
  const result=await loading;
  assert.deepEqual(Array.from(result.files,value=>value.path),[0,1,2].map(page=>`review/${id}/scope/${page}`));
});

test('verification starts all frozen content pages together',async()=>{
  const held=[],id=uuid(51),path='src/main.js',revision='frozen';
  const content={diff:{start:0,count:2},old:{start:2,count:2},new:{start:4,count:2}};
  const verify=vm.runInNewContext(`${await source('content')}\nverify`,{
    key:(id,suffix)=>`review/${id}/${suffix}`,record:key=>new Promise(resolve=>held.push({key,resolve})),
  });
  const loading=verify({id},{path,revision,content});
  assert.equal(held.length,6,'every part and chunk starts before any frozen content settles');
  for(const entry of [...held].reverse()){
    const index=Number(entry.key.split('/').at(-1)),part=['diff','old','new'][Math.floor(index/2)],offset=index%2;
    const page={v:1,kind:'content',capture:id,path,part,revision,offset,next:offset===0?1:null,
      ...(part==='diff'?{lines:[{kind:'context',old_line:offset+1,new_line:offset+1,text:'frozen'}]}:{text:'frozen\n'})};
    entry.resolve({key:entry.key,present:true,revision:'1',value:page});
  }
  await loading;
});

test('completion verifies independent files before publishing',async()=>{
  const held=[];
  const current=await fixture([file('src/a.js'),file('src/b.js'),file('src/c.js')]);
  const scope=await current.invoke('scope',{});
  // The production completion owner must enter every file verification immediately.
  const code=(await source('complete')).replace(/^import .*;\n/gm,'');
  const prepare=vm.runInNewContext(`${code}\nprepare`,{
    fields:value=>value,identifier:value=>value,open(){},
    head:async()=>({record:{revision:'1'},value:{creator:current.captured.session,reviewed_groups:[],receipts:[],findings:[]}}),
    manifest:async()=>({files:scope.files,groups:scope.groups}),
    path(){},revision(){},size:()=>1,limits:{page:100,output:128*1024},
    verify:(_value,file)=>new Promise(resolve=>held.push({path:file.path,resolve})),
    shards:()=>({operations:[]}),write:()=>({}),key:()=>'',transaction:(_operations,result)=>result,
  });
  const loading=prepare(receipt(scope),current.captured,uuid(52));await settle();
  assert.deepEqual(held.map(entry=>entry.path),scope.files.map(file=>file.path));
  for(const entry of held)entry.resolve();
  assert.equal((await loading).status,'complete');
});

test('capture reads grow once per content page',async()=>{
  for (const count of [1,5,23]) {
    const current=await fixture(Array.from({length:count},(_,index)=>file(`src/${index}.js`)));
    const scope=await current.capture();
    assert.equal(scope.counts.files,count);
    assert.equal(current.calls.filter(call=>call.kind === 'list_git_changes').length,2);
    assert.equal(current.calls.filter(call=>call.kind === 'read_git_change').length,count);
    assert.equal(current.calls.filter(call=>call.kind === 'read_git_context').length,count*2);
  }
});

test('large final scopes prepare packed results once',async()=>{
  const paths=Array.from({length:128},(_,index)=>`part-${index}/${'漢'.repeat(4000)}.rs`);
  const current=await fixture(paths.map(path=>file(path)));
  const scope=await current.capture();
  assert.equal(scope.counts.files,paths.length);
  const progress=current.values.get(`review/${scope.review_id}/capture`).value.progress;
  assert.ok(progress.final.length>16,'final scope crosses the continuation step budget');
  assert.ok(progress.results.length<paths.length,'completed rows use packed source shards');
  for(const page of progress.results) {
    const key=`review/${scope.review_id}/scope/${page}`;
    assert.equal(current.valueReads.filter(name=>name===key).length,1,'each source shard is prepared once');
  }
  const found=scope.files.map(value=>value.path);
  for(let next=scope.next;next!==null;) {
    const page=await current.invoke('scope',{review_id:scope.review_id,offset:next});
    found.push(...page.files.map(value=>value.path));
    next=page.next;
  }
  assert.deepEqual(new Set(found),new Set(paths));
});

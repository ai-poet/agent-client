import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFile} from 'node:fs/promises';

const plain=value=>JSON.parse(JSON.stringify(value));
const source=async(name,file)=>(await readFile(new URL(`../../${name}/dev.sailry.platform/host/${file}.js`,import.meta.url),'utf8'))
  .replace(/^import .*;\n/gm,'').replace(/^export /gm,'');
const settle=async()=>{for(let index=0;index<16;index++)await Promise.resolve();};

for(const name of ['reminders','scheduled-tasks']) {
  test(`${name} reads page entries concurrently in key order`,async()=>{
    const ids=Array.from({length:6},(_,index)=>`11111111-1111-4111-8111-${String(index+1).padStart(12,'0')}`),held=[];
    const prefix=name==='reminders'?'reminder/':'task/';
    const page=vm.runInNewContext(`${await source(name,'records')}\npage`,{
      context:()=>({execution_node:[1,...Array(31).fill(0)]}),
      listKeys:async()=>({keys:ids.map(id=>prefix+id),after:null}),
      getValue:key=>new Promise(resolve=>held.push({key,resolve})),
    });
    const loading=page();await settle();
    assert.equal(held.length,ids.length,'all records start before any record settles');
    for(const entry of [...held].reverse()){
      const id=entry.key.slice(prefix.length);
      entry.resolve({key:entry.key,present:true,revision:'1',value:{id,project:null,title:id,message:'',completed:false,due_ms:null,notified_ms:null}});
    }
    assert.deepEqual(plain((await loading).items.map(item=>item.id)),ids);
  });

  test(`${name} reads independent schedule metadata concurrently`,async()=>{
    const held=[],file=name==='reminders'?'schedules':'dispatch';
    const records=name==='scheduled-tasks'
      ?vm.runInNewContext(`${await source(name,'records')}\n({execution,requireConfig})`)
      :vm.runInNewContext(`${await source(name,'records')}\n({snapshot})`);
    const changes=vm.runInNewContext(`${await source(name,file)}\nchanges`,{
      context:()=>({package:{name},execution_node:[1,...Array(31).fill(0)]}),...records,
      prepareRequest:command=>command.data.action.kind,
      completeRequest:id=>new Promise(resolve=>held.push({id,resolve})),forgetRequest(){},
    });
    const loading=changes({id:'saved',config:{},completed:false,due_ms:null,notified_ms:null},'2');
    assert.deepEqual(held.map(entry=>entry.id),['list_schedules','list_handlers']);
    for(const entry of held)entry.resolve({Ok:{data:{data:[]}}});
    assert.equal((await loading).length,2);
  });

  test(`${name} reads job results concurrently in page order`,async()=>{
    const held=[],jobs=Array.from({length:6},(_,index)=>({id:`job-${index}`,handler:'saved',status:'completed',event:{payload:{}}}));
    const query=async(kind,data)=>kind==='list_jobs'?{jobs,next_before:null}:
      new Promise(resolve=>held.push({id:data.id,resolve}));
    const runtime={node:value=>value,read:async()=>({present:false}),key:value=>value,result:async action=>({Ok:await action()}),query};
    Object.assign(runtime,{context:()=>({package:{name},execution_node:[1,...Array(31).fill(0)]})});
    if(name==='reminders')Object.assign(runtime,{
      context:()=>({package:{name},execution_node:[1,...Array(31).fill(0)]}),prepareRequest:command=>command,
      completeRequest:async command=>({Ok:{data:{data:await query(command.data.action.kind,command.data.action.data)}}}),forgetRequest(){},
    });
    const history=vm.runInNewContext(`${await source(name,name==='reminders'?'schedules':'management')}\nhistory`,runtime);
    const loading=history({handler:'saved'});await settle();
    assert.equal(held.length,jobs.length,'all results start before any job result settles');
    for(const entry of [...held].reverse())entry.resolve({kind:'completed',data:{Ok:{kind:'queued_turn',data:{session:entry.id,id:'turn'}}}});
    const result=(await loading).Ok;
    assert.deepEqual(plain(result.items.map(item=>name==='reminders'?item.id:item.job.id)),jobs.map(job=>job.id));
  });
}

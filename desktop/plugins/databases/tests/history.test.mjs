import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFile} from 'node:fs/promises';
import {updates,result} from '../dev.sailry.platform/desktop/history.js';
test('distinguishes restored results from new canonical responses and preserves exact cells',()=>{
  const call={key:'turn:call',name:'database_query',arguments:{connection:'original',sql:'SELECT id'},state:'returned',sequence:'9007199254740993',result:{kind:'database_outcome',data:{kind:'query',data:{columns:['id'],rows:[[{kind:'integer',value:'9223372036854775807'}]],affected_rows:'0',truncated:false}}}};
  const snapshot={connected:true,session:'session',revision:'4',latest:call.sequence,calls:[call]};
  assert.equal(updates(snapshot,null)[0].live,false);
  const before={connected:true,session:'session',revision:'4',latest:'9007199254740992',calls:[]};
  assert.equal(updates(snapshot,before)[0].live,true);
  assert.equal(updates(snapshot,{...before,revision:'3'})[0].live,false);
  assert.deepEqual(updates(snapshot,snapshot),[]);
  assert.equal(result(call).rows.rows[0][0].value,'9223372036854775807');
  assert.deepEqual(result({...call,result:{error:{code:'permission_denied',message:'Denied'}}}),{error:{code:'permission_denied',message:'Denied'}});
  assert.deepEqual(result({...call,result:null,state:'interrupted'}),{error:{code:'cancelled'}});
});


test('table events select, open and copy the displayed query',async()=>{
  const source=(await readFile(new URL('../dev.sailry.platform/desktop/main.js',import.meta.url),'utf8'))
    .replace(/^import[\s\S]*?;\n/gm,'').replace('export default class','class').replace(/^export /gm,'');
  const copied=[],opened=[];
  const Controller=vm.runInNewContext(`${source}\nDatabases`,{View:class{},writeClipboard:text=>copied.push(text)}),owner=new Controller();
  const cx={notify(){}};
  Object.assign(owner,{opened:'first',logs:[{id:'older',sql:'SELECT older',database:'one'},{id:'newer',sql:'SELECT newer',database:'two'}],selectedLog:null});
  owner.openSql=(sql,database,context)=>opened.push([sql,database,context]);
  const revision='first:2:newer',event=(kind,row,extra={})=>owner.historyEvent({table:'database-history-table',revision,kind,row,...extra},cx);
  event('selection',0);assert.equal(owner.selectedLog,'newer','row 0 is the newest');
  event('selection',null);assert.equal(owner.selectedLog,null);
  event('open',1);assert.deepEqual(opened,[['SELECT older','one',cx]]);
  event('copy',0);assert.deepEqual(copied,['SELECT newer'],'⌘C copies the query alone');
  event('action',1,{action:'copy'});event('action',0,{action:'open'});
  assert.deepEqual(copied,['SELECT newer','SELECT older']);assert.deepEqual(opened.at(-1),['SELECT newer','two',cx]);
  owner.historyEvent({table:'database-history-table',revision:'first:1:older',kind:'open',row:0},cx);
  assert.equal(opened.length,2,'a stale revision names another query');
  owner.opened='other';event('open',0);assert.equal(opened.length,2,'another profile\'s history is not this one');
});

test('query dialog preserves its draft and closes after success',async()=>{
  const source=(await readFile(new URL('../dev.sailry.platform/desktop/main.js',import.meta.url),'utf8'))
    .replace(/^import[\s\S]*?;\n/gm,'').replace('export default class','class').replace(/^export /gm,'');
  const set=[];
  const Controller=vm.runInNewContext(`${source}\nDatabases`,{View:class{},setText:(_,text)=>set.push(text),focusText(){},readText:()=>'SELECT 1'}),owner=new Controller();
  const cx={notify(){}};
  Object.assign(owner,{connected:true,pending:null,sqlOpen:false,dialog:0,opened:'first',profiles:[{id:'first',revision:1,read_only:true}],catalog:{database:'main'}});
  owner.control({id:'db-new-query'},cx);
  assert.equal(owner.sqlOpen,true);assert.equal(owner.dialog,1);assert.deepEqual(set,[],'a new query keeps the last draft');
  owner.openSql('SELECT 2','other',cx);
  assert.equal(owner.dialog,1,'an open dialog takes new text in place');assert.deepEqual(set,['SELECT 2']);assert.equal(owner.catalog.database,'other');
  let after=null;
  owner.execute=(value,_cx,origin,then)=>{assert.equal(value.data.sql,'SELECT 1');after=then;};
  owner.run=action=>action(cx);
  owner.runSql(cx);
  assert.equal(owner.sqlOpen,true,'the dialog stays open while its query runs');
  after();assert.equal(owner.sqlOpen,false);
  owner.control({id:'db-new-query'},cx);owner.close(cx);assert.equal(owner.sqlOpen,false);
});

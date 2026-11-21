import assert from 'node:assert/strict';
import test from 'node:test';
import {context} from '../dev.sailry.platform/host/context.js';

const revision = 'a'.repeat(64);
const symbol = (path='src/main.rs',start=0,end=1) => ({path,language:'rust',name:'entry',kind:'function',
  range:{start_byte:start*10,end_byte:end*10,start_line:start,start_column:0,end_line:end,end_column:1},
  signature:'fn entry()',file_revision:revision,depth:0});
const page = symbols => ({symbols,index_revision:7,partial:false,truncated:false,visited:1,skipped:0,issues:[]});
const definition = (path,name,line) => ({path,name,kind:'function',start_line:line,end_line:line+2});
const graph = overrides => ({view:'context',index_revision:7,targets:[],callers:[],callees:[],impact:[],map:[],
  total:0,next_offset:null,partial:false,truncated:false,visited:1,skipped:0,issues:[],...overrides});
function search(symbols,input={}) {
  const start = context({arguments:input,step:0});
  assert.equal(start.call.operation,'code.search');
  return context({step:1,state:start.state,outcome:{kind:'code_search',data:page(symbols)}});
}
/** Answers the graph step and returns the first source read. */
function related(symbols,input={},overrides={}) {
  const pending = search(symbols,input);
  assert.equal(pending.call.operation,'code.graph');
  return context({step:2,state:pending.state,outcome:{kind:'code_graph',data:graph(overrides)}});
}
function reply(pending,text='fn entry() {}\n',count=1) {
  const query = pending.call.arguments;
  return {kind:'file_lines',data:{path:query.path,revision:query.expected_revision,offset:query.offset,
    size:Buffer.byteLength(text)*count,next_offset:null,
    lines:Array.from({length:count},(_,index) => ({number:query.offset+index+1,text}))}};
}

test('requests the first symbol graph before reading source',() => {
  const pending = search([symbol('src/main.rs',4,9),symbol('src/other.rs',1,2)]);
  assert.deepEqual(pending.call,{operation:'code.graph',arguments:{view:'context',name:'entry',paths:['src/main.rs'],
    line:4,limit:8}});
  assert.equal(pending.optional,true);
  assert.equal(pending.state.next,0);
});

test('merges overlapping ranges before expansion',() => {
  const pending = related([symbol('main.rs',2,4),symbol('main.rs',3,6),symbol('main.rs',2,4)]);
  assert.deepEqual(pending.call,{operation:'files.read_lines',arguments:{path:'main.rs',offset:0,limit:9,expected_revision:revision}});
  // A failed revision-checked read reaches the handler, which keeps the other snippets.
  assert.equal(pending.optional,true);
  const completed = context({step:3,state:pending.state,outcome:reply(pending,'line\r\n',9)});
  assert.equal(completed.result.symbols.length,3);
  assert.equal(completed.result.snippets.length,1);
  assert.equal(completed.result.snippets[0].start_line,1);
  assert.equal(completed.result.snippets[0].end_line,9);
  assert.equal(completed.result.snippets[0].text,'line\r\n'.repeat(9));
  assert.equal(completed.result.snippets[0].file_revision,revision);
  assert.equal(completed.result.index_revision,7);
  assert.equal(completed.result.partial,false);
});

test('returns compact one-based symbols',() => {
  const pending = related([symbol('main.rs',2,4),{...symbol('main.rs',6,8),signature:null,
    range:{start_byte:60,end_byte:80,start_line:6,start_column:0,end_line:8,end_column:0}}]);
  const completed = context({step:3,state:pending.state,outcome:reply(pending,'line\n',9)});
  assert.deepEqual(completed.result.symbols,[
    {path:'main.rs',language:'rust',name:'entry',kind:'function',depth:0,start_line:3,end_line:5,signature:'fn entry()'},
    {path:'main.rs',language:'rust',name:'entry',kind:'function',depth:0,start_line:7,end_line:8},
  ]);
});

test('adds bounded callers callees and impact',() => {
  const caller = index => ({path:`src/caller${index}.rs`,name:'entry',kind:'call',line:index,column:4,count:index === 0 ? 2 : 1,
    scope:definition(`src/caller${index}.rs`,`run${index}`,0),targets:[definition('src/main.rs','entry',0)],candidates:1,
    confidence:'imported',ambiguous:false});
  const impacted = index => ({path:`src/user${index}.rs`,depth:1,confidence:'directory',ambiguous:index === 1,truncated:false,
    symbols:[1,2,3,4].map(line => ({name:`use${line}`,kind:'function',start_line:line,end_line:line+1,depth:1,
      confidence:line === 4 ? 'workspace' : 'directory',ambiguous:false}))});
  const pending = related([symbol()],{},{
    callers:Array.from({length:10},(_,index) => caller(index)),
    callees:[
      {path:'src/main.rs',name:'helper',kind:'call',line:0,column:2,count:1,scope:null,
        targets:[definition('src/util.rs','helper',4)],candidates:1,confidence:'directory',ambiguous:false},
      {path:'src/main.rs',name:'open',kind:'call',line:1,column:2,count:3,scope:null,
        targets:[definition('a.rs','open',0),definition('b.rs','open',0)],candidates:2,confidence:'workspace',ambiguous:true},
      {path:'src/main.rs',name:'println',kind:'call',line:2,column:2,count:1,scope:null,targets:[],candidates:0,
        confidence:null,ambiguous:false},
    ],
    impact:Array.from({length:6},(_,index) => impacted(index)),total:6,
  });
  const result = context({step:3,state:pending.state,outcome:reply(pending)}).result;
  assert.equal(result.callers.length,8);
  assert.deepEqual(result.callers[0],{path:'src/caller0.rs',line:1,kind:'call',scope:'run0',count:2,confidence:'imported'});
  assert.deepEqual(result.callees,[
    {name:'helper',line:1,kind:'call',confidence:'directory',definition:{path:'src/util.rs',line:5}},
    {name:'open',line:2,kind:'call',count:3,confidence:'workspace',ambiguous:true,candidates:2},
  ]);
  assert.equal(result.impact.total,6);
  assert.equal(result.impact.files.length,5);
  assert.deepEqual(result.impact.files[1].symbols.map(item => item.name),['use1','use2','use3']);
  assert.equal(result.impact.files[1].ambiguous,true);
  assert.equal(result.impact.files[0].truncated,true);
  assert.equal(result.truncated,true);
  assert.equal(result.snippets.length,1);
  assert.ok(Buffer.byteLength(JSON.stringify(result)) <= 16384);
});

test('degrades to symbols and snippets when the graph fails',() => {
  const failures = [
    {isError:true,error:{code:'busy',message:'Code query capacity is exhausted'}},
    {isError:true,error:{code:'outcome_unknown',message:'x'.repeat(400)}},
    {kind:'code_graph',data:graph({view:'map'})},
    {kind:'code_graph'},
  ];
  for (const outcome of failures) {
    const pending = search([symbol()]);
    const reading = context({step:2,state:pending.state,outcome});
    assert.equal(reading.call.operation,'files.read_lines');
    const completed = context({step:3,state:reading.state,outcome:reply(reading)});
    const result = completed.result;
    assert.equal(result.isError,undefined);
    assert.equal(result.symbols.length,1);
    assert.equal(result.snippets.length,1);
    assert.equal(result.partial,true);
    assert.ok(result.graph_unavailable.length > 0 && Array.from(result.graph_unavailable).length <= 160);
    for (const key of ['callers','callees','impact']) assert.equal(result[key],undefined);
  }
  const pending = search([symbol()]);
  const reading = context({step:2,state:pending.state,outcome:failures[0]});
  assert.equal(reading.state.data.graph_unavailable,'Code query capacity is exhausted');
});

test('reports pagination before the requested range ends',() => {
  const pending = related([symbol('main.rs',10,11)]);
  const outcome = reply(pending,'first line\n');
  outcome.data.next_offset = pending.call.arguments.offset+1;
  const completed = context({step:3,state:pending.state,outcome});
  assert.equal(completed.call,undefined);
  assert.equal(completed.result.partial,true);
  assert.equal(completed.result.truncated,true);
  assert.equal(completed.result.snippets.length,1);
  assert.equal(completed.result.snippets[0].truncated,true);
  assert.equal(completed.result.snippets[0].text,'first line\n');
  assert.ok(Buffer.byteLength(JSON.stringify(completed.result)) < 16384);
});

test('keeps complete ranges with later file pages',() => {
  const pending = related([symbol('main.rs',10,11)]);
  const query = pending.call.arguments;
  const outcome = reply(pending,'line\n',query.limit);
  outcome.data.next_offset = query.offset+query.limit;
  const completed = context({step:3,state:pending.state,outcome});
  assert.equal(completed.call,undefined);
  assert.equal(completed.result.partial,false);
  assert.equal(completed.result.truncated,false);
  assert.equal(completed.result.snippets[0].truncated,false);
  assert.equal(completed.result.snippets[0].end_line,query.offset+query.limit);
  assert.equal(completed.result.snippets[0].text,'line\n'.repeat(query.limit));
});

test('caps symbols snippets and operation count',() => {
  let pending = related(Array.from({length:30},(_,index) => symbol(`file${index}.rs`)));
  let step = 3, reads = 0;
  while (pending.call) {
    assert.equal(pending.call.operation,'files.read_lines');
    reads++;
    pending = context({step:step++,state:pending.state,outcome:reply(pending)});
  }
  assert.equal(reads,5);
  assert.equal(step,8);
  assert.equal(pending.result.symbols.length,20);
  assert.equal(pending.result.snippets.length,5);
  assert.equal(pending.result.partial,true);
  assert.equal(pending.result.truncated,true);
  assert.ok(Buffer.byteLength(JSON.stringify(pending.result)) <= 16384);
});

test('bounds escaped multibyte output by bytes',() => {
  const pending = related([symbol()],{max_bytes:1024});
  const result = context({step:3,state:pending.state,outcome:reply(pending,'中文🙂"\\\t'.repeat(1000))}).result;
  assert.equal(result.partial,true);
  assert.equal(result.truncated,true);
  assert.ok(result.snippets[0].text.length > 0);
  assert.ok(Buffer.byteLength(JSON.stringify(result)) <= 1024);
  assert.equal(result.snippets[0].text.includes('\ufffd'),false);
  assert.equal(result.snippets[0].truncated,true);
});

test('bounds metadata before requesting source',() => {
  const large = {...symbol(),name:'字'.repeat(10000)};
  const completed = search([large],{max_bytes:512});
  assert.equal(completed.call,undefined);
  assert.deepEqual(completed.result.symbols,[]);
  assert.deepEqual(completed.result.snippets,[]);
  assert.equal(completed.result.truncated,true);
  assert.ok(Buffer.byteLength(JSON.stringify(completed.result)) <= 512);
});

test('skips a changed file without stale expansion',() => {
  const pending = related([symbol()]);
  const outcome = {isError:true,error:{code:'revision_conflict',message:'File changed'}};
  const completed = context({step:3,state:pending.state,outcome});
  assert.equal(completed.result.isError,undefined);
  assert.equal(completed.result.partial,true);
  assert.deepEqual(completed.result.snippets,[]);
  assert.equal(completed.result.symbols.length,1);
});

test('continues revision-checked reads after a later conflict',() => {
  const first = related([symbol('first.rs'),symbol('changed.rs'),symbol('remaining.rs')]);
  const second = context({step:3,state:first.state,outcome:reply(first)});
  assert.equal(second.call.arguments.path,'changed.rs');
  assert.equal(second.state.data.snippets.length,1);
  const outcome = {isError:true,error:{code:'revision_conflict',message:'File changed'}};
  const third = context({step:4,state:second.state,outcome});
  assert.equal(third.call.arguments.path,'remaining.rs');
  assert.equal(third.state.data.partial,true);
  const completed = context({step:5,state:third.state,outcome:reply(third)});
  assert.deepEqual(completed.result.snippets.map(snippet => snippet.path),['first.rs','remaining.rs']);
  assert.equal(completed.result.partial,true);
});

test('rejects a mismatched successful source revision',() => {
  const pending = related([symbol()]);
  const outcome = reply(pending);
  outcome.data.revision = 'b'.repeat(64);
  const completed = context({step:3,state:pending.state,outcome});
  assert.equal(completed.result.isError,true);
  assert.deepEqual(completed.result.error,{code:'invalid_request',message:'Invalid revision-checked source result'});
});

test('clips long declarations and reports incomplete context',() => {
  const pending = related([symbol('large.rs',1,1000)]);
  assert.equal(pending.call.arguments.limit,80);
  const result = context({step:3,state:pending.state,outcome:reply(pending,'line\n',80)}).result;
  assert.equal(result.partial,true);
  assert.equal(result.truncated,true);
  assert.equal(result.snippets[0].truncated,true);
  assert.equal(result.snippets[0].end_line,80);
});

test('preserves incomplete search metadata',() => {
  const start = context({arguments:{name:'entry',path:'src/',language:'rust',limit:3},step:0});
  assert.deepEqual(start.call.arguments,{name:'entry',path:'src/',language:'rust',limit:3});
  const data = {...page([]),partial:true,skipped:1,issues:[{path:'vendor.sql',status:'unsupported'}]};
  const completed = context({step:1,state:start.state,outcome:{kind:'code_search',data}});
  assert.equal(completed.result.partial,true);
  assert.deepEqual(completed.result.issues,data.issues);
  assert.equal(completed.result.skipped,1);
});

test('rejects invalid limits and continuations',() => {
  for (const input of [{limit:21},{max_snippets:6},{max_bytes:16385},{max_bytes:100},{query:3}]) {
    assert.equal(context({arguments:input,step:0}).result.isError,true);
  }
  assert.equal(context({step:8}).result.isError,true);
});

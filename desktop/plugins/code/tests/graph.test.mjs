import assert from 'node:assert/strict';
import test from 'node:test';
import {impact,map} from '../dev.sailry.platform/host/graph.js';

const page = (view,overrides) => ({view,index_revision:9,targets:[],callers:[],callees:[],impact:[],map:[],total:0,
  next_offset:null,partial:false,truncated:false,visited:3,skipped:0,issues:[],...overrides});
function answer(handler,input,data) {
  const pending = handler({arguments:input,step:0});
  assert.equal(pending.call.operation,'code.graph');
  return handler({arguments:input,step:1,state:pending.state,outcome:{kind:'code_graph',data}});
}
const affected = (name,line,depth,confidence='imported',ambiguous=false) =>
  ({name,kind:'function',start_line:line,end_line:line+3,depth,confidence,ambiguous});

test('builds impact queries from symbols lines and changes',() => {
  for (const [input,query] of [
    [{name:'run'},{view:'impact',depth:3,limit:20,offset:0,name:'run'}],
    [{path:'src/main.rs',line:12,depth:2},{view:'impact',depth:2,limit:20,offset:0,paths:['src/main.rs'],line:11}],
    [{paths:['src/a.rs','src/b'],limit:5,offset:10},{view:'impact',depth:3,limit:5,offset:10,paths:['src/a.rs','src/b']}],
    [{name:'run',path:'src/main.rs'},{view:'impact',depth:3,limit:20,offset:0,name:'run',paths:['src/main.rs']}],
  ]) {
    assert.deepEqual(impact({arguments:input,step:0}).call,{operation:'code.graph',arguments:query});
  }
  for (const input of [{},{line:3},{path:'a.rs',paths:['b.rs']},{name:''},{name:'run',depth:4},{name:'run',limit:51},
    {paths:'a.rs'},{paths:[1]},{path:'a.rs',line:0},{paths:['a.rs','b.rs'],line:2},{name:'run',max_bytes:100}]) {
    assert.equal(impact({arguments:input,step:0}).result.isError,true,JSON.stringify(input));
  }
});

test('reports impact with one-based lines and confidence',() => {
  const result = answer(impact,{name:'open'},page('impact',{
    targets:[{path:'src/store.rs',name:'open',kind:'function',start_line:0,end_line:2}],
    impact:[
      {path:'src/cli.rs',depth:1,confidence:'imported',ambiguous:false,truncated:false,
        symbols:[affected('main',3,1),affected('helper',9,1,'workspace',true)]},
      {path:'docs/demo.rs',depth:2,confidence:'workspace',ambiguous:true,truncated:true,symbols:[]},
    ],
    total:3,next_offset:2,truncated:true,
  })).result;
  assert.deepEqual(result.targets,[{path:'src/store.rs',name:'open',kind:'function',start_line:1,end_line:3}]);
  assert.deepEqual(result.files,[
    {path:'src/cli.rs',depth:1,confidence:'imported',symbols:[
      {name:'main',kind:'function',start_line:4,depth:1},
      {name:'helper',kind:'function',start_line:10,depth:1,confidence:'workspace',ambiguous:true},
    ]},
    {path:'docs/demo.rs',depth:2,confidence:'workspace',ambiguous:true,symbols:[],truncated:true},
  ]);
  assert.deepEqual([result.total,result.next_offset,result.truncated,result.partial,result.index_revision],[3,2,true,false,9]);
});

test('bounds impact output by bytes',() => {
  const impacted = Array.from({length:50},(_,index) => ({path:`src/${'module_'.repeat(8)}${index}.rs`,depth:1,
    confidence:'directory',ambiguous:false,truncated:false,symbols:[affected(`caller_${index}`,index,1,'directory')]}));
  const result = answer(impact,{name:'open',max_bytes:1024,offset:7},
    page('impact',{impact:impacted,total:70,next_offset:57})).result;
  assert.ok(result.files.length > 0 && result.files.length < 50);
  assert.equal(result.truncated,true);
  assert.equal(result.partial,true);
  assert.equal(result.next_offset,7+result.files.length);
  assert.ok(Buffer.byteLength(JSON.stringify(result)) <= 1024);
});

test('resumes map output at the first omitted file',() => {
  const entries = Array.from({length:10},(_,index) => ({path:`src/${index}.rs`,score:100-index,symbols:[
    {name:`method_${index}`,kind:'function',start_line:0,end_line:10,signature:'parameter: String, '.repeat(20),references:4},
  ]}));
  const result = answer(map,{max_bytes:1024,offset:12},page('map',{map:entries,total:22})).result;
  assert.ok(result.files.length > 0 && result.files.length < entries.length);
  assert.equal(result.next_offset,12+result.files.length);
  assert.equal(result.truncated,true);
  assert.equal(result.partial,true);
  assert.ok(Buffer.byteLength(JSON.stringify(result)) <= 1024);
  assert.deepEqual(result.files.map(file => file.path),entries.slice(0,result.files.length).map(file => file.path));
});

test('ranks map entries with signatures',() => {
  assert.deepEqual(map({arguments:{},step:0}).call,{operation:'code.graph',arguments:{view:'map',limit:20,offset:0}});
  assert.deepEqual(map({arguments:{paths:['src/auth'],limit:5},step:0}).call.arguments,
    {view:'map',limit:5,offset:0,paths:['src/auth']});
  const result = answer(map,{},page('map',{map:[
    {path:'src/store.rs',score:1000,symbols:[
      {name:'open',kind:'function',start_line:0,end_line:2,signature:'pub fn open() -> Store',references:4},
      {name:'Store',kind:'struct',start_line:4,end_line:4,signature:null,references:0},
    ]},
  ],total:12,next_offset:1,truncated:true})).result;
  assert.deepEqual(result.files,[{path:'src/store.rs',score:1000,symbols:[
    {name:'open',kind:'function',start_line:1,end_line:3,signature:'pub fn open() -> Store',references:4},
    {name:'Store',kind:'struct',start_line:5,end_line:5},
  ]}]);
  assert.deepEqual([result.total,result.next_offset,result.truncated],[12,1,true]);
});

test('advances past oversized first entries',() => {
  for (const handler of [impact,map]) {
    const input = {name:'open',max_bytes:1024,offset:4};
    const oversized = {path:'x'.repeat(2048),depth:1,confidence:'imported',score:1,symbols:[]};
    const view = handler === impact ? 'impact' : 'map';
    const result = answer(handler,input,page(view,{[view]:[oversized],total:6,next_offset:5})).result;
    assert.equal(result.isError,undefined);
    assert.deepEqual(result.files,[]);
    assert.equal(result.next_offset,5);
    assert.equal(result.partial,true);
    assert.equal(result.truncated,true);
    assert.ok(Buffer.byteLength(JSON.stringify(result)) <= 1024);
  }
});

test('passes graph failures and rejects mismatched results',() => {
  const failure = {isError:true,error:{code:'busy',message:'Busy'}};
  const pending = map({arguments:{},step:0});
  assert.equal(map({step:1,state:pending.state,outcome:failure}).result,failure);
  assert.equal(answer(map,{},page('impact',{})).result.isError,true);
  assert.equal(map({step:2,state:pending.state}).result.isError,true);
  assert.equal(impact({step:1,state:{}}).result.isError,true);
  assert.equal(map({arguments:{paths:[]},step:0}).call.arguments.paths,undefined);
});

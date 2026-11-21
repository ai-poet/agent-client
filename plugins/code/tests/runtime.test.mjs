import assert from 'node:assert/strict';
import {execute,packageInput,runtimeTest} from '../../scripts/test-host.mjs';

const directory = new URL('../',import.meta.url);

runtimeTest('production script prepares a bounded context search',async () => {
  const fixture = await packageInput(directory,'context',{
    arguments:{query:'render',path:'src/'},step:0,
  },{policy:{call_id:'00000000-0000-4000-8000-000000000003',mode:'plan',read_only:true,turn:{}}});
  const output = execute(fixture);
  assert.deepEqual(output.result.Ok.call,{operation:'code.search',arguments:{query:'render',path:'src/',limit:20}});
  assert.deepEqual(output.calls,[]);
});

runtimeTest('production script preserves versioned source ranges',async () => {
  const fixture = await packageInput(directory,'context',{
    step:1,state:{query:{limit:20},snippets:5,budget:16384,next:0},
    outcome:{kind:'code_search',data:{index_revision:3,partial:false,truncated:false,visited:1,skipped:0,issues:[],
      symbols:[{path:'src/vue.vue',language:'vue',name:'render',kind:'function',depth:0,signature:null,
        file_revision:'a'.repeat(64),range:{start_byte:16,end_byte:32,start_line:1,start_column:3,end_line:2,end_column:4}}]}},
  });
  const output = execute(fixture);
  assert.deepEqual(output.result.Ok.call,{operation:'code.graph',arguments:{
    view:'context',name:'render',paths:['src/vue.vue'],line:1,limit:8,
  }});
  assert.equal(output.result.Ok.optional,true);
  assert.deepEqual(output.result.Ok.state.ranges,[{path:'src/vue.vue',file_revision:'a'.repeat(64),
    start:0,end:5,limit:5,truncated:false}]);
  assert.equal(output.result.Ok.state.data.index_revision,3);
  assert.deepEqual(output.calls,[]);
});

runtimeTest('production script shapes a bounded impact query',async () => {
  const fixture = await packageInput(directory,'impact',{arguments:{path:'src/main.rs',line:12},step:0});
  const output = execute(fixture);
  assert.deepEqual(output.result.Ok.call,{operation:'code.graph',arguments:{
    view:'impact',depth:3,limit:20,offset:0,paths:['src/main.rs'],line:11,
  }});
  assert.deepEqual(output.calls,[]);
});

async function sourcePage(count,nextOffset) {
  const revision = 'a'.repeat(64);
  return packageInput(directory,'context',{
    step:3,state:{query:{limit:20},snippets:5,budget:16384,next:0,
      data:{symbols:[],snippets:[],index_revision:3,partial:false,truncated:false,visited:1,skipped:0,issues:[]},
      ranges:[{path:'src/main.rs',file_revision:revision,start:8,end:14,limit:6,truncated:false}]},
    outcome:{kind:'file_lines',data:{path:'src/main.rs',revision,offset:8,size:200000,next_offset:nextOffset,
      lines:Array.from({length:count},(_,index) => ({number:9+index,text:'short line\n'}))}},
  });
}

runtimeTest('production script reports shortened source pages',async () => {
  const output = execute(await sourcePage(1,9));
  const result = output.result.Ok.result;
  assert.equal(output.result.Ok.call,undefined);
  assert.equal(result.partial,true);
  assert.equal(result.truncated,true);
  assert.equal(result.snippets[0].truncated,true);
  assert.equal(result.snippets[0].text,'short line\n');
  assert.ok(Buffer.byteLength(JSON.stringify(result)) < 16384);
  assert.deepEqual(output.calls,[]);
});

runtimeTest('production script accepts fully covered source pages',async () => {
  const output = execute(await sourcePage(6,14));
  const result = output.result.Ok.result;
  assert.equal(output.result.Ok.call,undefined);
  assert.equal(result.partial,false);
  assert.equal(result.truncated,false);
  assert.equal(result.snippets[0].truncated,false);
  assert.equal(result.snippets[0].end_line,14);
  assert.equal(result.snippets[0].text,'short line\n'.repeat(6));
  assert.deepEqual(output.calls,[]);
});

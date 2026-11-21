import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {State,query,choices,nodeId,resourceId,unit,offset} from '../dev.sailry.platform/desktop/state.js';
import {metrics} from '../dev.sailry.platform/desktop/metrics.js';
import {messages} from '../dev.sailry.platform/desktop/locales.js';
const text=messages('en');
test('speed details use latest own time to first token',()=>{
  for (const locale of ['en','zh-CN','de','es','fr','ja','ko','pt-BR','ru','zh-TW']) {
    const text=messages(locale);
    for (const [first,value] of [[null,text.composer_metric_unknown],['0','0.0 ms'],['125000','125.0 ms'],['2000000','2.00 s']]) {
      const statistics={first_token_us:first,generation:{output_tokens:'20',elapsed_us:'1000000',responses:'1'},
        delegated:{first_token_us:'1',generation:{output_tokens:'10000',elapsed_us:'1',responses:'2'}}};
      const before=structuredClone(statistics),speed=metrics({has_messages:true,statistics},text).find(value=>value.id==='composer_speed');
      assert.equal(speed.value,'20.0/s');
      assert.deepEqual(speed.details[2],{label:{label:text.composer_first_token},value});
      assert.deepEqual(statistics,before);
      assert.ok(text.composer_first_token);
    }
  }
});
test('normalizes public endpoint bytes into an opaque stable selection',()=>{
  assert.equal(nodeId(Array(32).fill(15)),'0f'.repeat(32));
  assert.equal(new State(Array(32).fill(255)).node,'ff'.repeat(32));
});
const position={node:'node-a',timestamp_ms:'1',sequence:'18446744073709551615'};
const report={requests:{items:[{position}],has_more:true}};
test('cursor pagination retains prior geometry and rejects stale or incomplete observations',()=>{
  const state=new State('node-a'),generation=state.start();
  assert(state.accept({cursor:'9',all:false,view:{report,connected:true,refreshing:false}},generation));
  assert(state.turn(2));assert.equal(state.data(),report);assert.equal(state.query.before.sequence,position.sequence);
  const next=state.start();assert(!state.accept({cursor:'10',all:false,view:{}},generation));
  assert(state.accept({cursor:'1',all:false,view:{report,connected:true,refreshing:true}},next));
  assert.equal(state.data(),report);assert(!state.ready());assert(!state.turn(3));
  assert(state.accept({cursor:'11',all:false,view:{report,connected:true,refreshing:false}},next));
  assert(!state.accept({cursor:'9',all:false,view:null},next));assert(state.ready());
  assert(state.turn(1));assert.equal(state.query.before,null);
});
test('aggregate pages retain source identity without host controls',()=>{
  const state=new State('a'),generation=state.start();
  assert(!state.select('host','all'));assert(!state.select('host','b'));assert.equal(state.node,'a');
  const summary={...report,groups:[{node:'b',group:{key:{kind:'project',data:'same'}}}]};
  assert(state.all);
  assert(state.accept({cursor:'1',all:true,view:{sources:[{node:'b',view:{report:{cursor:'9'},connected:true,refreshing:false}}],summary,complete:true}},generation));
  assert.equal(state.data().groups[0].node,'b');assert.equal(state.resourceCursor(),'b:9');assert(state.turn(2));
  assert.equal(state.query.before.node,'node-a');assert.equal(state.query.before.sequence,position.sequence);
});
test('project and provider filters route their owning nodes',()=>{
  const state=new State('local');
  assert(state.select('project',resourceId('host-b','same-project')));
  assert.equal(state.node,'host-b');assert.equal(state.all,false);assert.deepEqual(state.query.projects,['same-project']);
  assert(state.select('provider',resourceId('host-b','same-provider')));
  assert.deepEqual(state.query.providers,['same-provider']);assert.equal(state.selected('provider'),'host-b:same-provider');
  assert(!state.select('provider',resourceId('host-a','same-provider')));assert.equal(state.node,'host-b');
  assert(state.select('provider',''));assert.equal(state.node,'host-b');assert.equal(state.all,false);
  assert(state.select('project',''));assert.equal(state.all,true);assert.equal(state.node,'local');
  assert(state.select('provider','host-a:same-provider'));assert.equal(state.node,'host-a');assert.equal(state.all,false);
  assert(state.select('model','model-name'));assert.deepEqual(state.query.models,['model-name']);
  assert(state.select('provider',''));assert.equal(state.all,true);
  assert(state.select('model','model-name'));assert.equal(state.all,true);assert.deepEqual(state.query.models,['model-name']);
});
test('project choices match inventories even without usage',()=>{
  const state=new State('host-a'),inventory=[
    {node:'host-a',label:'Laptop',projects:[{id:'same',name:'App'}],providers:[{id:'p',name:'Local',models:['one']}]},
    {node:'host-b',label:'Server',projects:[{id:'same',name:'App'}],providers:[{id:'p',name:'Remote',models:['two']}]},
  ];
  let options=choices(state,inventory,[{node:'host-a',kind:'project',data:'removed'}],text);
  assert.deepEqual(options.project.map(item=>item.id),['','host-a:same','host-b:same']);
  assert.deepEqual(options.project.map(item=>item.label),['All projects','App · Laptop','App · Server']);
  assert.deepEqual(options.provider.map(item=>item.id),['','host-a:p','host-b:p']);
  assert(state.select('provider','host-b:p'));options=choices(state,inventory,[],text);
  assert.deepEqual(options.model.map(item=>item.id),['','two']);
  assert(state.select('project','host-b:same'));options=choices(state,inventory,[],text);
  assert.deepEqual(options.provider.map(item=>item.id),['','host-b:p']);
  assert.deepEqual(options.model.map(item=>item.id),['','two']);
  assert(state.select('provider','host-b:p'));options=choices(state,inventory,[],text);
  assert.deepEqual(options.model.map(item=>item.id),['','two']);
});
test('partial overview keeps honest data and source failures',()=>{
  const state=new State('a'),generation=state.start();
  state.accept({cursor:'1',all:true,view:{summary:{...report,groups:[]},complete:false,sources:[
    {node:'a',view:{report:{cursor:'1'},connected:true,refreshing:false}},
    {node:'b',view:{connected:false,refreshing:false,error:{code:'unavailable'}}},
  ]}},generation);
  assert.equal(state.data().requests,report.requests);assert.equal(state.view.partial,true);
  assert.equal(state.view.error,null);assert.equal(state.overview.sources[1].view.error.code,'unavailable');
  assert.equal(state.ready(),false);assert.equal(state.turn(2),false);
});
test('revoked access clears retained pagination data while transient errors preserve it',()=>{
  for(const code of ['not_configured','not_found','permission_denied','revision_conflict']) {
    const state=new State('a'),generation=state.start();
    state.accept({cursor:'1',all:false,view:{report,connected:true,refreshing:false}},generation);
    assert(state.turn(2));const next=state.start();
    state.accept({cursor:'1',all:false,view:{report:null,connected:true,refreshing:false,error:{code:'unavailable'}}},next);
    assert.equal(state.data(),report);
    state.accept({cursor:'2',all:false,view:{report:null,connected:true,refreshing:false,error:{code}}},next);
    assert.equal(state.data(),null);assert.equal(state.previous,null);assert(!state.ready());
    state.accept({cursor:'3',all:false,view:{report,connected:true,refreshing:false,error:null}},next);
    assert.equal(state.data(),report);assert(state.ready());
  }
});
test('filter changes reset dependent fields and preserve frozen deleted model choices',()=>{
  const state=new State('a');state.query.providers=['p'];state.query.models=['removed'];state.query.projects=['x'];
  state.query.dimension='project';
  assert(!state.select('range','30'));
  state.select('range','7d');assert.deepEqual(state.query.models,['removed']);assert.equal(state.query.dimension,'project');
  const options=choices(state,{projects:[],providers:[]},[{kind:'model',data:{provider:'p',model:'old'}}],text);
  assert(options.model.some(item=>item.id==='old'));assert(options.model.some(item=>item.id==='removed'));
  state.select('project','next');assert.deepEqual(state.query.providers,[]);assert.deepEqual(state.query.models,[]);
});
test('filter reads retain prior reports until current data arrives',()=>{
  for(const field of ['range','project','provider','model']) {
    const state=new State('a'),generation=state.start();
    state.accept({cursor:'1',all:false,view:{report,connected:true,refreshing:false}},generation);
    assert(state.select(field,field==='range'?'7d':'selected'));
    const next=state.start(),replacement={requests:{items:[],has_more:false}};
    assert.equal(state.data(),report);assert(!state.ready());
    assert(!state.accept({cursor:'2',all:false,view:{report:replacement,connected:true,refreshing:false}},generation));
    assert.equal(state.data(),report);
    assert(state.accept({cursor:'1',all:false,view:{report:replacement,connected:true,refreshing:true}},next));
    assert.equal(state.data(),report);assert(!state.ready());
    assert(state.accept({cursor:'2',all:false,view:{report:null,connected:true,refreshing:false,error:{code:'unavailable'}}},next));
    assert.equal(state.data(),report);assert(!state.ready());
    assert(state.accept({cursor:'3',all:false,view:{report:replacement,connected:true,refreshing:false,error:null}},next));
    assert.equal(state.data(),replacement);assert(state.ready());
  }
});
test('composer preserves unknown measurements',()=>{
  const state={has_messages:true},before=structuredClone(state),unknown=metrics(state,text);
  assert.deepEqual(unknown.slice(0,5).map(metric=>metric.value),Array(5).fill(text.composer_metric_unknown));
  assert.deepEqual(unknown.slice(0,5).map(metric=>metric.details.map(detail=>detail.value)),
    [2,3,2,3,1].map(count=>Array(count).fill(text.composer_metric_unknown)));
  assert.equal(unknown.length,5);
  assert.deepEqual(state,before);
  const zero=metrics({has_messages:true,statistics:{usage:{input:'0',output:'0',cached_input:'0',reasoning:'0'},context_tokens:'100',context_limit:100},context_limit:100,compacting:true},text);
  assert.equal(zero[0].value,'0');
});
test('footer keeps units counts and unknown prices',async()=>{
  const manifest=JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  const icons=Object.fromEntries(manifest.extensions['dev.sailry.platform'].ui.map(entry=>[entry.id,entry.icon]));
  assert.equal(icons.composer_speed,'gauge');assert.equal(icons.composer_cost,'dollar-sign');assert.equal(icons.composer_turns,'messages-square');
  for(const locale of ['en','zh-CN']) {
    const text=messages(locale),statistics={turns:'7',responses:'12',generation:{output_tokens:'43',elapsed_us:'2000000',responses:'3'},cost:null};
    const before=structuredClone(statistics),values=metrics({has_messages:true,statistics},text);
    const metric=id=>values.find(value=>value.id===id);
    assert.equal(metric('composer_speed').value,'21.5/s');assert.equal(metric('composer_turns').value,'7');
    assert.equal(metric('composer_turns').details[0].value,'12');assert.equal(metric('composer_cost').value,text.composer_metric_unknown);
    assert.deepEqual(metric('composer_cost').details,[
      {label:{label:text.composer_this_session},value:text.composer_metric_unknown},
      {label:{label:text.composer_subagents},value:text.composer_metric_unknown}]);
    assert.deepEqual(statistics,before);
    const zero=metrics({has_messages:true,statistics:{...statistics,cost:{usd_micros:'0',responses:'12',breakdown:{input:'0',output:'0',cache_read:'0',cache_write:'0'}}}},text).find(value=>value.id==='composer_cost');
    assert.equal(zero.value,'0.00');assert.equal(zero.details[0].value,'$0.00');assert.equal(zero.details[1].value,text.composer_metric_unknown);
  }
});

test('composer combines child totals and preserves own metrics',()=>{
  const statistics={turns:'7',responses:'2',context_tokens:'60',context_limit:100,
    usage:{input:'100',output:'20',cached_input:'25',reasoning:'5'},
    generation:{output_tokens:'43',elapsed_us:'2000000',responses:'2'},
    cost:{usd_micros:'15000',responses:'2'},
    delegated:{responses:'3',tokens:{input:'200',output:'80',cached_input:'200',reasoning:'50'},
      cost:{usd_micros:'15000',responses:'3'},
      generation:{output_tokens:'4000',elapsed_us:'1',responses:'3'}}};
  const before=structuredClone(statistics);
  for(const locale of ['en','zh-CN']) {
    const text=messages(locale),values=metrics({has_messages:true,statistics,context_limit:100},text);
    const metric=id=>values.find(value=>value.id===id);
    assert.equal(metric('composer_tokens').value,'400');
    assert.deepEqual(metric('composer_tokens').details,[
      {label:{label:text.composer_this_session},value:'120'},
      {label:{label:text.composer_subagents},value:'280'}]);
    assert.equal(metric('composer_cost').value,'0.03');
    assert.deepEqual(metric('composer_cost').details,[
      {label:{label:text.composer_this_session},value:'$0.01'},
      {label:{label:text.composer_subagents},value:'$0.01'}]);
    assert.equal(metric('composer_speed').value,'21.5/s');
    assert.deepEqual(metric('composer_speed').details.map(detail=>detail.value),['2','43',text.composer_metric_unknown]);
    assert.equal(metric('composer_cache').value,'25%');
    assert.deepEqual(metric('composer_cache').details.map(detail=>detail.value),['25','75','100']);
    assert.equal(metric('composer_turns').value,'7');
    assert.equal(metric('composer_turns').details[0].value,'2');
    assert(!values.some(value=>value.id==='context' || value.id==='context_panel'));
    assert.ok(text.composer_this_session && text.composer_subagents);
  }
  assert.equal(messages('en').composer_this_session,'This session');
  assert.equal(messages('en').composer_subagents,'Subagents');
  assert.equal(messages('zh-CN').composer_this_session,'本会话');
  assert.equal(messages('zh-CN').composer_subagents,'子代理');
  assert.deepEqual(statistics,before);
});

test('composer defaults absent children without changing own values',()=>{
  const statistics={turns:'4',responses:'2',usage:{input:'100',output:'20',cached_input:'0',reasoning:'0'},
    cost:{usd_micros:'30000',responses:'1'}};
  const state={has_messages:true,statistics};
  const absent=metrics(state,text),empty=metrics({...state,statistics:{...statistics,
    delegated:{responses:'0',tokens:null,cost:null,generation:null}}},text);
  assert.deepEqual(absent,empty);
  const metric=id=>absent.find(value=>value.id===id);
  assert.equal(metric('composer_tokens').value,'120');
  assert.deepEqual(metric('composer_tokens').details.map(detail=>detail.value),['120',text.composer_metric_unknown]);
  assert.equal(metric('composer_cost').value,'0.03+');
  assert.deepEqual(metric('composer_cost').details.map(detail=>detail.value),['$0.03+',text.composer_metric_unknown]);
});

test('composer retains partial prices and exact microdollar sums',()=>{
  const statistics={responses:'2',cost:{usd_micros:'9007199254749998',responses:'1'},
    delegated:{responses:'3',cost:{usd_micros:'1',responses:'2'}}};
  let values=metrics({has_messages:true,statistics},text);
  let charge=values.find(value=>value.id==='composer_cost');
  assert.equal(charge.value,'9007199254.74+');
  assert.deepEqual(charge.details.map(detail=>detail.value),['$9007199254.74+','$0.00+']);
  values=metrics({has_messages:true,statistics:{responses:'2',cost:null,
    delegated:{responses:'3',cost:{usd_micros:'20000',responses:'3'}}}},text);
  charge=values.find(value=>value.id==='composer_cost');
  assert.equal(charge.value,'0.02+');
  assert.deepEqual(charge.details.map(detail=>detail.value),[text.composer_metric_unknown,'$0.02']);
});

function zone(name,body) {
  const prior=process.env.TZ;
  try {process.env.TZ=name;return body();}
  finally {if(prior===undefined)delete process.env.TZ;else process.env.TZ=prior;}
}
test('ranges map to trend buckets and default to thirty days',()=>{
  assert.deepEqual(['24h','7d','30d','90d','1y'].map(unit),['half_hour','day','day','week','week']);
  const state=new State('a');
  assert.equal(state.range,'30d');assert.equal(state.breakdown,'model');assert.equal(state.query.dimension,'model');
  assert.equal(state.query.end_ms-state.query.start_ms,30*86400000);
});
test('windows end at the next local boundary with the viewer offset',()=>{
  zone('Asia/Shanghai',()=>{
    const now=Date.UTC(2026,9,13,17,10);
    assert.equal(offset(now),480);
    const days=query('7d',now);
    // 01:10 on 10-14 in UTC+8 ends the window at local midnight starting 10-15.
    assert.equal(days.end_ms,Date.UTC(2026,9,14,16));
    assert.equal(days.start_ms,Date.UTC(2026,9,7,16));
    assert.equal(days.utc_offset_minutes,480);
    const hours=query('24h',now);
    assert.equal(hours.end_ms,Date.UTC(2026,9,13,17,30));
    assert.equal(hours.end_ms-hours.start_ms,86400000);
    assert.equal(query('1y',now).end_ms-query('1y',now).start_ms,365*86400000);
  });
  zone('America/New_York',()=>{
    const now=Date.UTC(2026,8,30,23);
    assert.equal(offset(now),-240);
    assert.equal(query('7d',now).end_ms,Date.UTC(2026,9,1,4));
  });
  zone('UTC',()=>assert.ok(Object.is(offset(Date.UTC(2026,0,1)),0)));
});
test('advancing keeps filters and reports moved windows',()=>{
  const state=new State('a');state.query.projects=['x'];state.query.dimension='project';
  state.query.end_ms-=1;
  assert(state.advance());
  assert(!state.advance());
  assert.deepEqual(state.query.projects,['x']);assert.equal(state.query.dimension,'project');
});
test('breakdown switches regroup only for projects and keep the request page',()=>{
  const state=new State('node-a'),generation=state.start();
  state.accept({cursor:'1',all:false,view:{report,connected:true,refreshing:false}},generation);
  assert(state.turn(2));state.start();
  state.accept({cursor:'1',all:false,view:{report,connected:true,refreshing:false}},state.generation);
  state.expanded=true;
  assert(!state.group('provider'));assert.equal(state.breakdown,'provider');assert.equal(state.expanded,false);
  assert.equal(state.query.dimension,'model');assert.equal(state.data(),report);
  assert(!state.group('provider'));assert(!state.group('session'));
  assert(state.group('project'));
  assert.equal(state.query.dimension,'project');assert.equal(state.page,2);assert.equal(state.query.before.sequence,position.sequence);
  assert.equal(state.data(),report);assert(!state.ready());
  assert(state.group('model'));assert.equal(state.query.dimension,'model');
});

test('summary estimates keep session totals',()=>{
  for(const locale of ['en','zh-CN','de','es','fr','ja','ko','pt-BR','ru','zh-TW']) {
    const text=messages(locale);
    const statistics={context_tokens:null,context_estimate:'120',context_limit:1000,
      usage:{input:'880',output:'10',cached_input:'100',reasoning:'0'},cost:{usd_micros:'300',responses:'1'},responses:'1'};
    const before=structuredClone(statistics);
    assert.equal(metrics({has_messages:true,statistics},text).find(value=>value.id==='composer_tokens').value,'890');
    assert.deepEqual(statistics,before);
  }
});

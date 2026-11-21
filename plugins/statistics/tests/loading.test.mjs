import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import {State,unit,DIMENSIONS} from '../dev.sailry.platform/desktop/state.js';
import {messages} from '../dev.sailry.platform/desktop/locales.js';
import {compact,grouped} from '../dev.sailry.platform/desktop/format.js';
import {heatmap,trend,SERIES} from '../dev.sailry.platform/desktop/charts.js';
import {overview,percent} from '../dev.sailry.platform/desktop/summary.js';
import {breakdown,annotated,VISIBLE,NAME_WIDTH} from '../dev.sailry.platform/desktop/breakdown.js';
import {formatRows} from '../dev.sailry.platform/desktop/requests.js';

const source=(await readFile(new URL('../dev.sailry.platform/desktop/view.js',import.meta.url),'utf8'))
  .replace(/^import .*;\n/gm,'').replace(/^export /gm,'');
const plain=value=>JSON.parse(JSON.stringify(value));

function fixture() {
  const nodes=new Map();
  function element(kind='div',id=null,props={}) {
    const value={kind,selector:id,props,items:[],style:{}};
    const proxy=new Proxy(value,{get(target,key) {
      if(key==='id')return id=>{target.selector=id;nodes.set(id,proxy);return proxy;};
      if(key==='child')return child=>{target.items.push(child);return proxy;};
      if(key==='children')return children=>{target.items.push(...children);return proxy;};
      if(key in target)return target[key];
      return (...args)=>{target.style[key]=args;return proxy;};
    }});
    if(id)nodes.set(id,proxy);
    return proxy;
  }
  const adapters=Object.fromEntries(['Header','SettingsGroup','DataTable','Heatmap','StackedChart','ProgressBar','Loading','SelectField']
    .map(kind=>[kind,{new:(id,props)=>element(kind,id,props)}]));
  const constructors=Object.fromEntries(['Button','Progress','Tab','TabBar','Tag','Pagination']
    .map(kind=>[kind,function(id){return element(kind,id);} ]));
  const {render}=vm.runInNewContext(`${source}\n({render})`,{
    div:()=>element(),...adapters,...constructors,compact,grouped,heatmap,trend,SERIES,overview,percent,breakdown,annotated,VISIBLE,NAME_WIDTH,formatRows,unit,DIMENSIONS,
    theme:()=>({colors:{muted_foreground:'muted'}}),window:{viewport_size:()=>({width:1440})},
  });
  const view={state:new State('captured'),text:messages('en'),error:null,header:()=>({title:'Usage'}),
    filters:()=>({range:{items:[{id:'30d',label:'30 days'}],selected:'30d'},selects:[{id:'usage-filter-project',label:'All projects',items:[],selected:'1:'}]})};
  return {view,nodes,render(){nodes.clear();return render(view);}};
}

test('initial reads keep filters without fabricated usage',()=>{
  const setup=fixture();
  for(const error of [null,'usage_read_failed']) {
    setup.view.error=error;setup.render();
    assert.equal(setup.nodes.has('usage-initial-loading'),false);
    assert.equal(setup.nodes.get('usage-content').items[0].selector,'usage-filters');
    assert.equal(setup.nodes.has('usage-summary'),false);
    assert.equal(setup.nodes.has('usage-breakdown'),false);
    assert.equal(setup.nodes.has('usage-requests'),false);
    assert.ok(setup.nodes.has('usage-header'));assert.ok(setup.nodes.has('usage-content'));
  }
});

test('partial host failure stays visible beside available totals',()=>{
  const setup=fixture(),{state}=setup.view;
  const report={totals:{responses:'0',tokens:null,cost:null,generation:null},days:[],recent:[],groups:[],requests:{items:[],has_more:false}};
  setup.view.sources=[{node:'local',label:'Laptop'},{node:'remote',label:'Server'}];
  state.accept({cursor:'1',all:true,view:{summary:report,complete:false,sources:[
    {node:'local',view:{connected:true,refreshing:false,report:{cursor:'1'}}},
    {node:'remote',view:{connected:false,refreshing:false,error:{code:'unavailable'}}},
  ]}},state.start());
  setup.render();
  assert.equal(setup.nodes.get('usage-partial').items[0],'Some hosts are out of date · Server');
  assert.ok(setup.nodes.has('usage-summary'));assert.ok(setup.nodes.has('usage-requests'));
});

test('filter refreshes retain report sections and avoid flow changes',()=>{
  const setup=fixture(),{state}=setup.view;
  const report={node:'captured',query:{dimension:'model'},totals:{responses:'0',tokens:null,cost:null,generation:null},days:[],recent:[],groups:[],requests:{items:[],has_more:false}};
  state.accept({cursor:'1',all:false,view:{report,connected:true,refreshing:false}},state.start());
  setup.render();
  const selectors=setup.nodes.get('usage-content').items.map(item=>item.selector);
  assert.deepEqual(selectors,['usage-filters','usage-summary','usage-trend-card','usage-breakdown-card','usage-requests-card']);
  state.select('project','selected');const generation=state.start();
  for(const observation of [null,{report:null,connected:true,refreshing:true},
    {report:null,connected:true,refreshing:false,error:{code:'unavailable'}}]) {
    if(observation)state.accept({cursor:observation.refreshing?'1':'2',all:false,view:observation},generation);
    setup.view.error=observation?.error?'usage_read_failed':null;setup.render();
    assert.equal(state.data(),report);
    const body=setup.nodes.get('usage-content');
    assert.deepEqual(body.items.filter(item=>item.selector).map(item=>item.selector),selectors);
    assert.equal(setup.nodes.has('usage-initial-loading'),false);
    // Retained reports refresh in place without a page-level progress bar.
    assert.equal(setup.nodes.has('usage-loading'),false);
  }
});

test('token legend is a two-by-two grid with reasoning unshared',()=>{
  const setup=fixture(),{state}=setup.view;
  const totals={responses:'2',tokens:{input:'1000',output:'200',cached_input:'600',reasoning:'50'},cost:null,generation:null};
  const report={node:'captured',query:{dimension:'model'},totals,days:[],recent:[],groups:[],requests:{items:[],has_more:false}};
  state.accept({cursor:'1',all:false,view:{report,connected:true,refreshing:false}},state.start());
  setup.render();
  const rows=setup.nodes.get('usage-parts').items;
  assert.equal(rows.length,2);
  assert.deepEqual(rows.map(row=>row.items.map(cell=>cell.selector)),
    [['usage-part-cache','usage-part-input'],['usage-part-output','usage-part-reasoning']]);
  const texts=id=>setup.nodes.get(id).items.slice(1).map(item=>item.items[0]);
  assert.deepEqual(texts('usage-part-cache'),['Cached','600','50.0%']);
  assert.deepEqual(texts('usage-part-input'),['Uncached','400','33.3%']);
  assert.deepEqual(texts('usage-part-output'),['Output','200','16.7%']);
  assert.deepEqual(texts('usage-part-reasoning'),['Reasoning','50']);
});

test('one-year ranges add the fitted heatmap below the trend',()=>{
  const setup=fixture(),{state}=setup.view;
  const report={node:'captured',query:{dimension:'model'},totals:{responses:'0',tokens:null,cost:null,generation:null},days:[],recent:[],groups:[],requests:{items:[],has_more:false}};
  state.accept({cursor:'1',all:false,view:{report,connected:true,refreshing:false}},state.start());
  setup.render();
  assert.ok(setup.nodes.has('usage-trend-day'));assert.equal(setup.nodes.has('usage-heatmap-plot'),false);
  state.range='1y';setup.render();
  assert.ok(setup.nodes.has('usage-trend-week'));
  assert.equal(setup.nodes.get('usage-heatmap-plot').kind,'Heatmap');
});

test('uses retained report offset while next query loads',()=>{
  const setup=fixture(),{state}=setup.view,point={start_ms:String(Date.UTC(2024,0,15,4)),models:{},
    metrics:{responses:'1',tokens:{input:'1',output:'0',cached_input:'0',reasoning:'0'}}};
  const report={node:'captured',query:{dimension:'model',utc_offset_minutes:-240},
    totals:{responses:'1',tokens:point.metrics.tokens,cost:null,generation:null},days:[point],recent:[],groups:[],requests:{items:[],has_more:false}};
  state.accept({cursor:'1',all:false,view:{report,connected:true,refreshing:false}},state.start());
  state.range='1y';assert.ok(state.select('project','next'));state.query.utc_offset_minutes=-600;
  setup.render();
  assert.equal(state.data(),report);
  assert.equal(setup.nodes.get('usage-trend-week').props.buckets[0].tooltip.heading,'2024-01-15');
  assert.equal(setup.nodes.get('usage-heatmap-plot').props.cells[0].tooltip.heading,'2024-01-15');
});

import assert from 'node:assert/strict';
import test from 'node:test';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';

const source = readFileSync(new URL('../dev.sailry.platform/document/charts.js',import.meta.url),'utf8');
const window = {},calls = [];
window.echarts = {init(_element,_theme,options) {calls.push(options);return {
  setOption(option) {calls.push(option);},renderToSVGString() {return '<svg></svg>';},dispose() {calls.push('dispose');},
};}};
vm.runInNewContext(source,{window,TextEncoder});
const {parse,option,render} = window.sailryCharts;
const palette = {primary:'primary',success:'success',warning:'warning',danger:'danger',foreground:'foreground',
  border:'border',muted_foreground:'muted_foreground',background:'background',font_family:'font_family'};
const point = {label:'A',value:1};

test('seven closed chart shapes retain the conversation contract',() => {
  for (const type of ['line','bar','area','pie','radar']) {
    const data = type === 'radar' ? [point,{label:'B',value:2},{label:'C',value:3}] : [point];
    assert.equal(parse(JSON.stringify({type,data})).type,type);
  }
  assert.equal(parse(JSON.stringify({type:'candlestick',data:[{label:'A',open:2,close:3,low:1,high:4}]})).type,'candlestick');
  assert.equal(parse(JSON.stringify({type:'sankey',nodes:[{id:'a',label:'A'},{id:'b',label:'B'}],links:[{source:'a',target:'b',value:1}]})).type,'sankey');
});
test('unknown options and unbounded or invalid values stay source content',() => {
  for (const value of [
    {type:'bar',data:[point],color:'red'}, {type:'bar',data:[{...point,extra:true}]},
    {type:'bar',data:[{label:'A',value:1e13}]}, {type:'pie',data:[{label:'A',value:0}]},
    {type:'radar',data:[point]}, {type:'candlestick',data:[{label:'A',open:2,close:3,low:4,high:5}]},
    {type:'bar',data:Array.from({length:501},() => point)},
  ]) assert.throws(() => parse(JSON.stringify(value)));
  assert.throws(() => parse(' '.repeat(65537)));assert.throws(() => parse(JSON.stringify({type:'bar',data:[{label:'中'.repeat(100),value:1}]})));
});
test('sankey rejects cycles and undefined nodes',() => {
  const nodes = [{id:'a',label:'A'},{id:'b',label:'B'}],links = [{source:'a',target:'b',value:1}];
  assert.throws(() => parse(JSON.stringify({type:'sankey',nodes,links:[...links,{source:'b',target:'a',value:1}]})));
  assert.throws(() => parse(JSON.stringify({type:'sankey',nodes,links:[{source:'a',target:'missing',value:1}]})));
});
test('all chart strokes and series use injected semantic colors',() => {
  const chart = parse(JSON.stringify({type:'line',title:'Trend',data:[point]})),value = option(chart,palette);
  assert.deepEqual(JSON.parse(JSON.stringify(value.color)),['primary','success','warning','danger','foreground']);
  assert.equal(value.xAxis.axisLabel.color,'muted_foreground');assert.equal(value.yAxis.splitLine.lineStyle.color,'border');
  assert.equal(value.title.textStyle.color,'foreground');assert.equal(value.series[0].lineStyle.width,3);
  assert.equal(render(JSON.stringify(chart),palette,10000),'<svg></svg>');
  assert.equal(calls[0].width,960);assert.equal(calls.at(-1),'dispose');assert.doesNotMatch(source,/#(?:[a-f0-9]{3,8})\b/i);
});
test('cartesian grids use both themed axes',() => {
  for (const type of ['line','bar','area','candlestick']) {
    const data = type === 'candlestick' ? [{label:'A',open:2,close:3,low:1,high:4}] : [point];
    const chart = parse(JSON.stringify({type,data})),value = option(chart,palette);
    for (const axis of [value.xAxis,value.yAxis]) {
      assert.equal(axis.splitLine.show,true,type);
      assert.equal(axis.splitLine.lineStyle.color,palette.border,type);
    }
  }
});

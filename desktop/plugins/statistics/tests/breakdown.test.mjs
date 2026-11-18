import test from 'node:test';
import assert from 'node:assert/strict';
import {breakdown,annotated,VISIBLE} from '../dev.sailry.platform/desktop/breakdown.js';

test('provider notes show only when every row fits its name and note',()=>{
  assert(annotated([{name:'gpt-5',note:'OpenAI'},{name:'Unassigned',note:null}]));
  assert(!annotated([{name:'gpt-5',note:'OpenAI'},{name:'a-model-name-far-too-long-for-the-column',note:'Provider'}]));
  assert(!annotated([{name:'通义千问旗舰版长上下文模型',note:'阿里云百炼平台'}]));
  assert(annotated([]));
});

const text={usage_unassigned:'Unassigned'};
const names={providers:new Map([['p1','Anthropic']]),projects:new Map([['proj','Web']])};
const metrics=(input,output,cached='0',cost)=>({responses:'2',tokens:{input,output,cached_input:cached,reasoning:'0'},...cost ? {cost} : {}});
const model=(provider,name,values,label)=>({key:{kind:'model',data:{provider,model:name}},metrics:values,...label ? {name:label} : {}});

test('models sort by tokens with provider notes and shares',()=>{
  const rows=breakdown([model('p1','small',metrics('10','10')),model('p2','big',metrics('60','20','30'),'OpenAI'),
    model('p1','none',{responses:'1',tokens:null})],'model',names,text);
  assert.deepEqual(rows.map(row=>row.name),['big','small','none']);
  assert.deepEqual(rows.map(row=>row.note),['OpenAI','Anthropic','Anthropic']);
  assert.equal(rows[0].total,80n);assert.equal(rows[0].share,80);assert.equal(rows[0].hit,50);
  assert.equal(rows[2].total,null);assert.equal(rows[2].share,null);assert.equal(rows[2].hit,null);
});

test('providers aggregate their models and mark partial costs',()=>{
  const rows=breakdown([model('p1','a',metrics('10','0','5',{usd_micros:'1000000',responses:'2'})),
    model('p1','b',metrics('30','0','0')),model('p2','c',metrics('1','0'))],'provider',names,text);
  assert.deepEqual(rows.map(row=>[row.id,row.name,row.total,row.responses]),[['p1','Anthropic',40n,4n],['p2','p2',1n,2n]]);
  assert.equal(rows[0].cost,'1.00+');assert.equal(rows[1].cost,null);
  assert.equal(rows[0].hit,12.5);
});

test('projects merge unowned conversations and ignore models',()=>{
  const rows=breakdown([{key:{kind:'project',data:'proj'},metrics:metrics('5','0')},
    {key:{kind:'session',data:'s1'},metrics:metrics('1','0')},{key:{kind:'session',data:'s2'},metrics:metrics('2','0')},
    {key:{kind:'connection',data:{id:'c'}},name:'Prod DB',metrics:metrics('4','0')},model('p1','a',metrics('9','0'))],'project',names,text);
  assert.deepEqual(rows.map(row=>[row.name,row.total]),[['Web',5n],['Prod DB',4n],['Unassigned',3n]]);
  assert.deepEqual(breakdown([{key:{kind:'project',data:'proj'},metrics:metrics('5','0')}],'model',names,text),[]);
});

test('ties sort by name and the first rows stay visible',()=>{
  const groups=Array.from({length:VISIBLE+2},(_,index)=>model('p1',`m${index}`,metrics('1','0')));
  const rows=breakdown(groups.reverse(),'model',names,text);
  assert.equal(VISIBLE,6);
  assert.deepEqual(rows.slice(0,VISIBLE).map(row=>row.name),['m0','m1','m2','m3','m4','m5']);
});

test('same resource UUIDs stay separate across Nodes',()=>{
  const catalog={providers:new Map([['a:p','Laptop model'],['b:p','Host model']]),
    projects:new Map([['a:proj','Local project'],['b:proj','Remote project']]),hosts:new Map([['a','Laptop'],['b','Server']])};
  const groups=[{node:'a',...model('p','same',metrics('10','0'))},{node:'b',...model('p','same',metrics('20','0'))}];
  assert.deepEqual(breakdown(groups,'provider',catalog,text).map(row=>[row.id,row.name,row.total]),
    [['b:p','Host model',20n],['a:p','Laptop model',10n]]);
  assert.deepEqual(breakdown(groups,'model',catalog,text).map(row=>row.note),['Host model · Server','Laptop model · Laptop']);
  const projects=breakdown(['a','b'].map(node=>({node,key:{kind:'project',data:'proj'},metrics:metrics('1','0')})),'project',catalog,text);
  assert.deepEqual(projects.map(row=>row.id),['a:project:proj','b:project:proj']);
});

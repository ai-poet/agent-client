import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import {review} from '../dev.sailry.platform/host/review.js';

async function load(sdk = {}) {
  const context = vm.createContext({});
  const cache = new Map();
  const names = ['readTurnState','stageTurnState','newId','isValueConflict'];
  const bridge = new vm.SyntheticModule(names,function() {
    for (const name of names) this.setExport(name, sdk[name] ?? (() => { throw new Error(`unexpected ${name}`); }));
  }, {context});
  const synthetic = values => new vm.SyntheticModule(Object.keys(values),function() {
    for (const [name,value] of Object.entries(values)) this.setExport(name,value);
  },{context});
  const unexpected = name => sdk[name] ?? (()=>{throw new Error(`unexpected ${name}`);});
  const services = new Map([
    ['settings.js',synthetic({readMemorySettings:unexpected('readMemorySettings')})],
    ['retrieval.js',synthetic({searchMemories:unexpected('searchMemories'),
      reviewMemories:async(settings,filter)=>review(await unexpected('listMemories')(),settings,filter,unexpected('readMemory'))})],
    ['storage.js',synthetic({readMemory:unexpected('readMemory'),snapshotChanged:()=>false,
      stable:read=>read(),head:async()=>({record:{revision:'1'}}),unchanged:async()=>{}})],
    ['mutations.js',synthetic({putOperations:(entry,expected_revision)=>({entry,expected_revision}),
      removeOperations:(id,expected_revision)=>({id,expected_revision}),mergeOperations:(entry,sources)=>({entry,sources}),
      memoryOutput:output=>({kind:'memory_results',data:output.data.map(value=>value.data.value.summary)}),RecordChanged:class extends Error{},EquivalentMemory:class extends Error{}})],
  ]);
  async function module(path) {
    if (path === 'sailry/sdk') return bridge;
    if (services.has(path)) return services.get(path);
    if (cache.has(path)) return cache.get(path);
    const source = await readFile(new URL(`../dev.sailry.platform/host/${path}`,import.meta.url),'utf8');
    const value = new vm.SourceTextModule(source,{context,identifier:path});
    cache.set(path,value);
    await value.link(name => module(name.replace('./','')));
    return value;
  }
  const main = await module('main.js');
  await main.evaluate();
  return {main:main.namespace,policy:cache.get('policy.js').namespace};
}
const id = '11111111-1111-4111-8111-111111111111';
const project = '22222222-2222-4222-8222-222222222222';
const settings = {enabled:true,auto_write:true,context_bytes:4096,review_after_days:90};
const summary = {id,project,title:'Feedback',kind:'feedback',revision:1,updated_at_ms:0,archived:false};
const plain = value => JSON.parse(JSON.stringify(value));

const hit = (index,body,changes={}) => ({summary:{...summary,id:`hit-${index}`,title:`Hit ${index}`,...changes},body});
const tag = 'Relevant memory: ';
const bodies = parts => plain(Array.from(parts).filter(part=>part.startsWith(tag)).map(part=>JSON.parse(part.slice(tag.length))));

test('turns one relevant body into one part',async()=>{
  const {policy}=await load();
  const parts=policy.turnParts({relevant:[{summary,body:'Use tabs'}],message:'Format Go files',settings,write:true});
  assert.equal(parts.length,1);
  assert.ok(parts[0].startsWith(tag));
  assert.deepEqual(bodies(parts),[{id,title:'Feedback',kind:'feedback',global:false,body:'Use tabs'}]);
  const global=policy.turnParts({relevant:[{summary:{...summary,project:null},body:'Use tabs'}],message:'Format',settings,write:true});
  assert.equal(bodies(global)[0].global,true);
  assert.deepEqual(plain(policy.turnParts({relevant:[],message:'Format Go files',settings,write:true})),[]);
});

test('keeps at most three bodies that fit half the budget',async()=>{
  const {policy}=await load();
  const bounded={...settings,context_bytes:2048};
  const relevant=[hit(0,'Fact 0'),hit(1,'中文🙂'.repeat(400)),hit(2,'Fact 2'),hit(3,'Fact 3'),hit(4,'Fact 4'),hit(5,'Fact 5')];
  const parts=policy.turnParts({relevant,message:'Format',settings:bounded,write:true});
  assert.deepEqual(bodies(parts).map(value=>value.id),['hit-0','hit-2','hit-3']);
  assert.ok(parts.reduce((size,part)=>size+Buffer.byteLength(part),0)<=bounded.context_bytes/2);
  assert.equal(policy.turnParts({relevant:relevant.slice(1,2),message:'Format',settings:bounded,write:true}).length,0);
  const big=hit(0,'a'.repeat(900)), size=Buffer.byteLength(policy.relevantParts([big],Infinity)[0]);
  assert.ok(size <= bounded.context_bytes/2 && size+Buffer.byteLength(policy.cue) > bounded.context_bytes/2);
  assert.deepEqual(bodies(policy.turnParts({relevant:[big],message:'Format',settings:bounded,write:true})).map(value=>value.id),['hit-0']);
  assert.deepEqual(plain(policy.turnParts({relevant:[big],message:'Always format',settings:bounded,write:true})),[policy.cue],'the cue shares the half budget');
  assert.equal(policy.relevantLimit,3);
});

test('adds the save cue only for cue messages when writing is allowed',async()=>{
  const {policy}=await load();
  const cued=['Always use tabs','never commit generated files',"Don't use semicolons",'I prefer short names','Please remember my editor',
    'From now on answer in Chinese','\u4e0d\u8981\u5220\u9664\u6d4b\u8bd5','\u4ee5\u540e\u7528\u4e2d\u6587\u56de\u7b54','\u8bb0\u4f4f\u6211\u7684\u5e38\u7528\u8bbe\u7f6e','\u522b\u518d\u6dfb\u52a0\u6ce8\u91ca'];
  for (const message of cued) {
    assert.deepEqual(plain(policy.turnParts({message,settings,write:true})),[policy.cue],message);
    assert.deepEqual(plain(policy.turnParts({message,settings,write:false})),[],message);
  }
  for (const message of ['Format the Go files','Explain this stack trace','\u89e3\u91ca\u8fd9\u6bb5\u4ee3\u7801','',null,undefined]) {
    assert.deepEqual(plain(policy.turnParts({message,settings,write:true})),[],String(message));
  }
  assert.equal(policy.cue,'If this states a lasting preference or correction, save it with save_memory.');
  const both=policy.turnParts({relevant:[{summary,body:'Use tabs'}],message:'Always use tabs',settings,write:true});
  assert.deepEqual([both.length,both.at(-1)],[2,policy.cue]);
});

test('filters tools at initialization and returns no instruction',async () => {
  let state;
  let current = {...settings,auto_write:false};
  const {main} = await load({readMemorySettings:async()=>current,stageTurnState:value=>state=plain(value),readTurnState:()=>plain(state)});
  const tools = ['search_memory','review_memories','save_memory','forget_memory','consolidate_memories'];
  let initialized = await main.initialize({tools,project,resources:true});
  assert.deepEqual(plain(initialized),{instruction:'',parts:[],tools:['search_memory','review_memories']});
  assert.equal(state.remaining,settings.context_bytes);
  initialized = await main.initialize({tools:['search_memory'],project,resources:false,message:'Always use tabs'});
  assert.deepEqual(plain(initialized),{instruction:'',parts:[],tools:['search_memory']});
  current = {...current,enabled:false};
  initialized = await main.initialize({tools,project,resources:true,message:'Always use tabs'});
  assert.deepEqual(plain(initialized),{instruction:'',parts:[],tools:[]});
});

test('turns without a hit or cue add no context or reads',async()=>{
  let state;
  const calls=[];
  const {main}=await load({
    readMemorySettings:async()=>{calls.push('settings');return settings;},
    searchMemories:async()=>{calls.push('search');return [];},
    stageTurnState:value=>state=plain(value),readTurnState:()=>plain(state),
  });
  const tools=['search_memory','save_memory'];
  for (const message of [null,undefined,'  ']) {
    const initialized=await main.initialize({tools,project,resources:true,message});
    assert.deepEqual(plain(initialized),{instruction:'',parts:[],tools});
  }
  assert.deepEqual(calls,['settings','settings','settings']);
  const searched=await main.initialize({tools,project,resources:true,message:'Format the Go files'});
  assert.deepEqual(plain(searched),{instruction:'',parts:[],tools});
  assert.deepEqual(calls.slice(3),['settings','search']);
  await main.beforeModel({results:[]});
  assert.deepEqual(state.reads,[]);
  assert.equal(state.remaining,settings.context_bytes);
});

test('injects matched bodies and the cue without counting recalls',async()=>{
  let state;
  const queries=[];
  const matched={summary,body:'Prefer tabs in Go files'};
  const {main}=await load({readMemorySettings:async()=>settings,
    searchMemories:async(scope,query,options)=>{queries.push([scope,query,options]);return query==='Always format the Go files' ? [matched] : [];},
    readMemory:async()=>structuredClone(matched),
    stageTurnState:value=>state=plain(value),readTurnState:()=>plain(state),newId:()=>id});
  const initialized=await main.initialize({tools:['search_memory','save_memory'],project,resources:true,message:'Always format the Go files',mode:'code'});
  assert.deepEqual(plain(queries),[[project,'Always format the Go files',{keyword:true}]]);
  assert.equal(initialized.instruction,'');
  assert.equal(initialized.parts.length,2);
  assert.deepEqual(bodies(initialized.parts),[{id,title:'Feedback',kind:'feedback',global:false,body:'Prefer tabs in Go files'}]);
  assert.equal(initialized.parts.at(-1),'If this states a lasting preference or correction, save it with save_memory.');
  assert.equal(state.remaining,settings.context_bytes-initialized.parts.reduce((size,part)=>size+Buffer.byteLength(part),0));
  assert.deepEqual(state.reads,[],'injected bodies grant no read authority');
  const denied=await main.save({arguments:{id,expected_revision:1,kind:'feedback',title:'Formatting',body:'Prefer tabs in Go files and Makefiles'},step:0});
  assert.equal(denied.result.error.code,'invalid_request','updates still require a search result');
  const searched=await main.search({query:'tests'});
  await main.beforeModel({results:[{name:'search_memory',response:searched}]});
  assert.deepEqual(Object.keys(state).sort(),['mode','project','reads','remaining','settings']);
  const planning=await main.initialize({tools:['search_memory','save_memory'],project,resources:true,message:'Always format the Go files',mode:'plan'});
  assert.deepEqual(bodies(planning.parts).map(value=>value.id),[id]);
  assert.ok(!planning.parts.includes('If this states a lasting preference or correction, save it with save_memory.'),'planning turns reject writes');
});

test('search, replacement, scope policy and live revocation share one captured turn state',async () => {
  let state;
  let current = {...settings};
  const original = {summary,body:'Existing complete body'};
  const {main} = await load({readMemorySettings:async()=>current,listMemories:async()=>[summary],searchMemories:async()=>[original],readMemory:async()=>structuredClone(original),stageTurnState:value=>state=plain(value),readTurnState:()=>plain(state),newId:()=>id});
  await main.initialize({tools:['search_memory','save_memory'],project,resources:true});
  const input = {id,expected_revision:1,kind:'feedback',title:'Correction',body:'Updated with conditions'};
  const save=value=>main.save({arguments:value,step:0});
  assert.equal((await save(input)).result.error.code,'invalid_request');
  const retrieved = await main.search({query:id});
  main.beforeModel({results:[{name:'search_memory',response:retrieved}]});
  const prepared = await save(input);
  assert.equal(prepared.call.arguments.entry.summary.project,project);
  assert.equal(prepared.call.arguments.entry.body,input.body);
  main.beforeModel({results:[]});
  assert.equal((await save(input)).result.error.code,'invalid_request');
  const create = {expected_revision:0,global:true,kind:'project',title:'Invalid global fact',body:'A project fact'};
  assert.equal((await save(create)).result.error.code,'invalid_request');
  current.auto_write = false;
  assert.equal((await save({...create,kind:'user'})).result.error.code,'permission_denied');
});

test('uncaptured mutations preserve fields and authoritative faults',async () => {
  const {main} = await load();
  const output = {kind:'plugin_transaction',data:[{kind:'plugin_value',data:{value:{summary}}}]};
  assert.deepEqual(plain(main.result({output})),{updated:[summary]});
  const failure = {isError:true,error:{code:'outcome_unknown',message:'Uncertain'}};
  assert.equal(main.result({output:failure}),failure);
});

test('package review shares the captured content budget and never grants replacement authority',async () => {
  let state;
  const read = [];
  const entries = [{summary,body:'Use concise Chinese replies'},
    {summary:{...summary,id:'older',updated_at_ms:1},body:'Use concise Chinese replies today'},
    {summary:{...summary,id:'foreign',project:'elsewhere'},body:'Unavailable'}];
  const {main} = await load({readMemorySettings:async()=>settings,listMemories:async()=>entries.map(entry=>entry.summary),
    readMemory:async id=>{read.push(id);return entries.find(entry=>entry.summary.id===id);},
    stageTurnState:value=>state=plain(value),readTurnState:()=>plain(state)});
  await main.initialize({tools:['review_memories'],project,resources:false});
  const output = await main.review({});
  assert.equal(output.candidates.length,2);
  assert.ok(output.candidates.some(candidate=>candidate.duplicate_of==='older'));
  assert.ok(!read.includes('foreign'));
  assert.ok(state.remaining < settings.context_bytes);
  assert.ok(!JSON.stringify(output).includes('Use concise'));
  main.beforeModel({results:[{name:'review_memories',response:output}]});
  assert.deepEqual(state.reads,[]);
  state.remaining = 1;
  assert.deepEqual(plain(await main.review({})),{candidates:[]});
});


test('the fixed policy is the manifest instructions, not turn output',async () => {
  const manifest = JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  const extension = manifest.extensions['dev.sailry.platform'];
  const {policy,main}=await load({readMemorySettings:async()=>settings,searchMemories:async()=>[{summary,body:'Use tabs'}],stageTurnState:()=>{}});
  assert.equal(extension.instructions,'Memory holds lasting user preferences, corrections and decisions. Use relevant memories from context or search_memory when they apply; current instructions win. When save_memory is available, save a lasting preference, correction or decision the user states, or anything they ask you to remember. Do not save secrets, task progress or facts available in code, Git or project docs.');
  assert.ok(Buffer.byteLength(extension.instructions)<=500);
  assert.equal(policy.policy,undefined);
  assert.equal(policy.instruction,undefined);
  const initialized=await main.initialize({tools:['search_memory','save_memory'],project,resources:true,message:'Always use tabs',mode:'code'});
  assert.equal(initialized.instruction,'');
  assert.ok(initialized.parts.every(part=>!part.includes('Memory holds lasting')&&!part.includes('Do not save secrets')));
  assert.ok(extension.actions.includes('conversation.message.read'));
  assert.deepEqual(extension.host.turn,{initialize:'initialize',before_model:'beforeModel'});
  const initial=extension.tools.filter(tool=>tool.initial).map(tool=>tool.name);
  assert.deepEqual(initial,['search_memory','save_memory']);
  for (const tool of extension.tools) {
    assert.ok(tool.description.length<=400,tool.name);
    assert.doesNotMatch(tool.description,/receipt|Claim saved|Search first/);
  }
  const save=extension.tools.find(tool=>tool.name==='save_memory');
  assert.deepEqual(save.handler.parameters.required,['kind','title','body']);
  assert.match(save.description,/Omit id to create/);
});

test('recall counters and their storage are gone',async () => {
  const manifest = JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  const extension = manifest.extensions['dev.sailry.platform'];
  assert.equal(extension.storage.collections.some(collection=>`${collection.key ?? collection.prefix}`.startsWith('memory/recall')),false);
  for (const resources of [extension.host.resources,extension.desktop.resources]) {
    assert.ok(resources.every(path=>!path.endsWith('recall.js')));
  }
  await assert.rejects(readFile(new URL('../dev.sailry.platform/host/recall.js',import.meta.url)),{code:'ENOENT'});
});

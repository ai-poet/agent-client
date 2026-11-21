import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {fixture,entry,id,project} from './store.mjs';

const settings={enabled:true,auto_write:true,context_bytes:8192,review_after_days:90};
const cue='If this states a lasting preference or correction, save it with save_memory.';
const tag='Relevant memory: ';
const tools=['search_memory','save_memory'];
const bodies=parts=>Array.from(parts).filter(part=>part.startsWith(tag)).map(part=>JSON.parse(part.slice(tag.length)));
async function setup(entries,changes={}) {
  const f=await fixture();
  f.configure({...settings,...changes});
  f.seed(entries);
  return {f,turn:(message,mode='code')=>f.main.initialize({tools,project,resources:true,message,mode})};
}
const notes=count=>Array.from({length:count},(_,index)=>entry(index+1,`Parser note ${index+1}`,{title:`Parser ${index+1}`}));

test('a turn without a match adds no context',async()=>{
  const {f,turn}=await setup(notes(3));
  for (const message of ['Explain the stack trace','Format the Go files',null]) {
    const result=await turn(message);
    assert.deepEqual(Array.from(result.parts),[],String(message));
    assert.equal(result.instruction,'');
    assert.deepEqual(Array.from(result.tools),tools);
  }
  assert.equal(f.prepared.length,0);
  assert.equal(f.completed.length,0);
});

test('returns at most three matched bodies, one part each',async()=>{
  const {turn}=await setup(notes(5));
  const result=await turn('How does the parser work?');
  assert.equal(result.instruction,'');
  assert.equal(result.parts.length,3);
  assert.deepEqual(bodies(result.parts).map(value=>value.id),[id(1),id(2),id(3)]);
  assert.ok(result.parts.every(part=>part.startsWith(tag) && !part.includes('\n')));
  assert.ok(result.parts.every(part=>!part.includes('Memory holds lasting')));
  const [first]=bodies(result.parts);
  assert.deepEqual(Object.keys(first),['id','title','kind','global','body']);
  assert.equal(first.global,false);
  assert.deepEqual(Array.from((await turn('How does the parser work?')).parts),Array.from(result.parts),'the same memories give the same bytes');
});

test('adds the cue only for cue messages while writing is allowed',async()=>{
  const {turn}=await setup(notes(1));
  assert.deepEqual(Array.from((await turn('Always explain the stack trace')).parts),[cue]);
  assert.deepEqual(Array.from((await turn('以后都用中文回答')).parts),[cue]);
  assert.deepEqual(Array.from((await turn('Explain the stack trace')).parts),[]);
  const matched=await turn('Never change the parser');
  assert.equal(matched.parts.length,2);
  assert.equal(matched.parts.at(-1),cue);
  assert.equal(bodies(matched.parts).length,1);
  assert.deepEqual(Array.from((await turn('Always explain the stack trace','plan')).parts),[],'planning turns reject writes');
  const {turn:readOnly}=await setup(notes(1),{auto_write:false});
  assert.deepEqual(Array.from((await readOnly('Always explain the stack trace')).parts),[]);
  assert.deepEqual(bodies((await readOnly('Parser')).parts).map(value=>value.id),[id(1)],'read-only turns still recall bodies');
  const {turn:disabled}=await setup(notes(1),{enabled:false});
  const off=await disabled('Always explain the parser');
  assert.deepEqual([Array.from(off.parts),Array.from(off.tools)],[[],[]]);
});

test('the policy is manifest instructions, never turn output',async()=>{
  const manifest=JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  const extension=manifest.extensions['dev.sailry.platform'];
  assert.match(extension.instructions,/^Memory holds lasting user preferences, corrections and decisions\./);
  assert.match(extension.instructions,/save_memory/);
  assert.match(extension.instructions,/Do not save secrets, task progress or facts available in code, Git or project docs\./);
  const {turn}=await setup(notes(2));
  const result=await turn('Always use the parser');
  const output=JSON.stringify(result);
  assert.equal(result.instruction,'');
  for (const phrase of ['Memory holds lasting','Do not save secrets','Memory is read-only','Memories (title','Other memories']) {
    assert.equal(output.includes(phrase),false,phrase);
  }
});

test('stays within the Node part limits at the largest budget',async()=>{
  const body=index=>`parser ${'word '.repeat(2400)}${index}`;
  const {turn}=await setup(Array.from({length:6},(_,index)=>entry(index+1,body(index+1))),{context_bytes:65536});
  const result=await turn('Always use the parser');
  const total=result.parts.reduce((size,part)=>size+Buffer.byteLength(part),0);
  assert.ok(result.parts.length <= 8);
  assert.ok(total <= 32*1024,String(total));
  assert.ok(bodies(result.parts).length <= 3);
  assert.equal(result.parts.at(-1),cue);
  assert.ok(result.parts.every(part=>part.trim().length>0));
});

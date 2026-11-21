import test from 'node:test';
import assert from 'node:assert/strict';
import {Surfaces,patch} from '../dev.sailry.platform/desktop/surfaces.js';

function fixture() {
  const calls = [];
  const make = (kind,text) => { calls.push([kind,text]); return {kind,text}; };
  return {surfaces:new Surfaces(text=>make('diff',text),text=>make('source',text)),calls};
}
function document(id,text,kind='file') { return {id,request:{kind},files:[{path:'a.txt',text}],text}; }

test('keeps stable states and replaces changed text',()=>{
  const {surfaces,calls}=fixture(), docs=new Map([['a',document('a','@@ -1 +1 @@\n-old\n+new\n')]]);
  surfaces.sync(docs); const initial=surfaces.get('git-diff-a');
  surfaces.sync(docs); assert.equal(surfaces.get('git-diff-a'),initial); assert.equal(calls.length,1);
  docs.set('a',document('a','@@ -1 +1 @@\n-old\n+latest\n'));
  surfaces.sync(docs); const changed=surfaces.get('git-diff-a');
  assert.notEqual(changed,initial);assert.equal(changed.kind,'diff');assert.equal(calls.length,2);
});

test('releases closed captures and isolates per-file states',()=>{
  const {surfaces}=fixture(),docs=new Map([['commit',document('commit','@@ -1 +1 @@\n-old\n+new\n','commit')]]);
  docs.get('commit').files.push({path:'b.txt',text:'@@ -1 +1 @@\n-before\n+after\n'});
  surfaces.sync(docs);
  const first=surfaces.get('git-diff-commit/a.txt'),second=surfaces.get('git-diff-commit/b.txt');
  assert.notEqual(first,second); docs.clear(); surfaces.sync(docs);
  assert.equal(surfaces.get('git-diff-commit/a.txt'),undefined); assert.equal(surfaces.states.size,0);
  surfaces.sync(new Map([['output',document('output','captured log','output')]]));
  assert.equal(surfaces.get('git-output-output').kind,'source'); surfaces.clear(); assert.equal(surfaces.states.size,0);
});

test('source refresh replaces changed editor state',()=>{
  const {surfaces,calls}=fixture(),docs=new Map([['output',document('output','first','output')]]);
  surfaces.sync(docs);const state=surfaces.get('git-output-output');
  docs.set('output',document('output','second','output'));surfaces.sync(docs);
  assert.notEqual(surfaces.get('git-output-output'),state);
  assert.deepEqual(calls,[['source','first'],['source','second']]);
});

test('normalizes only declared truncated hunks',()=>{
  const text='@@ -30,20 +40,20 @@\n-old\n+new\n same\n';
  const file={path:'nested/一 two.txt',text};
  assert(patch(file).endsWith(text));
  const partial=patch({...file,truncated:true});
  assert(partial.includes('@@ -30,2 +40,2 @@\n-old\n+new\n same'));
  assert(partial.startsWith('--- "a/nested/一 two.txt"\n+++ "b/nested/一 two.txt"\n'));
  const metadata='diff --git a/p b/p\nBinary files a/p and b/p differ\n';
  assert.equal(patch({path:'p',text:metadata}),metadata);
});

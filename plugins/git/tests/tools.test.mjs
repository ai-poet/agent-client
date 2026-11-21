import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {host} from './host.mjs';
import {cases} from './parameters.mjs';

const {status,diff,log,show,blame,changes,change}=await host({context:()=>({}),inspectGit:async()=>{}});

test('history preserves first-page defaults and exact continuation', () => {
  assert.deepEqual(log({}),{worktree:null,limit:20,cursor:null,revision:null,path:null});
  assert.deepEqual(log({limit:5,cursor:null}),{worktree:null,limit:5,cursor:null,revision:null,path:null});
  const cursor = {head:'a'.repeat(40),after:'b'.repeat(40),offset:20};
  assert.deepEqual(log({cursor,worktree:'captured-project-tree'}),
    {worktree:'captured-project-tree',limit:20,cursor,revision:null,path:null});
  assert.equal(log({cursor}).cursor,cursor);
  const filtered={...cursor,path:'src/资料.rs'};
  assert.deepEqual(log({revision:'feature',path:filtered.path,cursor:filtered}),
    {worktree:null,limit:20,cursor:filtered,revision:'feature',path:filtered.path});
  for (const invalid of ['HEAD',{...cursor,head:'abcdef'},
    {head:cursor.head,offset:5},{...cursor,after:'short'},
    {head:cursor.head,after:cursor.after},{...cursor,path:5}]) {
    assert.equal(log({cursor:invalid}).isError,true);
  }
  for (const offset of [-1,1.5,null,'20',Number.MAX_SAFE_INTEGER+1])
    assert.equal(log({cursor:{...cursor,offset}}).isError,true);
  assert.deepEqual(log({cursor:{...cursor,offset:0}}).cursor,{...cursor,offset:0});
});

test('history schema declares filtered commit continuations', async () => {
  const manifest = JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  const extension = manifest.extensions['dev.sailry.platform'];
  const tool = extension.tools.find(tool => tool.name === 'git_log');
  const properties = tool.handler.parameters.properties;
  assert.deepEqual(properties.cursor.required,['head','after','offset']);
  assert.deepEqual(properties.cursor.properties.offset,{type:'integer',minimum:0});
  assert.deepEqual(properties.cursor.properties.path.type,['string','null']);
  assert.deepEqual(properties.revision.type,['string','null']);
  assert.deepEqual(properties.path.type,['string','null']);
  assert.match(tool.description,/keep the same path filter/);
  assert.equal(extension.min_sailry_version,'0.1.6');
});

test('status and diff preserve explicit registered target and scope', () => {
  assert.deepEqual(status({}),{worktree:null});
  assert.deepEqual(status({worktree:'tree'}),{worktree:'tree'});
  assert.deepEqual(diff({path:'src/资料.rs'}),{worktree:null,path:'src/资料.rs',scope:'all'});
  assert.deepEqual(diff({path:'file',scope:'staged',worktree:'tree'}),
    {worktree:'tree',path:'file',scope:'staged'});
  for (const args of [{path:'file',scope:'both'},{path:'file',scope:null},{path:8}]) {
    assert.equal(diff(args).isError,true);
  }
});

test('arguments cannot inject project or execution commands', () => {
  assert.equal(status({project:'other'}).isError,true);
  assert.equal(diff({path:'file',command:'checkout'}).isError,true);
  for (const args of [{limit:0},{limit:1.5},{limit:null},{worktree:7},
    {revision:8},{path:[]},{revision:'main',command:'checkout'}]) {
    assert.equal(log(args).isError,true);
  }
});

test('show and blame preserve literal paths and immutable continuation', () => {
  assert.deepEqual(show({}), {worktree:null,revision:'HEAD',path:null,offset:0,limit:2000});
  assert.deepEqual(show({path:'-quoted[1].rs',revision:'HEAD~1',offset:8,limit:2}),
    {worktree:null,revision:'HEAD~1',path:'-quoted[1].rs',offset:8,limit:2});
  assert.deepEqual(blame({path:'资料.rs'}),
    {worktree:null,revision:null,path:'资料.rs',offset:0,limit:200});
  for (const handler of [show,blame]) {
    for (const args of [{path:'file',offset:-1},{path:'file',limit:0},
      {path:'file',offset:Number.MAX_SAFE_INTEGER},{path:'file',project:'outside'},
      {path:'file',revision:3},{path:2}]) {
      assert.equal(handler(args).isError,true);
    }
  }
  assert.equal(show({offset:1}).isError,true);
  assert.equal(blame({}).isError,true);
});

test('page requests preserve intent and report invalid fields', async () => {
  for (const handler of [show,blame]) {
    const args = {path:'src/main.rs',revision:'HEAD',offset:0,limit:4000};
    assert.deepEqual(handler(args),{...args,worktree:null});
    const error = handler({...args,limit:1.5});
    assert.equal(error.isError,true);
    assert.equal(error.error.code,'invalid_request');
    assert.equal(error.error.message,'limit must be a positive safe integer');
    assert.equal(args.limit,4000);
    assert.match(handler({path:'file',offset:-1}).error.message,/offset.*non-negative/);
  }
  const manifest = JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  const declaration = manifest.extensions['dev.sailry.platform'].tools.find(tool => tool.name === 'git_show');
  assert.equal(declaration.handler.parameters.properties.limit.maximum,undefined);
  assert.match(declaration.handler.parameters.properties.limit.description,/independent output budget/);
  assert.deepEqual(show({path:'apps/desktop/src/sidebar.rs',revision:'HEAD',offset:0,limit:1200}),
    {worktree:null,path:'apps/desktop/src/sidebar.rs',revision:'HEAD',offset:0,limit:1200});
});

test('change inventory keeps frozen cursors and explicit target alternatives', () => {
  const target = {kind:'worktree',data:{scope:'all'}};
  assert.deepEqual(changes({}), {worktree:null,target,cursor:null,limit:200});
  const snapshot = {target:{kind:'commit',data:{revision:'a'.repeat(40),parent:null}},metadata_revision:'b'.repeat(64)};
  for (const after_path of [null,'src/file.rs']) {
    const cursor = {snapshot,after_path};
    assert.deepEqual(changes({cursor}), {worktree:null,target:null,cursor,limit:200});
    assert.equal(changes({target,cursor}).isError,true);
  }
  for (const args of [{target:null},{limit:0},{limit:1.5},{project:'outside'},{cursor:{snapshot,after_path:3}}]) {
    assert.equal(changes(args).isError,true);
  }
});

test('mapped changes require strong revision on continuation', () => {
  const snapshot = {target:{kind:'range',data:{base:'a'.repeat(40),head:'b'.repeat(40)}},metadata_revision:'c'.repeat(64)};
  const initial = {snapshot,path:'-file[1].rs',expected_metadata_revision:'d'.repeat(64)};
  assert.deepEqual(change(initial), {...initial,worktree:null,expected_revision:null,offset:0,limit:200});
  assert.equal(change({...initial,offset:1}).isError,true);
  assert.deepEqual(change({...initial,expected_revision:'e'.repeat(64),offset:10}),
    {...initial,worktree:null,expected_revision:'e'.repeat(64),offset:10,limit:200});
  for (const args of [{...initial,limit:0},{...initial,command:'checkout'},{...initial,offset:-1}]) {
    assert.equal(change(args).isError,true);
  }
  for (const field of ['snapshot','path','expected_metadata_revision']) {
    const args = {...initial,expected_revision:'e'.repeat(64),offset:0};
    delete args[field];
    assert.equal(change(args).error.code,'invalid_request');
  }
});

test('file change descriptions distinguish captured revision fields', async () => {
  const manifest = JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  const extension = manifest.extensions['dev.sailry.platform'];
  const tool = extension.tools.find(tool => tool.name === 'read_git_change');
  assert.equal(extension.api_version,'v1');
  assert.deepEqual(tool.handler.parameters.required,['snapshot','path','expected_metadata_revision']);
  assert.match(tool.description,/Every request requires the unchanged snapshot/);
  const fields = tool.handler.parameters.properties;
  assert.match(fields.snapshot.description,/pass it unchanged on every page/);
  assert.equal(fields.snapshot.type,'object');
  assert.equal(fields.snapshot.properties,undefined);
  const inventory = extension.tools.find(tool => tool.name === 'list_git_changes').handler.parameters.properties;
  assert.deepEqual(inventory.target.type,['object','null']);
  assert.equal(inventory.target.anyOf,undefined);
  const cursor = inventory.cursor.anyOf.find(value => value.type === 'object');
  assert.deepEqual(cursor.required,['snapshot','after_path']);
  assert.deepEqual(cursor.properties.snapshot,fields.snapshot);
  assert.match(fields.path.description,/required on every page/);
  assert.match(fields.expected_metadata_revision.description,/file entry's metadata_revision, not snapshot.metadata_revision/);
  assert.match(fields.expected_revision.description,/Strong revision of this file/);
});

test('all read adapters retain larger page requests', () => {
  const handlers = {log,show,blame,changes,change};
  for (const entry of cases()) {
    const args = structuredClone(entry.input), original = structuredClone(args);
    const output = handlers[entry.handler](args);
    assert.deepEqual(args,original);
    if (entry.valid) assert.deepEqual(output,entry.expected);
    else {
      assert.equal(output.isError,true);
      assert.equal(output.error.code,'invalid_request');
    }
  }
});

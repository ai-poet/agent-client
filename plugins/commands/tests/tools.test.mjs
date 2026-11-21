import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {run,read,write} from '../dev.sailry.platform/host/main.js';

test('reads exact command IDs or lists without an ID', () => {
  const manifest = JSON.parse(readFileSync(new URL('../plugin.json', import.meta.url)));
  const declaration = manifest.extensions['dev.sailry.platform'].tools
    .find(tool => tool.name === 'read_command');
  const field = declaration.handler.parameters.properties.id;
  assert.deepEqual(field.type, ['string', 'null']);
  assert.deepEqual(declaration.handler.parameters.required, []);
  assert.match(declaration.description, /data.id by run_command with background=true/);
  assert.match(declaration.description, /does not start a process or read a file directly/);
  assert.match(declaration.description, /Never substitute a PID, filename, command text or invented ID/);
  assert.match(declaration.description, /Use run_command to start a new command/);
  assert.match(field.description, /omit or null to list commands/);
  const pattern = new RegExp(field.pattern);
  for(const id of ['00000000-0000-0000-0000-000000000000',
    '11111111-1111-4111-8111-111111111111', 'AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE']) {
    assert(pattern.test(id));
    assert.deepEqual(read({id}), {id});
  }
  for(const id of ['bogus', '123', 'file.txt', 'cat /etc/hosts']) assert(!pattern.test(id));
  assert.deepEqual(read({}), {});
  assert.deepEqual(read({id:null}), {id:null});
  for(const args of [{id:3},{id:false},{id:[]},{id:null,session:'other'}]) {
    assert.equal(read(args).error.code, 'invalid_request');
  }
});

test('pages retained streams and canonical tool results', () => {
  const output = {call:'call-1', revision:'f'.repeat(64)};
  for (const args of [
    {output,offset:0,limit:32768}, {output,stream:'stderr',offset:32768,limit:4096},
    {id:'11111111-1111-4111-8111-111111111111',stream:'stdout',offset:0,limit:1},
  ]) assert.deepEqual(read(args),args);
  for (const args of [
    {output:{...output,revision:'bad'}},{output:{...output,call:''}},
    {id:'command',output},{offset:0},{stream:'stdout'},{output,limit:0},
    {output,offset:-1},{output,limit:32769},{output,stream:'combined'},
  ]) assert.equal(read(args).isError,true);
});

test('waits for output with optional text and paging', () => {
  const manifest = JSON.parse(readFileSync(new URL('../plugin.json', import.meta.url)));
  const declaration = manifest.extensions['dev.sailry.platform'].tools
    .find(tool => tool.name === 'read_command');
  const fields = declaration.handler.parameters.properties;
  assert.equal(fields.wait_ms.type, 'integer');
  assert.equal(fields.wait_ms.minimum, 0);
  assert.equal(fields.wait_ms.maximum, 30000);
  assert.equal(fields.until.type, 'string');
  assert.match(declaration.description, /event-driven wait/);
  assert.match(declaration.description, /either output preview/);
  assert.match(declaration.description, /command exit or deadline still returns/);
  assert.match(declaration.description, /Omitted or zero wait_ms reads immediately/);
  const id = '11111111-1111-4111-8111-111111111111';
  for (const args of [
    {id,wait_ms:0}, {id,wait_ms:1}, {id,wait_ms:30000},
    {id,until:'ready'}, {id,wait_ms:30000,until:'ready'},
    {id,stream:'stdout',offset:32768,limit:4096,wait_ms:30000,until:'ready'},
    {id,stream:'stderr',offset:0,limit:1,wait_ms:100},
  ]) assert.deepEqual(read(args),args);
});

test('rejects invalid wait bounds and text', () => {
  const id = '11111111-1111-4111-8111-111111111111';
  for (const wait_ms of [-1, 30001, 0.5, '100', null, true, [], {}])
    assert.equal(read({id,wait_ms}).error.code, 'invalid_request');
  for (const until of ['', null, true, 1, [], {}])
    assert.equal(read({id,until}).error.code, 'invalid_request');
});

test('waiting requires an ID while immediate reads preserve references', () => {
  const output = {call:'call-1', revision:'f'.repeat(64)};
  for (const args of [
    {wait_ms:1}, {id:null,wait_ms:1}, {until:'ready'}, {id:null,until:''},
    {wait_ms:0,until:'ready'}, {output,wait_ms:1}, {output,until:'ready'},
    {output,stream:'stdout',offset:0,limit:1,wait_ms:30000},
  ]) assert.equal(read(args).error.code, 'invalid_request');
  for (const args of [
    {wait_ms:0}, {id:null,wait_ms:0}, {output,wait_ms:0},
    {output,stream:'stdout',offset:0,limit:1,wait_ms:0},
  ]) assert.deepEqual(read(args),args);
});

test('command format affects only the summary', () => {
  const manifest = JSON.parse(readFileSync(new URL('../plugin.json', import.meta.url)));
  const declaration = manifest.extensions['dev.sailry.platform'].tools
    .find(tool => tool.name === 'run_command');
  assert.match(declaration.description, /read_command with wait_ms/);
  assert.match(declaration.description, /Don't append &; use background=true/);
  assert.doesNotMatch(declaration.description, /Do not poll with sleep or repeated read_command/);
  assert.doesNotMatch(declaration.description, /Keep the shell command in the foreground/);
  assert.deepEqual(declaration.display.input.summary, {path:'/command',code:true,format:'command'});
  assert.deepEqual(declaration.display.input.content, {path:'/command'});
  assert.equal(declaration.handler.operation, 'process.run');
  for (const command of [
    "cd '/tmp/worktree with spaces' && '/tmp/node/python/bin/python3' -c 'print(1)'",
    'SAILRY_PROJECT=fixture /usr/bin/env python3 -m unittest',
    "printf '%s\\n' '$HOME' 'quoted content'",
    "python3 - <<'PY'\nprint('🙂')\nPY",
  ]) {
    assert.equal(run({command}).command, command);
  }
});

test('description preserves command and optional metadata', () => {
  const command = "printf '%s' 'quoted\ncontent'";
  const description = 'Check the project configuration';
  assert.deepEqual(run({command,description}),
    {command,description,cwd:'',timeout_ms:120000,background:false,follow_up:false,pty:false,attachments:[],access:'workspace'});
  assert.equal(run({command}).description,null);
  assert.equal(run({command,description:null}).description,null);
  assert.equal(run({command,description:'🙂'.repeat(2001)}).isError,undefined);
  for (const description of ['', ' \n ', 7, 'a\0b']) {
    assert.equal(run({command,description}).isError,true);
  }
});

test('PTY requires a background command', () => {
  assert.equal(run({command:'cat',background:true,pty:true}).pty,true);
  assert.equal(run({command:'true'}).pty,false);
  for (const args of [{pty:true},{pty:'yes',background:true}])
    assert.equal(run({command:'cat',...args}).isError,true);
});

test('background follow-up requires an explicit boolean opt-in', () => {
  const manifest = JSON.parse(readFileSync(new URL('../plugin.json', import.meta.url)));
  const declaration = manifest.extensions['dev.sailry.platform'].tools
    .find(tool => tool.name === 'run_command');
  assert.deepEqual(declaration.handler.parameters.properties.follow_up.type, 'boolean');
  assert.equal(declaration.handler.parameters.properties.follow_up.default, false);
  const args = {command:'printf ready',background:true};
  assert.equal(run(args).follow_up, false);
  for (const follow_up of [false, true]) assert.equal(run({...args,follow_up}).follow_up, follow_up);
  for (const follow_up of [null, 'true', 1, [], {}]) assert.equal(run({...args,follow_up}).isError, true);
});

test('input preserves bytes and validates its bound', () => {
  const input = 'answer\n🙂\u001b[A';
  assert.deepEqual(write({id:'command',input}),{id:'command',input});
  assert.equal(write({id:'command',input:'🙂'.repeat(16384)}).isError,undefined);
  for (const args of [{input:'yes'}, {id:'',input:'yes'}, {id:'command',input:4},
      {id:'command',input:'🙂'.repeat(16385)}, {id:'command',input:'yes',session:'other'}])
    assert.equal(write(args).isError,true);
});

 test('access defaults to confinement and validates explicit escape', () => {
  const manifest = JSON.parse(readFileSync(new URL('../plugin.json', import.meta.url)));
  const declaration = manifest.extensions['dev.sailry.platform'].tools
    .find(tool => tool.name === 'run_command');
  const access = declaration.handler.parameters.properties.access;
  assert.doesNotMatch(declaration.description,/existing Git metadata/i);
  assert.match(access.description, /existing Git metadata/i);
  assert.match(access.description, /macOS also blocks .*\.git paths/);
  assert.match(access.description, /Git writes, including git init, git add and git commit/);
  assert.match(access.description, /Ask and Project require fresh approval for each unrestricted command/);
  assert.equal(access.default,'workspace');
  assert.match(access.description,/network access or paths outside the worktree/);
  assert.match(access.description,/Windows uses the selected permission mode/);
  assert.equal(run({command:'true'}).access, 'workspace');
  assert.equal(run({command:'true',access:'unrestricted'}).access, 'unrestricted');
  for (const access of [null, '', true, 'full', {}, []])
    assert.equal(run({command:'true',access}).isError, true);
});

test('large commands retain all requested inputs',()=>{
  const command='echo '.repeat(14000),description='Purpose '.repeat(400);
  const attachments=Array.from({length:9},(_,index)=>`attachment-${index}`);
  const result=run({command,description,timeout_ms:900001,attachments});
  assert.equal(result.command,command);
  assert.equal(result.description,description);
  assert.equal(result.timeout_ms,900001);
  assert.deepEqual(result.attachments,attachments);
  assert.equal(run({command,timeout_ms:Number.MAX_SAFE_INTEGER+1}).isError,true);
  const manifest=JSON.parse(readFileSync(new URL('../plugin.json',import.meta.url)));
  const timeout=manifest.extensions['dev.sailry.platform'].tools
    .find(tool=>tool.name==='run_command').handler.parameters.properties.timeout_ms;
  assert.equal(timeout.default,120000);
  assert.equal(timeout.maximum,600000);
});

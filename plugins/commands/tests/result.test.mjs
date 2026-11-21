import test from 'node:test';
import assert from 'node:assert/strict';
import {result} from '../dev.sailry.platform/host/main.js';
import {captions} from '../dev.sailry.platform/host/locales.js';

function info(pty = false) {
  return {
    id:'11111111-1111-4111-8111-111111111111',
    session:'22222222-2222-4222-8222-222222222222',
    turn:'33333333-3333-4333-8333-333333333333',
    command:'for i in $(seq 1 20); do echo tick $i; sleep 2; done',
    access:'workspace',pty,cwd:'/workspace',started_ms:1,
    status:{kind:'running'},services:[],
  };
}

function completion(outcome = {kind:'exited',data:0}) {
  return {
    kind:'command_result',
    data:{
      outcome,
      stdout:{text:'',truncated:false,invalid_utf8:false},
      stderr:{text:'',truncated:false,invalid_utf8:false},
      elapsed_ms:12,
    },
  };
}

test('background starts preserve their payload without details', () => {
  for (const pty of [false,true]) {
    const output = {kind:'command_started',data:info(pty)};
    const original = structuredClone(output);
    const prepared = result({output});
    assert.equal(prepared,output);
    assert.deepEqual(output,original);
    assert.equal(Object.hasOwn(prepared,'sailry_content'),false);
  }
});

test('empty captures do not add status notices', () => {
  const output = completion();
  const prepared = result({output});
  const {sailry_content,...retained} = prepared;
  assert.deepEqual(sailry_content.blocks,[
    {kind:'text',path:'/data/stdout/text',notices:[]},
    {kind:'text',path:'/data/stderr/text',notices:[]},
  ]);
  assert.deepEqual(retained,output);
});

test('stream results retain output and capture diagnostics', () => {
  for (const kind of ['command_result','command_output']) {
    const stdout = {text:'tick 1\n中文 🙂\n',truncated:true,invalid_utf8:true};
    const stderr = {text:'warning\n',truncated:false,invalid_utf8:true};
    const output = kind === 'command_result'
      ? {kind,data:{...completion().data,stdout,stderr}}
      : {kind,data:{info:info(),stdout,stderr}};
    const original = structuredClone(output);
    const prepared = result({output});
    const {sailry_content,...retained} = prepared;
    assert.deepEqual(retained,original);
    assert.deepEqual(output,original);
    assert.deepEqual(sailry_content.blocks,[
      {kind:'text',path:'/data/stdout/text',notices:[captions.truncated,captions.encoding]},
      {kind:'text',path:'/data/stderr/text',notices:[captions.encoding]},
    ]);
  }
});

test('command outcomes retain failure reporting', () => {
  for (const outcome of [
    {kind:'exited',data:7},
    {kind:'signal',data:15},
    {kind:'timed_out'},
    {kind:'unknown',data:'Process cleanup could not be confirmed'},
  ]) {
    const output = completion(outcome);
    output.data.stderr.text = 'original error\n';
    const prepared = result({output});
    const {sailry_content,isError,...retained} = prepared;
    assert.deepEqual(retained,output);
    assert.equal(isError,outcome.kind === 'exited' ? undefined : true);
    assert.equal(sailry_content.blocks.length,2);
  }
  const output = {isError:true,error:{code:'unavailable',message:'Command could not start'}};
  assert.equal(result({output}),output);
});

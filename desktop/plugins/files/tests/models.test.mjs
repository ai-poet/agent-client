import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const root = new URL('../',import.meta.url);
const extension = JSON.parse(await readFile(new URL('plugin.json',root),'utf8')).extensions['dev.sailry.platform'];
const source = (await readFile(new URL('dev.sailry.platform/host/models.js',root),'utf8'))
  .replace(/^import [^\n]+\n/gm,'').replace(/^export /gm,'');
const plain = value=>JSON.parse(JSON.stringify(value));
const id = 'a'.repeat(64);
const calls = [];
const entries = [{id,state:{kind:'installed'}},{id:'b'.repeat(64),state:{kind:'missing'}}];
const host = vm.runInNewContext(`${source}\n({ocr_models,ocr_install,ocr_remove})`,{models:{list:async input=>{calls.push(plain(input));return entries;}}});

test('discovery preserves definitions and filters installed state', async () => {
  assert.deepEqual(plain(await host.ocr_models({})),{models:entries});
  assert.deepEqual(plain(await host.ocr_models({installed_only:true})),{models:[entries[0]]});
  assert.deepEqual(calls,[{target:'node',kind:'ocr'},{target:'node',kind:'ocr'}]);
  for (const args of [{installed_only:1},{node:'foreign'},null]) {
    assert.equal((await host.ocr_models(args)).error.code,'invalid_request');
  }
});

test('lifecycle prepares exact native operations without configuration writes', () => {
  for (const handler of ['ocr_install','ocr_remove']) {
    assert.deepEqual(plain(host[handler]({id})),{model:{target:'node',kind:'ocr',id}});
    for (const args of [{id:'unknown'},{id,worktree:'foreign'},{id,enabled:true},{id,kind:'embedding'}]) {
      assert.equal(host[handler](args).error.code,'invalid_request');
    }
  }
  const tools = extension.tools.filter(tool=>tool.name.includes('ocr_model'));
  assert.equal(tools.length,3);
  assert.equal(tools[0].handler.read_only,true);
  assert.equal(tools[0].handler.operation,undefined);
  assert.equal(tools[1].handler.operation,'models.install');
  assert.equal(tools[2].handler.operation,'models.remove');
  assert.ok(extension.host.resources.includes('dev.sailry.platform/host/models.js'));
  assert.match(extension.tools.find(tool=>tool.name==='read_office').description,/visible in this conversation/);
});

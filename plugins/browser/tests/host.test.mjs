import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import test from 'node:test';
import * as host from '../dev.sailry.platform/host/main.js';

const manifest = JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
const extension = manifest.extensions['dev.sailry.platform'];
// Approval groups from the original agent adapter differ from lost-response classification.
const read = new Set('tabs read navigate scroll back forward refresh open focus frame wait screenshot'.split(' '));

test('research prefers bound search and anonymous fetch tools', () => {
  assert.match(extension.instructions, /Prefer available search, GitHub reads and web_fetch/);
  assert.match(extension.instructions, /public text, APIs and raw files/);
  assert.match(extension.instructions, /missing authenticated MCP access alone does not require browsing/);
  assert.match(extension.instructions, /interaction, login, JavaScript rendering/);
  assert.match(extension.instructions, /requested page views/);
  assert.match(extension.instructions, /no suitable read tool/);
  assert.match(extension.instructions, /Use only available tools and report missing capabilities/);
});

test('all eighteen tools retain approval groups and captured action dispatch', () => {
  assert.equal(extension.tools.length,18);
  assert.equal(new Set(extension.tools.map(tool => tool.name)).size,18);
  assert.deepEqual(new Set(extension.host.handlers),new Set(Object.keys(host)));
  for (const tool of extension.tools) {
    const action = tool.name.slice('browser_'.length);
    const args = action === 'wait' ? {tab:3,condition:{kind:'text',text:'Ready'},timeout_ms:500}
      : {tab:3,snapshot:'captured',element:4,url:'https://example.com',text:'页面 🙂'};
    const original = structuredClone(args);
    assert.equal(tool.handler.name,action);
    assert.equal(tool.handler.operation,`browser.${read.has(action) ? 'read' : 'control'}`);
    assert.equal(tool.presentation,action === 'screenshot' ? 'details' : 'summary');
    const prepared = ['open','navigate'].includes(action) ? {...original,background:false} : original;
    assert.deepEqual(host[action](args),{action,arguments:prepared});
    assert.deepEqual(args,original);
  }
  assert.equal(extension.tools.find(tool => tool.name === 'browser_screenshot').handler.operation,'browser.read');
});

test('captures show elements to the model and save only on request', () => {
  const tool = extension.tools.find(tool => tool.name === 'browser_screenshot');
  const {properties, required} = tool.handler.parameters;
  assert.deepEqual(required,['tab']);
  assert.deepEqual(Object.keys(properties).sort(),['format','save','scale','tab','target']);
  assert.deepEqual(properties.target.oneOf.map(option => option.properties.kind.const),['selector','element','rect']);
  assert.deepEqual(properties.scale.enum,['css','device']);
  assert.deepEqual(properties.format.enum,['jpeg','png']);
  assert.equal(properties.save.default,false);
  assert.equal(tool.keep_images,5);
  assert.match(tool.description,/save=true only when a project file is needed/);
  assert.match(tool.description,/images lists references/);
  assert.equal(tool.display.input.summary.result_path,'/response/path');
  assert.equal(tool.display.output.paths.value.path,'/response/path');
  assert.match(extension.instructions,/After a visual change, capture the changed element/);
  assert.match(extension.instructions,/Markdown image syntax using a returned images reference/);
  const args = {tab:2,target:{kind:'selector',selector:'#save'},scale:'device',format:'png',save:true};
  const original = structuredClone(args);
  assert.deepEqual(host.screenshot(args),{action:'screenshot',arguments:args});
  assert.deepEqual(args,original);
  const captured = {response:{tab:2,images:['sailry-image:call_1/0'],bounds:{x:1,y:2,width:30,height:20}}};
  assert.equal(host.result({output:{kind:'browser',data:captured.response}}),captured.response);
});

test('background navigation is explicit and preserves caller arguments', () => {
  for (const action of ['open','navigate']) {
    const tool = extension.tools.find(tool => tool.name === `browser_${action}`);
    assert.deepEqual(tool.handler.parameters.properties.background, {
      type:'boolean',default:false,
      description:'Use true for background automation; false shows the browser sidebar'
    });
    for (const background of [undefined,false,true]) {
      const args = {url:'https://example.com',...(background === undefined ? {} : {background})};
      const original = structuredClone(args);
      assert.deepEqual(host[action](args),{action,arguments:{...args,background:background ?? false}});
      assert.deepEqual(args,original);
    }
    for (const background of [null,'true',0,1,[],{}]) {
      assert.equal(host[action]({url:'https://example.com',background}).error.code,'invalid_request');
    }
    assert.match(tool.description,/background=true for automation/);
    assert.match(tool.description,/default false/);
  }
  for (const tool of extension.tools.filter(tool => !['browser_open','browser_navigate'].includes(tool.name))) {
    assert.equal(tool.handler.parameters.properties.background,undefined);
  }
  assert.match(extension.instructions,/foreground navigation for requested page views/);
  assert.match(extension.instructions,/later actions preserve the tab's background state/);
});

test('wait validation preserves requested duration and conditions', () => {
  for (const condition of [{kind:'text',text:'Ready'},{kind:'visible',selector:'#ready'},{kind:'hidden',selector:'#old'}]) {
    for (const timeout_ms of [1,20001,0xffffffff]) {
      const args = {tab:0,condition,timeout_ms};
      assert.deepEqual(host.wait(args),{action:'wait',arguments:args});
    }
  }
  assert.equal(host.wait({condition:{kind:'text',text:'🙂'.repeat(501)},timeout_ms:1}).action,'wait');
  for (const args of [
    {condition:{kind:'text',text:''},timeout_ms:1},
    {condition:{kind:'unknown',selector:'#ready'},timeout_ms:1},
    {condition:{kind:'text',text:'Ready'},timeout_ms:0},
    {condition:{kind:'text',text:'Ready'},timeout_ms:0x100000000},
    {condition:{kind:'text',text:'Ready'},timeout_ms:1.5}
  ]) assert.equal(host.wait(args).error.code,'invalid_request');
  for (const args of [undefined,null,[],false,'']) assert.equal(host.tabs(args).isError,true);
});

test('success preserves browser payload and known failures supply localized notices', () => {
  const data = {tab:3,snapshot:'snapshot',text:'literal <script> content',elements:[],truncated:true,path:'.generated/capture.jpg'};
  assert.equal(host.result({output:{kind:'browser',data}}),data);
  const failures = [
    'browser tab is empty; navigate to a URL before reading or interacting',
    'browser page is unavailable',
    'browser page failed to load'
  ];
  for (const message of failures) {
    const output = {isError:true,error:{code:'unavailable',message}};
    const annotation = host.result({output});
    assert.equal(annotation.sailry_content.blocks[0].kind,'notice');
    assert.ok(annotation.sailry_content.blocks[0].message.locales['zh-CN']);
    assert.equal(output.error.message,message);
  }
  const unknown = {isError:true,error:{code:'outcome_unknown',message:'Browser response unavailable'}};
  assert.equal(host.result({output:unknown}),unknown);
});

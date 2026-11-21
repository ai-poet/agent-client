import assert from 'node:assert/strict';
import test from 'node:test';
import {available,needsKey,models,popup,chosen,contains,icon} from '../dev.sailry.platform/desktop/catalog.js';

test('uses provider marks and service fallbacks',()=>{
  for(const driver of ['exa','tavily','brave','searxng','bocha','volcengine','deepseek'])assert.equal(icon(driver),`ailogo:${driver}-color`);
  assert.equal(icon('tinyfish'),'globe');assert.equal(icon('model'),'bot');assert.equal(icon('unknown'),undefined);
});

test('offers only advertised drivers',()=>{
  assert.deepEqual(available({providers:{model:{},exa:{},volcengine:{},tinyfish:{},unknown:{}}}),
    ['exa','tinyfish','volcengine','model']);
  assert.deepEqual(available(null),[]);
  assert.equal(needsKey({key_required:true}),true);
  assert.equal(needsKey({key_required:false}),false);
});

test('filters unsupported interfaces and login methods',()=>{
  const choices=models({models:[
    {kind:'provider',id:'one/first',provider:'One',api:'responses',authentication:'api_key'},
    {kind:'provider',id:'one/second',provider:'One',api:'responses',authentication:'chat_gpt_o_auth',default:true},
    {kind:'provider',id:'two/vendor/model',provider:'Two',api:'anthropic',authentication:'plugin'},
    {kind:'provider',id:'three/search',provider:'Three',api:'gemini',authentication:'api_key'},
    {kind:'provider',id:'four/chat',provider:'Four',api:'chat_completions'},
    {kind:'provider',id:'five/plugin',provider:'Five',api:'plugin'},
    {kind:'provider',id:'copilot/search',provider:'Copilot',api:'responses',authentication:'copilot'},
    {kind:'provider',id:'copilot/claude',provider:'Copilot',api:'anthropic',authentication:'copilot'},
    {kind:'provider',id:'invalid',provider:'Invalid',api:'responses'},
  ]});
  assert.deepEqual(popup(choices).map(item=>[item.kind,item.id,item.provider,item.model]),[
    ['provider','one/first','One','first'],['provider','one/second','One','second'],
    ['provider','two/vendor/model','Two','vendor/model'],['provider','three/search','Three','search']]);
  assert.deepEqual(chosen(choices,'two/vendor/model'),{provider:'two',model:'vendor/model'});
  assert.equal(chosen(choices,'four/chat'),null);
  assert.equal(contains(choices,{provider:'three',model:'search'}),true);
  assert.equal(contains(choices,{provider:'one',model:'vendor/model'}),false);
  assert.equal(contains(choices,null),false);
});

test('uses the saved service order',()=>{
  assert.deepEqual(available({order:['model','exa','tinyfish'],providers:{exa:{},model:{},tinyfish:{}}}),
    ['model','exa','tinyfish']);
});

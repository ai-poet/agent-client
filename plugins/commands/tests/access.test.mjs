import test from 'node:test';
import assert from 'node:assert/strict';
import {choices, selection} from '../dev.sailry.platform/desktop/access.js';
import {messages} from '../dev.sailry.platform/desktop/locales.js';

test('ordinary and CLI access choices remain explicit and bounded',()=>{
  assert.deepEqual(selection('claude'),{tool:'claude',access:'unrestricted'});
  assert.deepEqual(selection('shell:workspace'),{tool:null,access:'workspace'});
  assert.deepEqual(selection('claude:workspace'),{tool:'claude',access:'workspace'});
  assert.deepEqual(selection('shell:unrestricted'),{tool:null,access:'unrestricted'});
  assert.deepEqual(selection('claude:unrestricted'),{tool:'claude',access:'unrestricted'});
  for(const value of ['',':workspace','shell:full','shell:unrestricted:extra'])
    assert.throws(()=>selection(value));
  const menu=choices([{tool:'claude',available:false}],messages('en'));
  assert.equal(menu.find(item=>item.id==='claude').enabled,false);
  assert.equal(menu.find(item=>item.id==='claude:workspace').enabled,false);
  assert.equal(menu.find(item=>item.id==='shell:workspace').enabled,true);
  assert.equal(menu.some(item=>item.id.endsWith(':unrestricted')),false);
});

test('consequences are translated in every supported locale',()=>{
  for(const locale of ['en','de','es','fr','ja','ko','pt-BR','ru','zh-CN','zh-TW']) {
    const text=messages(locale);
    for(const key of ['access','workspace','full']) {
      assert.ok(text[key]);
      if(locale!=='en')assert.notEqual(text[key],messages('en')[key]);
    }
  }
});

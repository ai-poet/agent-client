import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

async function fixture(value, answer = value) {
  let requests = 0;
  const source = (await readFile(new URL('../dev.sailry.platform/host/permissions.js',import.meta.url),'utf8'))
    .replace(/^import.*\n/,'').replaceAll('export async function','async function');
  const handlers = vm.runInNewContext(`${source}\n({check,request})`, {
    readComputerPermissions: async () => value,
    requestComputerPermission: async () => { requests++; return answer; },
  });
  return {handlers, requests: () => requests};
}
const missing = {local:true,platform:'macos',accessibility:false,screen_capture:false};

test('one request returns both current grants', async () => {
  const {handlers,requests} = await fixture(missing,{...missing,accessibility:true,screen_capture:true});
  const checked = await handlers.check();
  assert.equal(checked.can_request,true);
  assert.equal(requests(),0);
  const result = await handlers.request({kind:'accessibility'});
  assert.equal(requests(),1);
  assert.ok(result.grants.every(grant => grant.authorized === true));
});

test('granted remote and unsupported subjects do not prompt', async () => {
  for (const state of [{...missing,accessibility:true},{...missing,local:false},{...missing,platform:'windows'}]) {
    const {handlers,requests} = await fixture(state);
    await handlers.request({kind:'accessibility'});
    assert.equal(requests(),0);
  }
});

test('cancelled onboarding preserves missing grants', async () => {
  const {handlers,requests} = await fixture(missing,null);
  const result = await handlers.request({kind:'screen_recording'});
  assert.equal(requests(),1);
  assert.ok(result.grants.every(grant => grant.authorized === false));
});

test('contributes standard types without a settings page', async () => {
  const manifest = JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  const extension = manifest.extensions['dev.sailry.platform'];
  assert.equal(extension.settings_page,undefined);
  assert.deepEqual(extension.permissions.map(entry => entry.kind),['accessibility','screen_recording']);
  assert.equal(new Set(extension.permissions.map(entry => entry.subject)).size,1);
  assert.equal(new Set(extension.permissions.map(entry => entry.request_group)).size,1);
});

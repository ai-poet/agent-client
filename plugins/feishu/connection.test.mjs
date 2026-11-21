import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {gunzipSync} from 'node:zlib';
import {SourceTextModule} from 'node:vm';
import {runtimeTest, translate, translationInput} from '../scripts/test-host.mjs';

const manifest = JSON.parse(readFileSync(new URL('./plugin.json', import.meta.url)));
const extension = manifest.extensions['dev.sailry.platform'];
const connector = extension.connectors[0], registration = connector.auth[0].registration;
const module = new SourceTextModule(readFileSync(new URL('./dev.sailry.platform/auth/registration.js', import.meta.url), 'utf8'));
await module.link(() => { throw new Error('Registration imports are forbidden'); });
await module.evaluate();
const plain = value => JSON.parse(JSON.stringify(value));
const run = (state, event, now_ms = 1000) => plain(module.namespace.authorize({context:{scopes:registration.scopes},state,event,now_ms}));
const begin = body => run({phase:'begin'}, {status:200,body:{device_code:'private-device-code',verification_uri_complete:'https://accounts.feishu.cn/app/device?verify=opaque',interval:5,expires_in:600,...body}});

test('declares app credentials and private registration', () => {
  assert.equal(extension.settings_schema, undefined);
  assert.equal(extension.settings_page, undefined);
  assert.equal(extension.registration, undefined);
  assert.equal(connector.auth[0].kind, 'application');
  assert.equal(connector.auth[0].token.url, 'https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal');
  assert.deepEqual(connector.inject, [{server:'feishu',header:'X-Lark-MCP-TAT'}]);
  assert.equal(connector.fields.find(field => field.name === 'app_secret').secret, true);
  assert.equal(registration.flow.kind, 'script');
  assert.equal(registration.flow.callback, null);
  assert.deepEqual(registration.issues, {client_id:'app_id',client_secret:'app_secret'});
});

test('requests fixed steps before prompting', () => {
  const initial = run(null, {kind:'start'});
  assert.deepEqual(initial, {kind:'request',name:'init',state:{phase:'init'},fields:{}});
  assert.equal(run(initial.state, {status:200,body:{supported_auth_methods:['client_secret']}}).name, 'begin');
  assert.equal(run(initial.state, {status:200,body:{supported_auth_methods:['pkce']}}).kind, 'reject');
  const prompted = begin();
  assert.equal(prompted.kind, 'prompt');
  assert.equal(prompted.expires_at_ms, 601000);
  assert.equal(prompted.url.includes('private-device-code'), false);
  const poll = run(prompted.state, {kind:'presented'});
  assert.equal(poll.name, 'poll');
  assert.equal(poll.fields.device_code, 'private-device-code');
});

test('preserves official verification and exact scopes', () => {
  const prompted = begin({verification_uri_complete:'https://accounts.feishu.cn/app/device?verify=opaque&source=old&addons=old&clientID=old'});
  const url = new URL(prompted.url);
  assert.equal(url.origin, 'https://accounts.feishu.cn');
  assert.equal(url.searchParams.get('verify'), 'opaque');
  assert.equal(url.searchParams.get('source'), 'sailry');
  assert.equal(url.searchParams.has('clientID'), false);
  assert.deepEqual(JSON.parse(gunzipSync(Buffer.from(url.searchParams.get('addons'), 'base64url'))), {preset:false,scopes:{tenant:registration.scopes}});
  for (const verification_uri_complete of ['http://accounts.feishu.cn/app/device','https://accounts.feishu.cn.evil.test/app/device','https://user@accounts.feishu.cn/app/device','https://accounts.feishu.cn/app/device#fragment']) assert.equal(begin({verification_uri_complete}).kind,'reject');
});

test('bounds polling and maps terminal outcomes', () => {
  let state = {...begin().state, phase:'poll'};
  const pending = run(state, {status:400,body:{error:'authorization_pending'}});
  assert.equal(pending.kind, 'wait'); assert.equal(pending.delay_ms, 5000);
  const slow = run(state, {status:400,body:{error:'slow_down'}});
  assert.equal(slow.delay_ms, 10000);
  assert.equal(run(state,{status:400,body:{error:'access_denied'}}).code,'permission_denied');
  assert.equal(run(state,{status:400,body:{error:'expired_token'}}).code,'expired');
  assert.equal(run(state,{status:200,body:{}},state.expires).code,'expired');
  for (const body of [{interval:0},{interval:61},{expires_in:901},{device_code:'private\ncode'}]) assert.equal(begin(body).kind,'reject');
});

test('issues only verified tenant credentials', () => {
  const state = {...begin().state, phase:'poll'};
  const issued = run(state,{status:200,body:{client_id:'cli_example',client_secret:'private-app-secret',user_info:{tenant_brand:'feishu'}}});
  assert.deepEqual(issued,{kind:'finish',grant:{client_id:'cli_example',client_secret:'private-app-secret'}});
  assert.equal(run(state,{status:200,body:{client_id:'cli_example',client_secret:'private-app-secret',user_info:{tenant_brand:'lark'}}}).code,'not_configured');
  assert.equal(run(state,{status:200,body:{client_id:'other',client_secret:'private-app-secret'}}).kind,'reject');
});

runtimeTest('runs registration in the private translator', async () => {
  const input = await translationInput(new URL('./',import.meta.url), registration.flow.script, registration.flow.handler,
    {context:{scopes:registration.scopes},state:{phase:'init'},event:{status:200,body:{supported_auth_methods:['client_secret']}},now_ms:1000});
  assert.equal(translate(input).result.Ok.name,'begin');
});

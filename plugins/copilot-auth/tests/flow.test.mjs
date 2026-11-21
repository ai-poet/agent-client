import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {authorize} from '../dev.sailry.platform/auth/flow.js';
import {runtimeTest, translate, translationInput} from '../../scripts/test-host.mjs';

const now = 1800000000000;
const context = {previous: null, host_id: 'install', registration: null, redirect: '', scopes: ['read:user'],
  fields: {client_id: 'Iv1.b507a08c87ecfe98', user_agent: 'Sailry/0.1', editor_version: 'vscode/1.140.0', editor_plugin_version: 'copilot-chat/0.68.0'},
  proof: {state: 'private-state', nonce: 'private-nonce', verifier: 'private-verifier', challenge: 'public-challenge'}};
const device = {device_code: 'private-device', user_code: 'ABCD-1234', verification_uri: 'https://github.com/login/device', expires_in: 600, interval: 5};
const exchanged = {token: 'session-token', expires_at: now / 1000 + 1500, endpoints: {api: 'https://api.individual.githubcopilot.com'}};
const answer = (body, status = 200) => ({kind: 'response', status, body});
const step = (event, state = null, overrides = {}, at = now) => authorize({event, state, context: {...context, ...overrides}, now_ms: at});

// Drives the step function with scripted answers, as the Node does.
function run(answers, overrides = {}) {
  const effects = [];
  let event = {kind: overrides.previous ? 'refresh' : 'start'}, state = null;
  for (;;) {
    const effect = step(event, state, overrides);
    effects.push(effect);
    state = effect.state;
    switch (effect.kind) {
      case 'request': event = answers.shift(); assert.ok(event, `unanswered ${effect.name} request`); break;
      case 'prompt': event = {kind: 'presented'}; break;
      case 'wait': event = {kind: 'elapsed'}; break;
      default: assert.deepEqual(answers, []); return effects;
    }
  }
}
const waits = effects => effects.filter(effect => effect.kind === 'wait').map(effect => effect.delay_ms);
const grant = {kind: 'copilot', access: 'session-token', github: 'github-token', expires_at_ms: now + 1500000, endpoint: 'https://api.githubcopilot.com', renewal_pending: false};

test('declares the device flow and the native credential', async () => {
  const manifest = JSON.parse(await readFile(new URL('../plugin.json', import.meta.url), 'utf8'));
  assert.equal(manifest.version, '0.1.2');
  const [declaration] = manifest.extensions['dev.sailry.platform'].model_auth;
  assert.match(declaration.warning.text, /GitHub's terms/);
  assert.deepEqual(Object.keys(declaration.warning.locales).sort(), ['de', 'es', 'fr', 'ja', 'ko', 'pt-BR', 'ru', 'zh-CN', 'zh-TW']);
  for (const text of [declaration.warning.text, ...Object.values(declaration.warning.locales)]) {
    assert.ok(text.trim().length > 0 && Buffer.byteLength(text) <= 1024 && !/[\u0000-\u001f\u007f]/.test(text));
  }
  assert.equal(declaration.authentication, 'copilot');
  assert.equal('token' in declaration, false);
  assert.deepEqual(declaration.scopes, ['read:user']);
  assert.equal(declaration.flow.callback, null);
  assert.equal(declaration.flow.refresh, true);
  const requests = declaration.flow.requests;
  assert.deepEqual(Object.keys(requests), ['device', 'token', 'session']);
  assert.equal(requests.device.url, 'https://github.com/login/device/code');
  assert.equal(requests.token.url, 'https://github.com/login/oauth/access_token');
  assert.equal(requests.token.body.fields.grant_type, 'urn:ietf:params:oauth:grant-type:device_code');
  assert.equal(requests.session.url, 'https://api.github.com/copilot_internal/v2/token');
  assert.equal(requests.session.method, 'GET');
  assert.equal(requests.session.headers.Authorization, 'token {github}');
});

test('signs in after one poll', () => {
  const effects = run([answer(device), answer({access_token: 'github-token'}), answer(exchanged)]);
  assert.deepEqual(effects.map(effect => effect.kind), ['request', 'prompt', 'wait', 'request', 'request', 'finish']);
  assert.deepEqual(effects[0].fields, {scope: 'read:user'});
  assert.equal(effects[1].url, 'https://github.com/login/device');
  assert.equal(effects[1].user_code, 'ABCD-1234');
  assert.equal(effects[1].expires_at_ms, now + 600000);
  assert.deepEqual(waits(effects), [5000]);
  assert.deepEqual(effects[3].fields, {device_code: 'private-device'});
  assert.deepEqual(effects[4].fields, {github: 'github-token'});
  assert.deepEqual(effects[5], {kind: 'finish', grant});
});

test('keeps polling while authorization is pending', () => {
  const pending = answer({error: 'authorization_pending'});
  const effects = run([answer(device), pending, pending, answer({access_token: 'github-token'}), answer(exchanged)]);
  assert.deepEqual(waits(effects), [5000, 5000, 5000]);
  assert.equal(effects.at(-1).grant.github, 'github-token');
});

test('slows down when GitHub asks', () => {
  const effects = run([answer(device), answer({error: 'slow_down'}), answer({error: 'slow_down', interval: 20}), answer({error: 'authorization_pending'}), answer({access_token: 'github-token'}), answer(exchanged)]);
  assert.deepEqual(waits(effects), [5000, 10000, 20000, 20000]);
  assert.equal(effects.at(-1).kind, 'finish');
});

test('bounds the interval and the code lifetime', () => {
  const prompt = step(answer({...device, interval: 0, expires_in: 86400}), {phase: 'device'});
  assert.equal(prompt.expires_at_ms, now + 900000);
  assert.equal(prompt.state.interval, 1);
  const fallback = step(answer({...device, interval: undefined, expires_in: undefined}), {phase: 'device'});
  assert.equal(fallback.expires_at_ms, now + 900000);
  assert.equal(fallback.state.interval, 5);
  assert.equal(step(answer({error: 'slow_down'}), {phase: 'polling', device: 'private-device', interval: 899, misses: 0}).delay_ms, 900000);
});

test('stops when the code expires or is declined', () => {
  assert.deepEqual(run([answer(device), answer({error: 'expired_token'})]).at(-1), {kind: 'reject', code: 'expired'});
  assert.deepEqual(run([answer(device), answer({error: 'access_denied'})]).at(-1), {kind: 'reject', code: 'permission_denied'});
  assert.deepEqual(run([answer(device), answer({error: 'unsupported_grant_type'})]).at(-1), {kind: 'reject', code: 'invalid_request'});
  assert.deepEqual(run([answer(device), answer({})]).at(-1), {kind: 'reject', code: 'invalid_request'});
});

test('tolerates a few unanswered polls', () => {
  const missed = answer(null, 502);
  const recovered = run([answer(device), missed, missed, answer({error: 'authorization_pending'}), missed, answer({access_token: 'github-token'}), answer(exchanged)]);
  assert.equal(recovered.at(-1).kind, 'finish');
  const failed = run([answer(device), missed, missed, missed, missed, answer(null, 429)]);
  assert.deepEqual(failed.at(-1), {kind: 'reject', code: 'invalid_request'});
  assert.deepEqual(waits(failed), [5000, 5000, 5000, 5000, 5000]);
});

test('rejects an invalid device answer', () => {
  for (const body of [{...device, verification_uri: 'https://github.example/login/device'}, {...device, device_code: ''}, {...device, user_code: undefined}, {...device, user_code: 'A'.repeat(257)}, null, []]) {
    assert.deepEqual(step(answer(body), {phase: 'device'}), {kind: 'reject', code: 'invalid_request'});
  }
  assert.deepEqual(step(answer(null, 503), {phase: 'device'}), {kind: 'reject', code: 'invalid_request'});
  assert.deepEqual(step({kind: 'start'}, null, {fields: {}}), {kind: 'reject', code: 'invalid_request'});
});

test('reports a revoked sign-in and a missing subscription', () => {
  const state = {phase: 'session', github: 'github-token'};
  for (const status of [401, 403, 404]) assert.deepEqual(step(answer(status === 401 ? null : {message: 'refused'}, status), state), {kind: 'reject', code: 'permission_denied'});
  assert.deepEqual(run([answer(device), answer({access_token: 'github-token'}), answer({message: 'no access'}, 403)]).at(-1), {kind: 'reject', code: 'permission_denied'});
});

test('rejects an invalid session answer', () => {
  const state = {phase: 'session', github: 'github-token'};
  for (const body of [{...exchanged, token: ''}, {...exchanged, token: undefined}, {...exchanged, expires_at: '1800001500'}, {...exchanged, expires_at: 1.5}, {...exchanged, expires_at: Number.MAX_SAFE_INTEGER}, null]) {
    assert.deepEqual(step(answer(body), state), {kind: 'reject', code: 'invalid_request'});
  }
  assert.deepEqual(step(answer(null, 500), state), {kind: 'reject', code: 'invalid_request'});
  assert.deepEqual(step(answer({...exchanged, expires_at: now / 1000}), state), {kind: 'reject', code: 'expired'});
});

test('retains true expiry and pins the service endpoint', () => {
  const state = {phase: 'session', github: 'github-token'};
  assert.equal(step(answer(exchanged), state).grant.expires_at_ms, now + 1500000);
  assert.equal(step(answer({...exchanged, expires_at: now / 1000 + 120}), state).grant.expires_at_ms, now + 120000);
  for (const endpoints of [undefined, {}, {api: 'https://api.business.githubcopilot.com'}, {api: 'https://copilot.example'}]) {
    assert.equal(step(answer({...exchanged, endpoints}), state).grant.endpoint, 'https://api.githubcopilot.com');
  }
});

test('refreshes with the stored GitHub sign-in', () => {
  const effects = run([answer({token: 'renewed-token', expires_at: now / 1000 + 1800})], {previous: grant});
  assert.deepEqual(effects.map(effect => effect.kind), ['request', 'finish']);
  assert.equal(effects[0].name, 'session');
  assert.deepEqual(effects[0].fields, {github: 'github-token'});
  assert.deepEqual(effects[1].grant, {...grant, access: 'renewed-token', expires_at_ms: now + 1800000});
  assert.deepEqual(run([answer(null, 401)], {previous: grant}).at(-1), {kind: 'reject', code: 'permission_denied'});
  assert.deepEqual(step({kind: 'refresh'}, null, {previous: {...grant, github: undefined}}), {kind: 'reject', code: 'permission_denied'});
});

test('refuses events out of order', () => {
  assert.deepEqual(step({kind: 'elapsed'}, {phase: 'presented'}), {kind: 'reject', code: 'invalid_request'});
  assert.deepEqual(step({kind: 'presented'}, {phase: 'waiting'}), {kind: 'reject', code: 'invalid_request'});
  assert.deepEqual(step({kind: 'failure', code: 'unavailable'}, {phase: 'polling'}), {kind: 'reject', code: 'invalid_request'});
  assert.deepEqual(step(answer(exchanged), null), {kind: 'reject', code: 'invalid_request'});
});

runtimeTest('uses production private translation', async () => {
  const manifest = JSON.parse(await readFile(new URL('../plugin.json', import.meta.url), 'utf8'));
  const [declaration] = manifest.extensions['dev.sailry.platform'].model_auth;
  let state = null;
  const call = async event => {
    const input = {event, state, context, now_ms: now};
    const report = translate(await translationInput(new URL('../', import.meta.url), declaration.flow.script, 'authorize', input));
    assert.deepEqual(report.calls, []);
    assert.deepEqual(report.result.Ok, authorize(input));
    state = report.result.Ok.state;
    return report.result.Ok;
  };
  assert.equal((await call({kind: 'start'})).name, 'device');
  assert.equal((await call(answer(device))).kind, 'prompt');
  assert.equal((await call({kind: 'presented'})).kind, 'wait');
  assert.equal((await call({kind: 'elapsed'})).name, 'token');
  assert.equal((await call(answer({access_token: 'github-token'}))).name, 'session');
  assert.deepEqual((await call(answer(exchanged))).grant, grant);
});

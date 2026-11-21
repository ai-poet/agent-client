import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import test from 'node:test';

const manifest = JSON.parse(await readFile(new URL('../plugin.json', import.meta.url), 'utf8'));
const extension = manifest.extensions['dev.sailry.platform'];
const [program] = extension.programs;
const [adapter] = extension.model_adapters;
const [provider] = extension.model_providers;
const file = name => program.files.find(entry => entry.path === name).content;

test('declares one program and no script', () => {
  assert.deepEqual(extension.actions, ['models.provide', 'process.run']);
  assert.equal(extension.programs.length, 1);
  assert.equal(program.command, 'agy');
  assert.equal(program.args[0], 'acp');
  assert.equal(adapter.script, undefined);
  assert.equal(extension.host, undefined);
  assert.equal(adapter.acp.program, program.id);
  assert.equal(provider.adapter, adapter.id);
});

test('keeps the profile and traffic of agy', () => {
  const words = [...program.args, ...Object.keys(program.env), ...Object.values(program.env)].join(' ');
  for (const refused of ['--gemini-dir', '--app-data-dir', '--cloud_code_endpoint', '--ide-mcp-port', '--login', 'SSH_CONNECTION']) {
    assert.ok(!words.includes(refused), refused);
  }
  assert.ok(program.files.every(entry => entry.path.startsWith('.agents/')));
  assert.deepEqual(provider.auth, {kind: 'none'});
});

test('strips the agent', () => {
  const name = program.args[program.args.indexOf('--agent') + 1];
  const agent = file(`.agents/agents/${name}/agent.md`);
  const header = agent.split('---')[1];
  for (const line of [`name: ${name}`, 'mainAgent: true', 'subagent: false', 'excludeDefaultComponents: true',
    'inheritMcp: false', 'inheritCustomizations: false', 'tools: []', 'skills: []', 'rules: []', 'agents: []', 'plugins: []']) {
    assert.ok(header.split('\n').includes(line), line);
  }
  assert.ok(program.args.includes('--include-default-customizations=false'));
});

test('denies every built-in tool', () => {
  const hooks = JSON.parse(file('.agents/hooks.json'));
  const [group] = Object.values(hooks);
  const [rule] = group.PreToolUse;
  assert.equal(rule.matcher, '*');
  assert.equal(rule.hooks[0].type, 'command');
  const printed = rule.hooks[0].command.match(/'(\{.*\})'$/)[1];
  assert.equal(JSON.parse(printed).decision, 'deny');
});

test('declares the vendor differences', () => {
  assert.equal(adapter.framing, 'ndjson');
  assert.deepEqual(adapter.origins, []);
  assert.equal(adapter.acp.system_meta, 'antigravity.systemPrompt.append');
  assert.deepEqual(adapter.acp.efforts, {low: '-low', medium: '-medium', high: '-high'});
  assert.equal(adapter.acp.replay_bytes, 112 * 1024);
  assert.equal(adapter.acp.model_argument, undefined);
  assert.equal(adapter.acp.auth_method, undefined);
  assert.ok(adapter.acp.sign_in.length <= 256 && !adapter.acp.sign_in.endsWith('.'));
  assert.deepEqual(provider.models, {kind: 'discover'});
});

test('warns before a channel is added', () => {
  // The channel has no sign-in, so the preset carries the warning.
  assert.equal(manifest.version, '0.1.1');
  assert.equal(extension.model_auth, undefined);
  assert.match(provider.warning.text, /your own account/);
  assert.deepEqual(Object.keys(provider.warning.locales).sort(), ['de', 'es', 'fr', 'ja', 'ko', 'pt-BR', 'ru', 'zh-CN', 'zh-TW']);
  for (const text of [provider.warning.text, ...Object.values(provider.warning.locales)]) {
    assert.ok(text.trim().length > 0 && Buffer.byteLength(text) <= 1024 && !/[\u0000-\u001f\u007f]/.test(text));
  }
});

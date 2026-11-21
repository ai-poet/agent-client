import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import test from 'node:test';

const manifest = JSON.parse(await readFile(new URL('../plugin.json', import.meta.url), 'utf8'));
const extension = manifest.extensions['dev.sailry.platform'];
const skill = await readFile(new URL('../skills/workflows/SKILL.md', import.meta.url), 'utf8');
const configuration = JSON.parse(await readFile(new URL('../mcp.json', import.meta.url), 'utf8'));

test('public research uses anonymous reads before browser interaction', () => {
  for (const instructions of [extension.instructions, skill]) {
    assert.match(instructions, /do(?:es)? not require a token/);
    assert.match(instructions, /web_fetch/);
    assert.match(instructions, /https:\/\/api\.github\.com\/search\/repositories\?q=/);
    assert.match(instructions, /https:\/\/api\.github\.com\/repos\//);
    assert.match(instructions, /before opening a browser/);
    assert.match(instructions, /interaction, login, JavaScript rendering/);
    assert.match(instructions, /no suitable (?:read|search or fetch) tool/);
    assert.match(instructions, /rate limits/);
  }
  assert.match(skill, /https:\/\/raw\.githubusercontent\.com\//);
  assert.match(skill, /web_fetch is bound/);
  assert.match(skill, /actual ref and path/);
  assert.match(skill, /do not invent a tool call/);
  assert.match(manifest.description, /Public GitHub research/);
});

test('authenticated workflows retain token and write boundaries', () => {
  assert.equal(extension.api_version, 'v1');
  assert.deepEqual(extension.actions, []);
  assert.equal(extension.tools, undefined);
  assert.equal(configuration.mcpServers.github.url, 'https://api.githubcopilot.com/mcp/');
  assert.equal(configuration.mcpServers.github.headers.Authorization, undefined);
  assert.match(extension.instructions, /official GitHub MCP service still requires authentication/);
  assert.match(skill, /official hosted MCP service requires a GitHub access token/);
  assert.match(skill, /Never ask them to paste a token into chat/);
  assert.match(skill, /Select a protected token or the current authenticated GitHub CLI account/);
  assert.match(skill, /silently reads the token and verifies that account/);
  assert.match(skill, /never enter the model's arguments or output/);
  assert.match(skill, /run a command that exposes it to the model/);
  assert.match(skill, /anonymous path to private repositories or writes/);
  assert.match(skill, /do not automatically repeat/);
});

test('declares explicit token and silent fixed CLI methods', () => {
  assert.equal(extension.credential_source, undefined);
  assert.equal(extension.settings_schema, undefined);
  const connector = extension.connectors[0], cli = connector.auth.find(auth=>auth.kind==='local_cli');
  assert.deepEqual(connector.auth.map(auth=>auth.kind),['secret','local_cli']);
  assert.deepEqual(connector.inject,[{server:'github',header:'Authorization',prefix:'Bearer '}]);
  assert.equal(cli.command,'gh');
  assert.deepEqual(cli.status.argv,['auth','status','--active','--hostname','github.com','--json','hosts']);
  assert.deepEqual(cli.token.argv,['auth','token','--hostname','github.com','--user','{account}']);
  assert.deepEqual(cli.env,{GH_PROMPT_DISABLED:'1'});
  assert.equal(cli.timeout_ms,15000); assert.equal(cli.max_output,65536);
  assert.equal(connector.fields[0].secret,true); assert.equal(connector.fields[0].default,undefined);
});

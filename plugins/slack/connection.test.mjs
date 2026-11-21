import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

const manifest = JSON.parse(readFileSync(new URL('./plugin.json', import.meta.url)));
const mcp = JSON.parse(readFileSync(new URL('./mcp.json', import.meta.url)));

test('declares the official remote connection', () => {
  assert.equal(manifest.name, "slack");
  assert.equal(manifest.extensions['dev.sailry.platform'].api_version, 'v1');
  const server = mcp.mcpServers["slack"];
  assert.equal(server.type, 'streamable-http');
  assert.equal(server.url, "https://mcp.slack.com/mcp");
  assert.equal(server.command, undefined);
  assert.equal(server.args, undefined);
  assert.equal(new URL(server.url).username, '');
});

test('keeps registered client secrets protected', () => {
  const connector = manifest.extensions['dev.sailry.platform'].connectors[0];
  assert.deepEqual(connector.auth[0], {kind:'mcp_oauth',server:'slack',client:{fields:{client_id:'client_id',client_secret:'client_secret'}}});
  assert.equal(connector.fields.find(field=>field.name==='client_secret').secret,true);
  assert.equal(connector.fields.find(field=>field.name==='client_secret').default,undefined);
  assert.equal(connector.fields.find(field=>field.name==='client_id').secret,false);
  assert.deepEqual(mcp.mcpServers.slack.headers ?? {}, {});
  assert.equal(manifest.extensions['dev.sailry.platform'].settings_schema,undefined);
  for (const field of connector.fields) assert.equal(Object.keys(field.locales).length,10);
});

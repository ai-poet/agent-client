import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

const manifest = JSON.parse(readFileSync(new URL('./plugin.json', import.meta.url)));
const mcp = JSON.parse(readFileSync(new URL('./mcp.json', import.meta.url)));

test('declares the official remote connection', () => {
  assert.equal(manifest.name, "tencent-docs");
  assert.equal(manifest.extensions['dev.sailry.platform'].api_version, 'v1');
  const server = mcp.mcpServers["tencent-docs"];
  assert.equal(server.type, 'streamable-http');
  assert.equal(server.url, "https://docs.qq.com/openapi/mcp");
  assert.equal(server.command, undefined);
  assert.equal(server.args, undefined);
  assert.equal(new URL(server.url).username, '');
});

test('binds a protected token to the declared server', () => {
  const connector = manifest.extensions['dev.sailry.platform'].connectors[0];
  assert.deepEqual(connector.inject,[{server:'tencent-docs',header:'Authorization'}]);
  assert.equal(connector.fields[0].secret,true);
  assert.equal(connector.fields[0].help,'https://docs.qq.com/open/auth/mcp.html');
  assert.equal(connector.fields[0].default,undefined);
  assert.equal(Object.keys(connector.fields[0].locales).length,10);
  assert.deepEqual(mcp.mcpServers['tencent-docs'].headers ?? {},{});
});

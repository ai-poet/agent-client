import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

const manifest = JSON.parse(readFileSync(new URL('./plugin.json', import.meta.url)));
const mcp = JSON.parse(readFileSync(new URL('./mcp.json', import.meta.url)));

test('declares the official remote connection', () => {
  assert.equal(manifest.name, "linear");
  assert.equal(manifest.extensions['dev.sailry.platform'].api_version, 'v1');
  const server = mcp.mcpServers["linear"];
  assert.equal(server.type, 'streamable-http');
  assert.equal(server.url, "https://mcp.linear.app/mcp/readonly");
  assert.equal(server.command, undefined);
  assert.equal(server.args, undefined);
  assert.equal(new URL(server.url).username, '');
});

test('uses OAuth without a manual token field', () => {
  assert.equal(manifest.extensions['dev.sailry.platform'].settings_schema, undefined);
  assert.deepEqual(mcp.mcpServers.linear.headers ?? {}, {});
  assert.deepEqual(manifest.extensions['dev.sailry.platform'].connectors[0].auth,[{kind:'mcp_oauth',server:'linear'}]);
});

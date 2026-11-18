import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

const manifest = JSON.parse(readFileSync(new URL('./plugin.json', import.meta.url)));
const mcp = JSON.parse(readFileSync(new URL('./mcp.json', import.meta.url)));
const extension = manifest.extensions['dev.sailry.platform'];
const hostSource = readFileSync(new URL('./dev.sailry.platform/host/connection.js', import.meta.url), 'utf8');
const {fallback} = await import('./dev.sailry.platform/desktop/locales.js');

async function host(configured) {
  const source = hostSource.replace(/^import .*;\s*$/gm, '').replace('export async function connection', 'async function connection');
  const {create} = await import(`data:text/javascript,${encodeURIComponent(`export function create(status,card,fallback){${source}\nreturn connection;}`)}`);
  const reads = [];
  const connection = create(async server => { reads.push(server); return configured ? {enabled:true,state:'connected'} : null; }, (key,props,fallback) => ({kind:'card',key,props,fallback}), fallback);
  const result = await connection();
  assert.deepEqual(reads, ['notion']);
  return result;
}

test('declares the official remote connection', () => {
  assert.equal(manifest.name, "notion");
  assert.equal(manifest.extensions['dev.sailry.platform'].api_version, 'v1');
  const server = mcp.mcpServers["notion"];
  assert.equal(server.type, 'streamable-http');
  assert.equal(server.url, "https://mcp.notion.com/mcp");
  assert.equal(server.command, undefined);
  assert.equal(server.args, undefined);
  assert.equal(new URL(server.url).username, '');
});

test('declares a card-only entry with native named targets', () => {
  assert.equal(manifest.version, '0.1.10');
  assert.equal(extension.desktop.entry, undefined);
  assert.equal(extension.desktop.navigation, undefined);
  assert.equal(extension.tools[0].handler.read_only, true);
  assert.equal(extension.tools[0].presentation, 'content');
  const card = extension.cards[0];
  assert.equal(card.key, 'connection');
  assert.equal(card.mcp_server, 'notion');
  assert.ok(extension.desktop.resources.includes(card.entry));
  assert.equal(card.props_schema.additionalProperties, false);
  assert.deepEqual(Object.keys(card.props_schema.properties), ['configured']);
  assert.equal(card.capabilities.qr, undefined, 'Notion does not expose device-QR authorization');
  assert.deepEqual(card.capabilities.browser[0].modes, ['embedded', 'external']);
  assert.deepEqual(card.capabilities.browser[0].source, {kind:'mcp_authorization'});
  assert.ok(card.capabilities.browser[0].origins.includes('https://app.notion.com'));
  assert.ok(card.capabilities.browser[0].origins.every(origin => new URL(origin).origin === origin));
});

test('disconnected status returns a neutral public draft', async () => {
  const result = await host(false);
  assert.equal(result.isError, undefined);
  assert.equal(result.error, undefined);
  assert.deepEqual(result.sailry_content.blocks, [{kind:'card',key:'connection',props:{configured:false},fallback:fallback('connect')}]);
  assert.equal(result.sailry_content.blocks[0].receipt, undefined, 'only Node stamps ownership');
  const connected = await host(true);
  assert.equal(connected.error, undefined);
  assert.equal(connected.isError, undefined);
  assert.equal(connected.sailry_content.blocks[0].props.configured, true);
});

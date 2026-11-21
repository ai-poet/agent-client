import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import test from 'node:test';
const manifest = JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
const extension = manifest.extensions['dev.sailry.platform'];
const configuration = JSON.parse(await readFile(new URL('../mcp.json',import.meta.url),'utf8'));
test('optional API key uses a protected connector', () => {
  assert.equal(manifest.version,'0.1.7');
  assert.equal(extension.settings_schema,undefined);
  const connector = extension.connectors[0], field = connector.fields[0];
  assert.equal(field.name,'api_key'); assert.equal(field.secret,true); assert.equal(field.optional,true);
  assert.equal(field.default,undefined); assert.equal(Object.keys(field.locales).length,10);
  assert.deepEqual(connector.inject,[{server:'context7',header:'Context7-API-Key'}]);
  assert.deepEqual(configuration.mcpServers.context7,{type:'streamable-http',url:'https://mcp.context7.com/mcp'});
});

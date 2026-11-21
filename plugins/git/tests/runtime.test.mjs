import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {execute,packageInput,runtimeTest,validateSchema} from '../../scripts/test-host.mjs';
import {cases} from './parameters.mjs';

const directory = new URL('../',import.meta.url);
const manifest = JSON.parse(await readFile(new URL('plugin.json',directory),'utf8'));
const tools = manifest.extensions['dev.sailry.platform'].tools;

runtimeTest('read pages preserve schema and adapter domains', async () => {
  const rows = cases();
  for (const name of new Set(rows.map(row => row.tool))) {
    const selected = rows.filter(row => row.tool === name);
    const schema = tools.find(tool => tool.name === name).handler.parameters;
    assert.deepEqual(validateSchema(schema,selected.map(row => row.input)),selected.map(row => row.schemaValid ?? row.valid));
  }
  for (const entry of rows) {
    const output = execute(await packageInput(directory,entry.handler,entry.input));
    assert.deepEqual(output.calls,[]);
    if (entry.valid) assert.deepEqual(output.result.Ok,entry.expected);
    else {
      assert.equal(output.result.Ok.isError,true);
      assert.equal(output.result.Ok.error.code,'invalid_request');
    }
  }
});

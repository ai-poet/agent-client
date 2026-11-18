import {createHash} from 'node:crypto';
import {readFile, writeFile} from 'node:fs/promises';

const directory = new URL('../skills/code-review/rules/', import.meta.url);
const index = await readFile(new URL('index.md', directory), 'utf8');
const entries = [...index.matchAll(/^\| `([^`]+)` \| ([\w.]+) \|$/gm)]
  .map(([,pattern,rule]) => ({pattern,rule}));
const files = [...new Set([...entries.map(entry => entry.rule),'default.md','objc.md'])];
const rules = {};
for (const file of files) {
  rules[file] = {path:`rules/${file}`,sha256:createHash('sha256')
    .update(await readFile(new URL(file,directory))).digest('hex')};
}
const source = '// Generated from the ordered rule index; preserve the original rule resources.\n'
  + `export const entries = ${JSON.stringify(entries,null,2)};\n`
  + `export const rules = ${JSON.stringify(rules,null,2)};\n`;
const output = new URL('../dev.sailry.platform/host/rules.js', import.meta.url);
if (process.argv.includes('--check')) {
  if (await readFile(output,'utf8') !== source) throw new Error('Generated review rule metadata is stale');
} else await writeFile(output,source);

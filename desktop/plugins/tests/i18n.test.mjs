import test from 'node:test';
import assert from 'node:assert/strict';
import {readdir, readFile} from 'node:fs/promises';
import {resolve, dirname} from 'node:path';
import {fileURLToPath, pathToFileURL} from 'node:url';
import vm from 'node:vm';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const namespace = 'dev.sailry.platform';
const chinese = /[\u3400-\u9fff]/;
const placeholders = text => [...text.matchAll(/%\{([^}]+)\}/g)].map(match => match[1]).sort();

async function packages(parent) {
  const result = [];
  for (const directory of await readdir(resolve(root, parent), {withFileTypes: true})) {
    if (!directory.isDirectory()) continue;
    const path = resolve(root, parent, directory.name);
    try {
      result.push({path, manifest: JSON.parse(await readFile(resolve(path, 'plugin.json'), 'utf8'))});
    } catch (error) {
      if (error.code !== 'ENOENT') throw error;
    }
  }
  return result;
}

function labels(value, context, localized = true) {
  if (!value || typeof value !== 'object') return;
  if (typeof value.label === 'string') {
    assert.ok(value.label.trim(), context);
    assert.equal(chinese.test(value.label), false, `${context}: English label`);
    const translated = value.locales?.['zh-CN'];
    if (localized || translated !== undefined) {
      assert.ok(translated?.trim(), `${context}: Chinese label`);
    }
    assert.deepEqual(placeholders(value.label), placeholders(translated ?? value.label), context);
  }
  for (const [key, child] of Object.entries(value)) labels(child, `${context}.${key}`, key !== 'display');
}

async function dictionaries(path) {
  const context = vm.createContext({});
  const modules = new Map();
  async function load(id, inspect = false) {
    if (!modules.has(id)) {
      let source = await readFile(fileURLToPath(id), 'utf8');
      if (inspect) source += '\nexport const dictionaries = ' +
        'typeof messages === "function" ? {en:messages("en"),zh:messages("zh-CN")} : ' +
        'typeof locales === "undefined" ? {en, zh} : {en:locales.en, zh:locales["zh-CN"]};';
      modules.set(id, new vm.SourceTextModule(source, {context, identifier:id}));
    }
    return modules.get(id);
  }
  const entry = await load(pathToFileURL(path).href, true);
  await entry.link((id, parent) => load(new URL(id, parent.identifier).href));
  await entry.evaluate();
  return entry.namespace.dictionaries;
}

test('packaged names preserve canonical fallback and descriptions cover both languages', async () => {
  const entries = [...await packages('.'), ...await packages('examples')];
  assert.ok(entries.length, 'packaged plugins must be discovered');
  for (const {manifest} of entries) {
    const extension = manifest.extensions?.[namespace];
    assert.ok(extension?.description, `${manifest.name}: description`);
    labels(extension, manifest.name);
  }
});

test('display names allow fallback without hiding invalid translations', () => {
  labels({display:{label:'Example OAuth'}}, 'example');
  labels({display:{label:'Example OAuth',locales:{'zh-CN':'Example OAuth'}}}, 'example');
  assert.throws(() => labels({display:{label:'Example OAuth',locales:{'zh-CN':''}}}, 'example'), /Chinese label/);
  assert.throws(() => labels({description:{label:'Sign in'}}, 'example'), /Chinese label/);
});

test('desktop dictionaries have matching keys and placeholders without cross-language fallback', async () => {
  for (const {path, manifest} of [...await packages('.'), ...await packages('examples')]) {
    const resource = manifest.extensions?.[namespace]?.desktop?.resources.find(path => path.endsWith('/desktop/locales.js'));
    if (!resource) continue;
    const values = await dictionaries(resolve(path, resource));
    const en = values.en, zh = values.zh;
    assert.deepEqual(Object.keys(en).sort(), Object.keys(zh).sort(), manifest.name);
    for (const [key, english] of Object.entries(en)) {
      if (typeof english !== 'string') continue;
      assert.equal(chinese.test(english), false, `${manifest.name}.${key}: English copy`);
      assert.ok(typeof zh[key] === 'string' && zh[key].trim(), `${manifest.name}.${key}: Chinese copy`);
      assert.deepEqual(placeholders(english), placeholders(zh[key]), `${manifest.name}.${key}`);
    }
  }
});

test('schema-generated settings localize field and tab titles', async () => {
  for (const {path, manifest} of await packages('.')) {
    const resource = manifest.extensions?.[namespace]?.settings_schema;
    if (!resource) continue;
    const schema = JSON.parse(await readFile(resolve(path, resource), 'utf8'));
    const localized = schema['x-sailry-locales']?.['zh-CN'];
    for (const [key, field] of Object.entries(schema.properties)) {
      assert.ok(field.title?.trim(), `${manifest.name}.${key}: field title`);
      assert.ok(localized?.[key]?.trim(), `${manifest.name}.${key}: Chinese field title`);
      assert.equal(chinese.test(field.title), false, `${manifest.name}.${key}: English field title`);
    }
    for (const tab of schema['x-sailry-tabs'] ?? []) {
      assert.ok(localized?.[tab.id]?.trim(), `${manifest.name}.${tab.id}: Chinese tab title`);
    }
  }
});

test('host-authored captions carry both languages', async () => {
  for (const {path, manifest} of await packages('.')) {
    const resources = manifest.extensions?.[namespace]?.host?.resources ?? [];
    for (const resource of resources.filter(path => /\/host\/(locales|labels)\.js$/.test(path))) {
      const source = await readFile(resolve(path, resource), 'utf8');
      const names = [...source.matchAll(/^export const (\w+)/gm)].map(match => match[1]);
      const expressions = names.map(name => `${name}:${name}`);
      if (/export function present\(/.test(source)) {
        expressions.push('blocked:present({state:"blocked",token_budget:1000})',
          'failed:present({state:"failed",token_budget:null})',
          'cancelled:present({state:"cancelled",token_budget:0})');
      }
      const values = vm.runInNewContext(source.replace(/^export /gm, '') + `\n;({${expressions.join(',')}})`);
      labels(values, `${manifest.name}.${resource}`);
    }
  }
});

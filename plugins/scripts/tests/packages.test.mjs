import assert from 'node:assert/strict';
import {execFileSync, spawnSync} from 'node:child_process';
import {copyFileSync, mkdtempSync, mkdirSync, realpathSync, writeFileSync, rmSync, symlinkSync} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import {checkPackage, dependencies, resource} from '../check-packages.mjs';

function fixture(t, extension = {}) {
  const temp = mkdtempSync(path.join(os.tmpdir(), 'sailry-plugin-check-'));
  t.after(() => rmSync(temp, {recursive: true, force: true}));
  const directory = path.join(temp, 'sample');
  mkdirSync(directory);
  const manifest = path.join(directory, 'plugin.json');
  const data = {name: 'sample', version: '0.1.0', extensions: {
    'dev.sailry.platform': {api_version: 'v1', actions: [], ...extension},
  }};
  writeFileSync(manifest, JSON.stringify(data));
  return {temp, directory, manifest, data};
}

test('syntax parsing never executes package initialization', () => {
  assert.deepEqual(dependencies("import {thing} from './helper.js'; throw Error('must not run');", 'main.js'), ['./helper.js']);
  assert.throws(() => dependencies('export function broken(', 'broken.js'), SyntaxError);
});

test('declared imports and SDK modules remain installable', t => {
  const {directory, manifest} = fixture(t, {desktop: {entry: 'main.js', resources: ['helper.js']}});
  writeFileSync(path.join(directory, 'main.js'), "import './helper.js'; import {value} from 'sailry/sdk'; import * as models from 'sailry/models';");
  writeFileSync(path.join(directory, 'helper.js'), 'export const value = 1;');
  assert.equal(checkPackage(manifest), 2);
});

test('missing and undeclared imports fail before plugin execution', t => {
  const {directory, manifest} = fixture(t, {desktop: {entry: 'main.js'}});
  writeFileSync(path.join(directory, 'main.js'), "import './helper.js';");
  assert.throws(() => checkPackage(manifest), /ENOENT/);
  writeFileSync(path.join(directory, 'helper.js'), 'export const value = 1;');
  assert.throws(() => checkPackage(manifest), /not a declared/);
  writeFileSync(path.join(directory, 'main.js'), "import {readFile} from 'node:fs';");
  assert.throws(() => checkPackage(manifest), /Unsupported host module/);
});

test('traversal and symlinks cannot escape package assets', t => {
  const {temp, directory} = fixture(t);
  writeFileSync(path.join(temp, 'outside.js'), '');
  for (const value of ['', '../outside.js', '/outside.js', 'folder/../outside.js', 'C:\\outside.js']) {
    assert.throws(() => resource(directory, value));
  }
  symlinkSync(path.join(temp, 'outside.js'), path.join(directory, 'linked.js'));
  assert.throws(() => resource(directory, 'linked.js'), /escapes the package/);
});

test('identity, contract and duplicate declarations fail clearly', t => {
  const {manifest, data} = fixture(t);
  for (const mutate of [
    value => { value.name = 'another'; },
    value => { value.version = 'unversioned'; },
    value => { value.extensions['dev.sailry.platform'].api_version = 'v2'; },
    value => { value.extensions['dev.sailry.platform'].actions = ['read', 'read']; },
    value => { value.extensions['dev.sailry.platform'].desktop = {resources: ['main.js', 'main.js']}; },
    value => { value.extensions['dev.sailry.platform'].tools = [{name: 'read'}, {name: 'read'}]; },
    value => { value.extensions['dev.sailry.platform'].host = {handlers: ['read', 'read']}; },
  ]) {
    const invalid = structuredClone(data);
    mutate(invalid);
    writeFileSync(manifest, JSON.stringify(invalid));
    assert.throws(() => checkPackage(manifest));
  }
});

test('versions accept previews and reject invalid SemVer identifiers', t => {
  const {manifest, data} = fixture(t);
  for (const value of ['0.1.0', '2.3.4', '0.1.0-alpha.1', '0.1.0-alpha.1+build.01']) {
    writeFileSync(manifest, JSON.stringify({...data, version: value}));
    assert.equal(checkPackage(manifest), 0);
  }
  for (const value of ['01.1.0', '0.01.0', '0.1.0-alpha.01', '0.1.0-bad_name', '0.1.0-']) {
    writeFileSync(manifest, JSON.stringify({...data, version: value}));
    assert.throws(() => checkPackage(manifest), /semantic version/);
  }
});

test('minimum Sailry versions accept previews and reject invalid declarations', t => {
  const {manifest, data} = fixture(t);
  for (const minimum of ['0.1.0-alpha.4', '0.2.0-alpha.1', '1.0.0', '1.0.0+build.01']) {
    const current = structuredClone(data);
    current.extensions['dev.sailry.platform'].min_sailry_version = minimum;
    writeFileSync(manifest, JSON.stringify(current));
    assert.equal(checkPackage(manifest), 0);
  }
  for (const minimum of ['', '>=0.1.0', '0.1.0-alpha.01', '01.0.0', 1, null, {}]) {
    const current = structuredClone(data);
    current.extensions['dev.sailry.platform'].min_sailry_version = minimum;
    writeFileSync(manifest, JSON.stringify(current));
    assert.throws(() => checkPackage(manifest), /Minimum Sailry version must be a semantic version/);
  }
});

test('repository packages and examples accept valid version increments', t => {
  const {temp, manifest, data} = fixture(t);
  const scripts = path.join(temp, 'scripts');
  mkdirSync(scripts);
  const checker = path.join(scripts, 'check-packages.mjs');
  copyFileSync(new URL('../check-packages.mjs', import.meta.url), checker);
  copyFileSync(new URL('../connectors.mjs', import.meta.url), path.join(scripts, 'connectors.mjs'));
  const example = path.join(temp, 'examples', 'sample', 'plugin.json');
  mkdirSync(path.dirname(example), {recursive: true});
  writeFileSync(example, JSON.stringify(data));
  execFileSync('git', ['init', '--quiet'], {cwd: temp});
  execFileSync('git', ['add', '.'], {cwd: temp});

  const check = () => spawnSync(process.execPath, ['--experimental-vm-modules', realpathSync(checker)], {
    cwd: temp, encoding: 'utf8',
  });
  const valid = check();
  assert.equal(valid.status, 0, valid.stderr);
  assert.match(valid.stdout, /Checked 2 packages/);
  for (const filename of [manifest, example]) {
    writeFileSync(filename, JSON.stringify({...data, version: '0.1.1'}));
    const updated = check();
    assert.equal(updated.status, 0, updated.stderr);
    for (const version of ['invalid', '0.01.1']) {
      writeFileSync(filename, JSON.stringify({...data, version}));
      const invalid = check();
      assert.equal(invalid.status, 1);
      assert.ok(invalid.stderr.includes(`${path.relative(temp, filename)}: Package version must be a semantic version`));
    }
    writeFileSync(filename, JSON.stringify({...data, extensions: {
      'dev.sailry.platform': {...data.extensions['dev.sailry.platform'], api_version:'v2'},
    }}));
    const invalid = check();
    assert.equal(invalid.status, 1);
    assert.ok(invalid.stderr.includes('Sailry extension must use v1'));
    writeFileSync(filename, JSON.stringify(data));
  }
});


test('model translations stay pure and preserve declared resources', t => {
  const script = {entry: 'dev.sailry.platform/adapter.js', resources: ['dev.sailry.platform/helper.js']};
  const {directory, manifest} = fixture(t, {
    actions: ['models.provide', 'models.authenticate', 'credentials.inspect'],
    model_providers: [{id: 'sample'}],
    model_adapters: [{id: 'sample', script}],
    model_auth: [{id: 'login', exchange: {script, handler: 'exchange'}}],
  });
  mkdirSync(path.join(directory, 'dev.sailry.platform'));
  writeFileSync(path.join(directory, script.entry), "import './helper.js'; import {version} from 'sailry/adapter'; export const prepare = () => ({version});");
  writeFileSync(path.join(directory, script.resources[0]), 'export const value = 1;');
  assert.equal(checkPackage(manifest), 2);
  writeFileSync(path.join(directory, script.resources[0]), "import * as sdk from 'sailry/sdk'; export const value = sdk;");
  assert.throws(() => checkPackage(manifest), /Unsupported host module/);
});

test('model declarations require explicit actions and confined scripts', t => {
  const script = {entry: 'dev.sailry.platform/main.js', resources: []};
  const {directory, manifest, data} = fixture(t, {model_adapters: [{id: 'sample', script}]});
  mkdirSync(path.join(directory, 'dev.sailry.platform'));
  writeFileSync(path.join(directory, script.entry), 'export const prepare = () => ({});');
  assert.throws(() => checkPackage(manifest), /models.provide/);
  const extension = data.extensions['dev.sailry.platform'];
  extension.actions = ['models.provide'];
  extension.model_adapters.push({id: 'sample', script});
  writeFileSync(manifest, JSON.stringify(data));
  assert.throws(() => checkPackage(manifest), /distinct/);
  extension.model_adapters = [{id: 'sample', script: {entry: '../escape.js'}}];
  writeFileSync(manifest, JSON.stringify(data));
  assert.throws(() => checkPackage(manifest), /namespace/);
  extension.model_adapters = [];
  extension.model_auth = [{id: 'login', exchange: {script, handler: 'exchange'}}];
  writeFileSync(manifest, JSON.stringify(data));
  assert.throws(() => checkPackage(manifest), /models.authenticate/);
  extension.actions.push('models.authenticate');
  writeFileSync(manifest, JSON.stringify(data));
  assert.throws(() => checkPackage(manifest), /credentials.inspect/);
});

import test from 'node:test';
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const script = fileURLToPath(new URL('../skills/plugin-builder/scripts/check_package.py', import.meta.url));
const root = fileURLToPath(new URL('../..', import.meta.url));
const created = [];

test.after(() => {
  for (const directory of created) rmSync(directory, {recursive: true, force: true});
});

function check(directory) {
  const result = spawnSync('python3', [script, directory], {encoding: 'utf8'});
  return {code: result.status, output: result.stdout + result.stderr};
}

function fixture(files, name = 'sample') {
  const parent = mkdtempSync(path.join(tmpdir(), 'plugin-builder-'));
  created.push(parent);
  const directory = path.join(parent, name);
  for (const [file, content] of Object.entries(files)) {
    const target = path.join(directory, file);
    mkdirSync(path.dirname(target), {recursive: true});
    writeFileSync(target, typeof content === 'string' ? content : JSON.stringify(content));
  }
  return directory;
}

const manifest = (extension = {}) => ({
  name: 'sample', version: '0.1.0', description: 'Sample',
  extensions: {'dev.sailry.platform': {api_version: 'v1', actions: [], display: {label: 'Sample'}, ...extension}},
});

const skill = '---\nname: sample\ndescription: Explain the sample\n---\n\nSteps\n';

test('accepts a skill package', () => {
  const result = check(fixture({'plugin.json': manifest(), 'skills/sample/SKILL.md': skill}));
  assert.equal(result.code, 0, result.output);
  assert.match(result.output, /^ok:/m);
});

test('accepts every official package and example', () => {
  const packages = parent => readdirSync(path.join(root, parent), {withFileTypes: true})
    .filter(entry => entry.isDirectory() && existsSync(path.join(root, parent, entry.name, 'plugin.json')))
    .map(entry => path.join(parent, entry.name));
  for (const directory of [...packages(''), ...packages('examples')]) {
    const result = check(path.join(root, directory));
    assert.equal(result.code, 0, `${directory}: ${result.output}`);
  }
});

test('accepts minimum Sailry versions', () => {
  for (const minimum of [null, '0.1.0-alpha.4', '1.0.0', '1.0.0+build.01']) {
    const result = check(fixture({'plugin.json': manifest({min_sailry_version: minimum})}));
    assert.equal(result.code, 0, result.output);
  }
});

test('rejects invalid minimum Sailry versions', () => {
  for (const minimum of [
    '', '>=0.1.0', '0.1.0-alpha.01', '01.0.0', '0.1.0\n', '1.2٢.0',
    1, false, [], {},
  ]) {
    const result = check(fixture({'plugin.json': manifest({min_sailry_version: minimum})}));
    assert.equal(result.code, 1, result.output);
    assert.match(result.output, /min_sailry_version must be a semantic version/);
  }
});

test('accepts declared conversation modes', () => {
  const result = check(fixture({'plugin.json': manifest({
    scope: 'host',
    conversation_options: {
      $schema: 'https://json-schema.org/draft/2020-12/schema',
      type: 'object', additionalProperties: false,
      properties: {mode: {type: 'string', default: '', enum: ['', 'focus', 'review']}},
    },
    conversation_modes: [
      {id: 'focus', option: 'mode', display: {label: 'Focus'},
        permissions: {write: true}, instructions: 'Complete the requested task'},
      {id: 'review', option: 'mode', display: {label: 'Review'},
        permissions: {write: false}, instructions: 'Inspect the requested changes'},
    ],
  })}));
  assert.equal(result.code, 0, result.output);
  assert.match(result.output, /^ok:/m);
});

test('accepts supported catalog metadata', () => {
  for (const [category,tint] of [
    ['development','blue'], ['documents','violet'], ['data','teal'],
    ['automation','amber'], ['entertainment','gray'], [null,null],
  ]) {
    const result = check(fixture({'plugin.json': manifest({category,tint})}));
    assert.equal(result.code, 0, result.output);
  }
});

test('rejects unsupported catalog metadata', () => {
  for (const [field,value] of [
    ['category','unknown'], ['category',[]], ['category',false],
    ['tint','red'], ['tint',{}], ['tint',0],
  ]) {
    const result = check(fixture({'plugin.json': manifest({[field]:value})}));
    assert.equal(result.code, 1, result.output);
    assert.match(result.output, new RegExp(`${field} must be one of:`));
  }
});

test('rejects a name that differs from the folder', () => {
  const result = check(fixture({'plugin.json': {...manifest(), name: 'other'}}));
  assert.equal(result.code, 1);
  assert.match(result.output, /must equal the folder name/);
});

test('rejects unknown actions and extension fields', () => {
  const result = check(fixture({'plugin.json': manifest({actions: ['files.everything'], network: true})}));
  assert.match(result.output, /unknown action: files\.everything/);
  assert.match(result.output, /unknown extension field: network/);
});

test('accepts team catalog and decision actions', () => {
  for (const action of ['teams.read', 'decisions.run']) {
    const result = check(fixture({'plugin.json': manifest({actions: [action]})}));
    assert.equal(result.code, 0, `${action}: ${result.output}`);
  }
});

test('accepts declared system permissions', () => {
  const result = check(fixture({'plugin.json': manifest({
    permissions: [{kind: 'microphone', subject: 'sailry'}],
  })}));
  assert.equal(result.code, 0, result.output);
});

test('accepts connector declarations and modules', () => {
  const result = check(fixture({
    'plugin.json': manifest({
      connectors: [],
      host: {
        entry: 'dev.sailry.platform/host/main.js',
        resources: ['dev.sailry.platform/host/main.js'],
        handlers: ['request'],
      },
    }),
    'dev.sailry.platform/host/main.js':
      'import * as http from "sailry/http";\nimport * as connectors from "sailry/connectors";\nexport async function request() { return {http, connectors}; }\n',
  }));
  assert.equal(result.code, 0, result.output);
});

test('accepts the registration declaration and module', () => {
  const directory = fixture({
    'plugin.json': manifest({
      registration: {
        provider: 'feishu', client_id: 'app_id', client_secret: 'app_secret',
        tenant_scopes: ['docs:document.content:read'],
      },
      desktop: {resources: ['dev.sailry.platform/desktop/settings.js']},
    }),
    'dev.sailry.platform/desktop/settings.js':
      'import {RegistrationButton} from "sailry/registration";\nexport default RegistrationButton;\n',
  });
  const result = check(directory);
  assert.equal(result.code, 0, result.output);
});

test('accepts registered settings and installation modules', () => {
  const result = check(fixture({
    'plugin.json': manifest({desktop: {resources: ['dev.sailry.platform/settings.js']}}),
    'dev.sailry.platform/settings.js':
      'import * as settings from "sailry/settings";\nimport * as installations from "sailry/installations";\nexport {settings, installations};\n',
  }));
  assert.equal(result.code, 0, result.output);
});

test('rejects undeclared imports and unavailable modules', () => {
  const directory = fixture({
    'plugin.json': manifest({desktop: {
      entry: 'dev.sailry.platform/desktop/main.js',
      resources: ['dev.sailry.platform/desktop/main.js'],
      panel: {label: 'Sample'},
    }}),
    'dev.sailry.platform/desktop/main.js': 'import {View} from "gpui-kit";\nimport left from "left-pad";\nimport {text} from "./text.js";\n',
    'dev.sailry.platform/desktop/text.js': 'export const text = "";\n',
  });
  const result = check(directory);
  assert.equal(result.code, 1);
  assert.match(result.output, /unavailable module: left-pad/);
  assert.match(result.output, /undeclared resource: \.\/text\.js/);
  assert.doesNotMatch(result.output, /gpui-kit/);
});

test('rejects resources outside the extension folder', () => {
  const directory = fixture({
    'plugin.json': manifest({host: {entry: 'host/main.js', resources: ['host/main.js'], handlers: ['run']}}),
    'host/main.js': 'export function run() {}\n',
  });
  assert.match(check(directory).output, /must be under dev\.sailry\.platform\//);
});

test('rejects script tools without a registered handler', () => {
  const directory = fixture({
    'plugin.json': manifest({
      tools: [{name: 'count', handler: {name: 'count', parameters: {type: 'object'}}}],
      host: {entry: 'dev.sailry.platform/host/main.js', resources: ['dev.sailry.platform/host/main.js'], handlers: []},
    }),
    'dev.sailry.platform/host/main.js': 'export function count() { return {}; }\n',
  });
  assert.match(check(directory).output, /handler 'count' is not in host\.handlers/);
});

test('requires the action behind a bound operation', () => {
  const result = check(fixture({'plugin.json': manifest({tools: [{name: 'save_note', operation: 'storage.set'}]})}));
  assert.match(result.output, /needs the storage\.write action/);
});

const operations = [
  ['models.install', 'models.install'], ['models.remove', 'models.install'],
  ['code.search', 'code.index.read'], ['code.outline', 'code.index.read'],
  ['files.read_lines', 'files.read'],
  ['files.edit', 'files.write'],
  ['files.find', 'files.read'],
  ['git.show', 'git.read'],
  ['git.blame', 'git.read'],
  ['git.changes', 'git.read'],
  ['git.change', 'git.read'],
];

function bindings(operation) {
  return [
    {name: 'invoke', operation},
    {name: 'invoke', handler: {name: 'invoke', operation, parameters: {type: 'object'}}},
  ];
}

function flow(operations) {
  return {name: 'invoke', description: 'Read indexed context', handler: {
    name: 'invoke', read_only: true, flow: {operations}, parameters: {type: 'object'},
  }};
}

function operationPackage(tool, actions) {
  return fixture({
    'plugin.json': manifest({
      actions,
      tools: [tool],
      host: {
        entry: 'dev.sailry.platform/host/main.js',
        resources: ['dev.sailry.platform/host/main.js'],
        handlers: ['invoke'],
      },
    }),
    'dev.sailry.platform/host/main.js': 'export function invoke(args) { return args; }\n',
  });
}

for (const [operation, action] of operations) {
  test(`requires ${action} for ${operation}`, () => {
    const unrelated = action === 'files.write' ? 'files.read' : 'files.write';
    for (const tool of bindings(operation)) {
      for (const actions of [[], [unrelated]]) {
        const result = check(operationPackage(tool, actions));
        assert.equal(result.code, 1, result.output);
        assert.ok(result.output.includes(`needs the ${action} action`), result.output);
      }
    }
  });

  test(`accepts ${action} for ${operation}`, () => {
    for (const tool of bindings(operation)) {
      const result = check(operationPackage(tool, [action]));
      assert.equal(result.code, 0, result.output);
      assert.match(result.output, /^ok:/m);
    }
  });
}

test('accepts current code index capabilities and bindings', () => {
  for (const scope of [undefined, 'host']) {
    const result = check(fixture({'plugin.json': manifest({
      scope, enabled_service: 'code_index',
      actions: ['code.index.read', 'code.index.settings.read', 'code.index.settings.write'],
    })}));
    assert.equal(result.code, 0, result.output);
  }
});

test('rejects unavailable code index bindings', () => {
  for (const [extension, message] of [
    [{enabled_service: 'language'}, 'enabled_service must be code_index'],
    [{enabled_service: 'unknown'}, 'enabled_service must be code_index'],
    [{enabled_service: 'code_index'}, 'needs the code.index.read action'],
    [{enabled_service: 'code_index', scope: 'desktop', actions: ['code.index.read']}, 'requires host scope'],
  ]) {
    const result = check(fixture({'plugin.json': manifest(extension)}));
    assert.equal(result.code, 1, result.output);
    assert.ok(result.output.includes(message), result.output);
  }
});

test('requires every context flow capability', () => {
  const tool = flow(['code.search', 'code.outline', 'files.read_lines']);
  for (const [actions, missing] of [
    [[], 'code.index.read'], [['files.read'], 'code.index.read'], [['code.index.read'], 'files.read'],
  ]) {
    const result = check(operationPackage(tool, actions));
    assert.equal(result.code, 1, result.output);
    assert.ok(result.output.includes(`needs the ${missing} action`), result.output);
  }
  const result = check(operationPackage(tool, ['code.index.read', 'files.read']));
  assert.equal(result.code, 0, result.output);
});

test('rejects retired model tool declarations', () => {
  const result = check(fixture({'plugin.json': manifest({
    model_tools: [{capability: 'web_search', display: {label: 'Search'}}],
  })}));
  assert.equal(result.code, 1, result.output);
  assert.match(result.output, /unknown extension field: model_tools/);
});

test('rejects retired language capabilities', () => {
  for (const action of ['code.read', 'code.manage', 'code.settings.read', 'code.settings.write']) {
    const result = check(fixture({'plugin.json': manifest({actions: [action]})}));
    assert.equal(result.code, 1, result.output);
    assert.ok(result.output.includes(`unknown action: ${action}`), result.output);
  }
});

test('rejects retired operations in every binding', () => {
  for (const operation of [
    'code.packages', 'code.catalog', 'code.servers', 'code.start', 'code.stop',
    'code.install', 'code.uninstall', 'code.diagnostics', 'code.definition',
    'code.references', 'code.hover', 'code.symbols',
  ]) {
    for (const tool of [...bindings(operation), flow([operation])]) {
      const result = check(operationPackage(tool, ['code.index.read', 'files.read']));
      assert.equal(result.code, 1, result.output);
      assert.ok(result.output.includes(`retired operation: ${operation}`), result.output);
    }
  }
});

test('limits initial tools to read-only or package-storage effects', () => {
  const actions = ['storage.read', 'storage.write', 'files.write', 'conversation.message.read'];
  const plain = {name: 'invoke', description: 'Invoke', handler: {name: 'invoke', parameters: {type: 'object'}}};
  for (const tool of [
    {...flow(['storage.get']), initial: true},
    {...flow(['storage.transaction']), handler: {...flow([]).handler, read_only: false, flow: {operations: ['storage.transaction']}}, initial: true},
    {name: 'invoke', operation: 'storage.get', initial: true},
    {...plain, handler: {...plain.handler, read_only: true}, initial: true},
  ]) {
    const result = check(operationPackage(tool, actions));
    assert.equal(result.code, 0, result.output);
  }
  for (const tool of [
    {...flow(['storage.set', 'files.write']), initial: true},
    {...plain, initial: true},
    {...flow(['storage.get']), initial: 'yes'},
  ]) {
    const result = check(operationPackage(tool, actions));
    assert.equal(result.code, 1, result.output);
    assert.match(result.output, /initial/);
  }
});

test('rejects a skill whose name differs from its folder', () => {
  const result = check(fixture({'plugin.json': manifest(), 'skills/notes/SKILL.md': skill}));
  assert.match(result.output, /skills\/notes: name must equal the folder name/);
});


test('validates declared authentication scripts', () => {
  const auth = {id:'example',flow:{kind:'script',script:{entry:'dev.sailry.platform/auth.js'},handler:'step'}};
  const files = actions => ({'plugin.json':manifest({actions,model_auth:[auth]}),
    'dev.sailry.platform/auth.js':'export function step() { return {}; }'});
  assert.equal(check(fixture(files(['models.authenticate','credentials.inspect']))).code,0);
  assert.match(check(fixture(files(['models.authenticate']))).output,/require credentials.inspect/);
  assert.match(check(fixture({...files(['models.authenticate','credentials.inspect']),
    'dev.sailry.platform/auth.js':'import x from "sailry/settings"; export {x};'})).output,/unavailable module/);
  const missing=files(['models.authenticate','credentials.inspect']);
  delete missing['dev.sailry.platform/auth.js'];
  assert.match(check(fixture(missing)).output,/does not exist/);
});

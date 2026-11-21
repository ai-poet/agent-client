/** Validate committed package assets without linking or executing plugin code. */
import {execFileSync} from 'node:child_process';
import {readFileSync, realpathSync, statSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {SourceTextModule} from 'node:vm';
import {checkConnectors} from './connectors.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const number = '(?:0|[1-9]\\d*)';
const preview = '(?:0|[1-9]\\d*|\\d*[A-Za-z-][0-9A-Za-z-]*)';
const version = new RegExp(`^${number}\\.${number}\\.${number}(?:-${preview}(?:\\.${preview})*)?(?:\\+[0-9A-Za-z-]+(?:\\.[0-9A-Za-z-]+)*)?$`);
const modules = new Set([
  'gpui-base', 'gpui-component', 'gpui-kit', 'sailry', 'sailry/ui',
  'sailry/search', 'sailry/settings', 'sailry/forms', 'sailry/sdk', 'sailry/connections', 'sailry/credentials', 'sailry/models',
  'sailry/card', 'sailry/cards', 'sailry/connectors', 'sailry/http', 'sailry/decisions',
  'sailry/documents', 'sailry/file-transfers', 'sailry/ssh-transfers', 'sailry/artifacts',
]);

export function dependencies(source, identifier) {
  // Construction parses syntax; do not link, instantiate or evaluate modules.
  return new SourceTextModule(source, {identifier}).moduleRequests.map(request => request.specifier);
}

function distinct(values, label) {
  if (!Array.isArray(values) || values.some(value => typeof value !== 'string' || !value)
      || new Set(values).size !== values.length) {
    throw new Error(`${label} must contain distinct nonempty strings`);
  }
}

export function resource(directory, value) {
  if (typeof value !== 'string' || !value || /[\\:]/.test(value)
      || value.split('/').some(part => !part || part === '.' || part === '..')) {
    throw new Error(`Invalid package resource: ${value}`);
  }
  const target = path.resolve(directory, value), relative = path.relative(directory, target);
  if (relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative) || !statSync(target).isFile()) {
    throw new Error(`Resource must be a file inside the package: ${value}`);
  }
  const actual = path.relative(realpathSync(directory), realpathSync(target));
  if (actual === '..' || actual.startsWith(`..${path.sep}`) || path.isAbsolute(actual)) {
    throw new Error(`Resource escapes the package: ${value}`);
  }
  return target;
}

export function checkPackage(manifest) {
  const directory = path.dirname(manifest), data = JSON.parse(readFileSync(manifest, 'utf8'));
  if (data.name !== path.basename(directory) || !/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(data.name)) {
    throw new Error('Package name must match its directory');
  }
  if (typeof data.version !== 'string' || !version.test(data.version)) {
    throw new Error('Package version must be a semantic version');
  }
  const extension = data.extensions?.['dev.sailry.platform'];
  if (!extension || extension.api_version !== 'v1') throw new Error('Sailry extension must use v1');
  if (extension.min_sailry_version !== undefined
      && (typeof extension.min_sailry_version !== 'string' || !version.test(extension.min_sailry_version))) {
    throw new Error('Minimum Sailry version must be a semantic version');
  }
  distinct(extension.actions ?? [], 'Actions');
  checkConnectors(extension, directory);
  const sections = [extension.host, extension.desktop, extension.settings_page].filter(Boolean);
  for (const section of sections) distinct(section.resources ?? [], 'Resources');
  distinct(extension.host?.handlers ?? [], 'Host handlers');
  if (extension.tools) distinct(extension.tools.map(tool => tool.name), 'Tools');
  for (const [key, label] of [['model_providers', 'Model providers'], ['model_auth', 'Model authentication'], ['model_adapters', 'Model adapters']]) {
    distinct((extension[key] ?? []).map(item => item.id), label);
  }
  const pure = [
    ...(extension.connectors ?? []).flatMap(connector => connector.auth.flatMap(auth => auth.registration?.flow?.kind === 'script' ? [auth.registration.flow.script] : [])),
    // An adapter over a declared program has no translation script.
    ...(extension.model_adapters ?? []).filter(adapter => !adapter.acp).map(adapter => adapter.script),
    ...(extension.model_auth ?? []).flatMap(auth => [
      ...(auth.exchange ? [auth.exchange.script] : []),
      ...(auth.flow?.kind === 'script' ? [auth.flow.script] : []),
    ]),
  ];
  if (pure.some(script => !script || typeof script.entry !== 'string')) throw new Error('Model translations must declare a script');
  if (extension.model_adapters?.length && !extension.actions.includes('models.provide')) throw new Error('Model adapters require models.provide');
  distinct((extension.programs ?? []).map(program => program.id), 'Programs');
  if (extension.programs?.length && !extension.actions.includes('process.run')) throw new Error('Programs require process.run');
  for (const adapter of extension.model_adapters ?? []) {
    if (adapter.acp && (adapter.script || !extension.programs?.some(program => program.id === adapter.acp.program))) {
      throw new Error('Agent adapters name a declared program and no script');
    }
  }
  if (extension.model_auth?.length && !extension.actions.includes('models.authenticate')) throw new Error('Model authentication requires models.authenticate');
  if (extension.model_auth?.some(auth => auth.exchange || auth.flow?.kind === 'script') && !extension.actions.includes('credentials.inspect')) throw new Error('Credential transformations require credentials.inspect');
  const pureFiles = new Set(), pureScopes = [];
  for (const script of pure) {
    distinct(script.resources ?? [], 'Model resources');
    const scope = new Set();
    pureScopes.push(scope);
    for (const value of [script.entry, ...(script.resources ?? [])]) {
      if (!value.startsWith('dev.sailry.platform/')) throw new Error('Model translations must stay in their namespace');
      const filename = resource(directory, value);
      pureFiles.add(filename);
      scope.add(filename);
    }
  }
  const declared = new Set(sections.flatMap(section => [
    ...(section.resources ?? []), ...['entry', 'ui_entry'].flatMap(key => section[key] ? [section[key]] : []),
  ]));
  const files = new Set([...declared].map(value => resource(directory, value)));
  for (const filename of pureFiles) files.add(filename);
  for (const filename of files) {
    if (!filename.endsWith('.js') && !filename.endsWith('.mjs')) continue;
    for (const specifier of dependencies(readFileSync(filename, 'utf8'), filename)) {
      if (!specifier.startsWith('.')) {
        const supported = pureFiles.has(filename) ? specifier === 'sailry/adapter' : modules.has(specifier);
        if (!supported) throw new Error(`Unsupported host module: ${specifier}`);
        continue;
      }
      const value = path.relative(directory, path.resolve(path.dirname(filename), specifier)).split(path.sep).join('/');
      const imported = resource(directory, value);
      if (!files.has(imported)) throw new Error(`Import is not a declared package resource: ${value}`);
      if (pureScopes.some(scope => scope.has(filename) && !scope.has(imported))) {
        throw new Error(`Import is not a declared model resource: ${value}`);
      }
    }
  }
  return files.size;
}

function main() {
  const files = execFileSync('git', ['ls-files', '-z'], {cwd: root, encoding: 'utf8'}).split('\0').filter(Boolean);
  let packages = 0, resources = 0, sources = 0;
  for (const file of files) {
    const filename = path.join(root, file);
    try {
      if (file.endsWith('.json')) JSON.parse(readFileSync(filename, 'utf8'));
      if (file.endsWith('.js') || file.endsWith('.mjs')) {
        dependencies(readFileSync(filename, 'utf8'), filename);
        ++sources;
      }
      if (path.basename(file) === 'plugin.json') {
        resources += checkPackage(filename);
        ++packages;
      }
    } catch (error) {
      throw new Error(`${file}: ${error.message}`);
    }
  }
  if (!packages || !sources) throw new Error('No plugin packages or JavaScript sources found');
  console.log(`Checked ${packages} packages, ${resources} resources and ${sources} JavaScript sources`);
}

if (path.resolve(process.argv[1] ?? '') === fileURLToPath(import.meta.url)) {
  try { main(); } catch (error) { console.error(error.message); process.exitCode = 1; }
}

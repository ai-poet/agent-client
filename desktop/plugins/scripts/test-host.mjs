/** Optional production moejs/SDK bridge; fixture services are not a real Node. */
import {execFileSync} from 'node:child_process';
import {readFile, realpath} from 'node:fs/promises';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import test from 'node:test';

const binary = process.env.SAILRY_PLUGIN_TEST_HOST;
export function runtimeTest(name, run) {
  return test(name, {skip: binary ? false : 'Production test host not selected; use --runtime-host'}, run);
}

export function execute(input) {
  return invoke(input, []);
}

/** Runs the production pure translator without a Node, SDK or network. */
export function translate(input) {
  return invoke(input, ['--translation']);
}

/** Validates raw JSON Schema offline, not a provider's normalized declaration. */
export function validateSchema(schema, values) {
  return invoke({schema, values}, ['--validate-schema']).valid;
}

function invoke(input, args) {
  if (!binary) throw new Error('Production test host is unavailable; use --runtime-host');
  const output = execFileSync(binary, args, {
    input: JSON.stringify(input), encoding: 'utf8', timeout: 65_000, maxBuffer: 16 * 1024 * 1024,
  });
  const result = JSON.parse(output);
  if (result.Err) throw new Error(`Production test host rejected the fixture: ${result.Err.message}`);
  if (!result.Ok) throw new Error('Production test host returned no report');
  return result.Ok;
}

/** Loads only the package's declared host modules; assertions remain in its tests. */
export async function packageInput(directory, handler, input, overrides = {}) {
  const root = await realpath(fileURLToPath(directory));
  const manifest = JSON.parse(await readFile(path.join(root, 'plugin.json'), 'utf8'));
  const host = manifest.extensions?.['dev.sailry.platform']?.host;
  if (!host?.resources?.includes(host.entry)) throw new Error('Package has no declared host entry');
  const files = {};
  for (const name of host.resources) {
    if (!name.endsWith('.js') && !name.endsWith('.mjs')) continue;
    if (name.includes('\\') || path.isAbsolute(name) || name.split('/').some(part => !part || part === '.' || part === '..')) {
      throw new Error('Invalid host fixture resource');
    }
    const filename = await realpath(path.join(root, name));
    const relative = path.relative(root, filename);
    if (relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) {
      throw new Error('Host fixture resource escapes its package');
    }
    files[name] = await readFile(filename, 'utf8');
  }
  return {entry: host.entry, files, handler, input, replies: [],
    context: {package: {name: manifest.name, digest: 'f'.repeat(64), settings_revision: 0},
      session: '00000000-0000-4000-8000-000000000001',
      worktree: '00000000-0000-4000-8000-000000000002'},
    policy: null, ...overrides};
}


/** Loads an explicit pure script declaration; package tests own the handler and payload. */
export async function translationInput(directory, script, handler, input) {
  const root = await realpath(fileURLToPath(directory));
  if (!script || typeof script.entry !== 'string') throw new Error('Package has no model translation entry');
  const files = {};
  for (const name of new Set([script.entry, ...(script.resources ?? [])])) {
    if (typeof name !== 'string' || !name.startsWith('dev.sailry.platform/') || name.includes('\\') || path.isAbsolute(name)
        || name.split('/').some(part => !part || part === '.' || part === '..')) {
      throw new Error('Invalid model translation resource');
    }
    const filename = await realpath(path.join(root, name));
    const relative = path.relative(root, filename);
    if (relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) {
      throw new Error('Model translation resource escapes its package');
    }
    if (name.endsWith('.js') || name.endsWith('.mjs')) files[name] = await readFile(filename, 'utf8');
  }
  return {entry: script.entry, files, handler, input};
}

/** Package-time mirror of the public connector declaration boundaries. */
import {isIP} from 'node:net';
import {readFileSync} from 'node:fs';
import path from 'node:path';

const identifier = value => typeof value === 'string' && /^[a-z0-9_-]{1,64}$/.test(value);
const pointer = value => typeof value === 'string' && value.startsWith('/') && value.length <= 256 && !/[\x00-\x1f\x7f]/.test(value);
const environment = value => typeof value === 'string' && /^[A-Z][A-Z0-9_]{0,63}$/.test(value);
const header = value => typeof value === 'string' && /^[!#$%&'*+.^_`|~0-9A-Za-z-]{1,128}$/.test(value)
  && !/^(host|content-length|transfer-encoding|connection|cookie|upgrade)$/i.test(value) && !/^proxy-/i.test(value);
function assert(value, message) { if (!value) throw new Error(`Connector: ${message}`); }
function keys(value, allowed) { assert(value && typeof value === 'object' && !Array.isArray(value) && Object.keys(value).every(key => allowed.includes(key)), 'unknown declaration field'); }
function distinct(values) { return Array.isArray(values) && new Set(values).size === values.length; }
function scopes(values = []) { return distinct(values) && values.length <= 64 && values.every(value => typeof value === 'string' && value.length > 0 && value.length <= 256 && !/\s/.test(value)); }
function origin(value) {
  try {
    const url = new URL(value);
    return url.protocol === 'https:' && !url.username && !url.password && url.href === `${url.origin}/` && value === url.origin
      && !isIP(url.hostname.replace(/^\[|\]$/g, '')) && url.hostname.includes('.')
      && !/(^|\.)(localhost|local|internal|lan|home)$/.test(url.hostname);
  } catch { return false; }
}
function publicUrl(value) {
  try { const url = new URL(value); return url.protocol === 'https:' && !url.username && !url.password && !url.hash; } catch { return false; }
}
function argv(values, account) {
  return Array.isArray(values) && values.length > 0 && values.length <= 16 && values.every(value => {
    if (typeof value !== 'string' || value.length > 256 || value.includes('\0')) return false;
    return !/[{}]/.test(account ? value.replaceAll('{account}', 'account') : value);
  });
}

export function checkConnectors(extension, directory) {
  const declarations = extension.connectors ?? [];
  assert(Array.isArray(declarations) && declarations.length <= 8 && distinct(declarations.map(value => value.id)), 'too many or duplicate connectors');
  const config = declarations.some(item => item.inject?.some(target => target.server))
    ? JSON.parse(readFileSync(path.join(directory, 'mcp.json'), 'utf8')).mcpServers : {};
  const settings = extension.settings_schema ? JSON.parse(readFileSync(path.join(directory, extension.settings_schema), 'utf8')) : null;
  for (const item of declarations) {
    keys(item, ['id','display','accounts','origins','auth','fields','inject','identity','health']);
    assert(identifier(item.id) && item.accounts === 'single' && item.display?.label?.trim(), 'invalid identity or account policy');
    assert(distinct(item.origins) && item.origins.length > 0 && item.origins.length <= 16 && item.origins.every(origin), 'origins must be public canonical HTTPS origins');
    const fields = item.fields ?? [];
    assert(Array.isArray(fields) && fields.length <= 8 && distinct(fields.map(field => field.name)), 'too many or duplicate fields');
    const secret = name => fields.some(field => field.name === name && field.secret === true);
    const clientFields = value => {
      keys(value, ['client_id','client_secret']);
      return fields.some(field => field.name === value.client_id && !field.secret) && (value.client_secret == null || secret(value.client_secret));
    };
    const client = value => {
      keys(value, ['bundled','fields']);
      if (value.bundled) {
        keys(value.bundled, ['client_id']);
        return !value.fields && typeof value.bundled.client_id === 'string' && value.bundled.client_id.length > 0 && value.bundled.client_id.length <= 2048 && !/[\x00-\x1f\x7f]/.test(value.bundled.client_id);
      }
      return clientFields(value.fields);
    };
    const endpoint = value => {
      if (typeof value !== 'string') return false;
      for (const field of fields.filter(field => !field.secret)) value = value.replaceAll(`{${field.name}}`, 'tenant');
      try { return !/[{}]/.test(value) && publicUrl(value) && item.origins.includes(new URL(value).origin); } catch { return false; }
    };
    for (const field of fields) {
      keys(field, ['name','title','locales','secret','optional','default','help']);
      assert(identifier(field.name) && typeof field.title === 'string' && field.title.trim() && field.title.length <= 256, 'invalid field');
      assert(Object.values(field.locales ?? {}).every(value => typeof value === 'string' && value.trim() && value.length <= 256), 'invalid localized field title');
      assert(field.default == null || (!field.secret && typeof field.default === 'string' && field.default.length <= 2048), 'secret defaults are forbidden');
      assert(field.help == null || publicUrl(field.help), 'invalid help URL');
      assert(!settings?.properties?.[field.name]?.['x-sailry-secret'], 'connector field duplicates a protected settings slot');
    }
    assert(Array.isArray(item.auth) && item.auth.length > 0 && item.auth.length <= 4 && distinct(item.auth.map(auth => auth.kind)), 'too many or duplicate methods');
    for (const auth of item.auth) {
      switch (auth.kind) {
        case 'oauth2':
          keys(auth, ['kind','flow','authorize','token','device','client','scopes','refresh']);
          assert(['pkce','device_code'].includes(auth.flow) && endpoint(auth.authorize) && endpoint(auth.token) && (auth.device == null || endpoint(auth.device)) && (auth.flow !== 'device_code' || endpoint(auth.device)) && client(auth.client) && scopes(auth.scopes), 'invalid OAuth method');
          break;
        case 'application': {
          keys(auth, ['kind','fields','token','registration']);
          keys(auth.token, ['url','body','access','expires_in','form']);
          assert(clientFields(auth.fields) && endpoint(auth.token.url) && pointer(auth.token.access) && pointer(auth.token.expires_in) && Object.keys(auth.token.body ?? {}).length <= 16 && Object.values(auth.token.body ?? {}).every(value => ['{client_id}','{client_secret}'].includes(value)), 'invalid application token exchange');
          if (auth.registration) {
            const registration = auth.registration;
            keys(registration, ['flow','token','issues','scopes']);
            assert(clientFields(registration.issues) && scopes(registration.scopes) && pointer(registration.token?.access), 'invalid registration');
            const visit = value => {
              if (Array.isArray(value)) return value.every(visit);
              if (value && typeof value === 'object') return Object.entries(value).every(([key, child]) => ['url','authorize','issuer'].includes(key) && child != null ? endpoint(child) : visit(child));
              return true;
            };
            assert(visit(registration.flow), 'registration endpoint is outside its origins');
          }
          break;
        }
        case 'secret':
          keys(auth, ['kind','fields']);
          assert(distinct(auth.fields) && auth.fields.length > 0 && auth.fields.length <= 4 && auth.fields.every(secret), 'invalid secret references');
          break;
        case 'mcp_oauth':
          keys(auth, ['kind','server','client']);
          assert(identifier(auth.server) && (auth.client == null || client(auth.client)), 'invalid MCP OAuth');
          assert(config?.[auth.server]?.url && endpoint(config[auth.server].url), 'MCP OAuth server is unavailable or outside its origins');
          assert(item.inject?.some(target => target.server === auth.server && target.header?.toLowerCase() === 'authorization'), 'MCP OAuth requires its protected header');
          break;
        case 'local_cli':
          keys(auth, ['kind','command','env','status','token','timeout_ms','max_output']);
          keys(auth.status, ['argv','account','condition']); keys(auth.token, ['argv','parse']);
          assert(typeof auth.command === 'string' && /^[a-z0-9][a-z0-9._-]{0,63}$/.test(auth.command), 'CLI executable must be a fixed filename');
          assert(Object.entries(auth.env ?? {}).length <= 8 && Object.entries(auth.env ?? {}).every(([name,value]) => environment(name) && !['PATH','HOME','USERPROFILE','APPDATA','LOCALAPPDATA'].includes(name) && typeof value === 'string' && value.length <= 2048 && !value.includes('\0')), 'invalid CLI environment');
          if(auth.status.condition!==undefined){const condition=auth.status.condition;keys(condition,['pointer','values']);assert(pointer(condition.pointer) && Array.isArray(condition.values) && condition.values.length>0 && condition.values.length<=8 && condition.values.every(value=>typeof value==='boolean' || Number.isSafeInteger(value) || typeof value==='string' && value.length<=256 && !/[\x00-\x1f\x7f]/.test(value)), 'invalid CLI account condition');}
          assert(argv(auth.status.argv, false) && pointer(auth.status.account) && argv(auth.token.argv, true), 'invalid fixed CLI arguments');
          assert(auth.token.parse === 'line' || (auth.token.parse && Object.keys(auth.token.parse).length === 1 && pointer(auth.token.parse.json)), 'invalid CLI token parser');
          assert(Number.isInteger(auth.timeout_ms) && auth.timeout_ms > 0 && auth.timeout_ms <= 30000 && Number.isInteger(auth.max_output) && auth.max_output > 0 && auth.max_output <= 65536, 'invalid CLI bounds');
          break;
        default: throw new Error('Connector: unknown authentication method');
      }
    }
    assert(Array.isArray(item.inject) && item.inject.length > 0 && item.inject.length <= 32, 'invalid injection count');
    const targets = new Set();
    for (const target of item.inject) {
      keys(target, ['origin','server','header','env','prefix','field']);
      assert((target.origin != null) !== (target.server != null), 'injection needs exactly one target');
      assert(target.origin != null ? item.origins.includes(target.origin) : identifier(target.server) && config?.[target.server], 'unknown injection target');
      assert((target.header != null) !== (target.env != null) && (target.header != null ? header(target.header) : target.server != null && environment(target.env) && !target.prefix), 'invalid injection name');
      assert(typeof (target.prefix ?? '') === 'string' && (target.prefix ?? '').length <= 64 && !/[^\x20-\x7e]/.test(target.prefix ?? '') && (target.field == null || secret(target.field)), 'invalid injection value');
      const server = config?.[target.server];
      if (server?.url) assert(target.header && !target.env && endpoint(server.url), 'MCP injection endpoint is outside its origins');
      else if (server) assert(target.env && !target.header, 'stdio injections require environment targets');
      const key = `${target.origin ?? target.server}:${target.header?.toLowerCase() ?? target.env}`;
      assert(!targets.has(key), 'duplicate injection target'); targets.add(key);
    }
    if (item.identity) { keys(item.identity, ['request','name','id']); assert(endpoint(item.identity.request) && pointer(item.identity.name) && pointer(item.identity.id), 'invalid identity probe'); }
    if (item.health) { keys(item.health, ['request','ok']); assert(endpoint(item.health.request) && Array.isArray(item.health.ok) && item.health.ok.length > 0 && item.health.ok.length <= 16 && item.health.ok.every(code => Number.isInteger(code) && code >= 100 && code <= 599), 'invalid health probe'); }
  }
}

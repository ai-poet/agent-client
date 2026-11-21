import {call, failure, response} from 'sailry/http';

const origin = 'https://api.figma.com';

function segment(value, name) {
  if (typeof value !== 'string' || !/^[A-Za-z0-9_-]+$/.test(value))
    throw new Error(`Invalid ${name}`);
  return value;
}
function parameters(args) {
  if (!args || typeof args !== 'object' || Array.isArray(args)
      || Object.keys(args).some(key => !['action','file_key','ids','version','depth'].includes(key)))
    throw new Error('Invalid Figma arguments');
  if (!['file','nodes'].includes(args.action)) throw new Error('Unknown Figma action');
  const values = [];
  if (args.action === 'nodes') {
    if (!Array.isArray(args.ids) || !args.ids.length
        || new Set(args.ids).size !== args.ids.length
        || args.ids.some(id => typeof id !== 'string' || !id.length
          || /[\u0000-\u0020,]/.test(id))) throw new Error('Invalid node IDs');
    values.push(['ids', args.ids.join(',')]);
  } else if (args.ids !== undefined) throw new Error('Node IDs require the nodes action');
  if (args.version !== undefined) values.push(['version', segment(args.version, 'version')]);
  if (args.depth !== undefined) {
    if (!Number.isSafeInteger(args.depth) || args.depth < 1)
      throw new Error('Invalid depth');
    values.push(['depth', String(args.depth)]);
  }
  const query = values.map(([key, value]) => `${key}=${encodeURIComponent(value)}`).join('&');
  return `/v1/files/${segment(args.file_key, 'file key')}${args.action === 'nodes' ? '/nodes' : ''}${query ? `?${query}` : ''}`;
}
export function read(input) {
  const args = input.arguments ?? {};
  if (input.step === 0) return call(`${origin}${parameters(args)}`, 'figma');
  const reply = response(input.outcome);
  if (reply.error) return {result:reply.error};
  const value = reply.value;
  if ((args.action === 'file' && (!value.document || typeof value.document !== 'object' || Array.isArray(value.document)))
      || (args.action === 'nodes' && (!value.nodes || typeof value.nodes !== 'object' || Array.isArray(value.nodes))))
    return {result:failure('Invalid Figma response', input.outcome.data.status)};
  const partial = args.depth !== undefined
    || (args.action === 'nodes' && Object.values(value.nodes).some(node => node === null));
  return {result:{data:value, untrusted:true, depth:args.depth ?? null, partial}};
}

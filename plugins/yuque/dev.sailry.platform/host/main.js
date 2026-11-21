import {text, id, query, failure, response, call} from 'sailry/http';

function index(value, minimum, name) {
  if (!Number.isSafeInteger(value) || value < minimum) throw new Error(`${name} must be a safe integer at least ${minimum}`);
  return value;
}
function repository(value) {
  const parts = text(value, 'repository').split('/');
  if (parts.length > 2 || parts.some(part => !part || part === '.' || part === '..'))
    throw new Error('Invalid repository');
  return parts.map(part => id(part)).join('/');
}
export function request(input) {
  const args = input.arguments ?? {};
  if (input.step === 0) {
    let path;
    switch (args.action) {
      case 'search':
        if (args.type !== undefined && !['doc','repo'].includes(args.type)) throw new Error('Invalid search type');
        path = `/search?${query({q:text(args.query, 'query'),type:args.type ?? 'doc',page:index(args.page ?? 1,1,'page')})}`; break;
      case 'repos': path = `/users/${id(args.user, 'user')}/repos?${query({offset:index(args.offset ?? 0,0,'offset')})}`; break;
      case 'docs': path = `/repos/${repository(args.repo)}/docs`; break;
      case 'doc': path = `/repos/${repository(args.repo)}/docs/${id(args.id)}?raw=1`; break;
      case 'toc': path = `/repos/${repository(args.repo)}/toc`; break;
      default: throw new Error('Unknown Yuque action');
    }
    return call(`https://www.yuque.com/api/v2${path}`, 'yuque');
  }
  const reply = response(input.outcome);
  if (reply.error) return {result:reply.error};
  if (reply.value.data === undefined) return {result:failure('Invalid Yuque response')};
  return {result:{data:reply.value.data,meta:reply.value.meta ?? null,untrusted:true}};
}

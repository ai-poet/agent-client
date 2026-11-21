import {call, failure, response as httpResponse} from 'sailry/http';

const origin = 'https://qyapi.weixin.qq.com';
const base = `${origin}/cli`;

function text(value, name) {
  if (typeof value !== 'string' || !/^[A-Za-z0-9_-]+$/.test(value)) throw new Error(`Invalid ${name}`);
  return value;
}
function path(value) {
  if (typeof value !== 'string' || !/^\/[A-Za-z0-9_/-]+$/.test(value) || value.includes('//'))
    throw new Error('Unsupported service path');
  return value;
}
function request(url, payload, state = null) {
  return call(url, 'wecom', state, {method:'POST',
    headers:{Accept:'application/json','Content-Type':'application/json'},
    body:JSON.stringify(payload)});
}
function response(outcome) {
  const reply = httpResponse(outcome);
  if (reply.error) return reply;
  const envelope = reply.value;
  if (envelope.error?.code && envelope.error.code !== 0)
    return {error:failure(`WeCom returned error ${envelope.error.code}`)};
  if (envelope.taskid || envelope.long_task_poll)
    return {error:failure('Asynchronous WeCom results are unsupported')};
  if (typeof envelope.result !== 'string') return {error:failure('Invalid WeCom result')};
  try { return {value:JSON.parse(envelope.result)}; }
  catch { return {error:failure('Invalid WeCom result')}; }
}
function method(schema, names) {
  if (!Array.isArray(names) || !names.length) throw new Error('Invalid method');
  let resource = schema;
  for (const name of names.slice(0, -1)) resource = resource?.resources?.[text(name, 'resource')];
  const target = resource?.methods?.[text(names.at(-1), 'method')];
  if (!target || typeof target.http_method !== 'string') throw new Error('Method is unavailable');
  // The official CLI gateway wraps method payloads in POST requests.
  const declaredBase = target.base_url ?? schema.base_url ?? base;
  if (typeof declaredBase !== 'string' || !(declaredBase === origin || declaredBase === `${origin}/`
      || declaredBase.startsWith(`${origin}/`))) throw new Error('Unsupported service origin');
  const suffix = declaredBase.slice(origin.length).replace(/\/$/, '');
  if (suffix) path(suffix);
  return `${origin}${suffix}${path(target.path)}`;
}
export function requestFlow(input) {
  const args = input.arguments ?? {};
  if (input.step === 0) {
    if (!['discover','describe','invoke'].includes(args.action)) throw new Error('Unknown WeCom action');
    const payload = args.action === 'discover' ? {} : {service:text(args.service, 'service')};
    return request(`${base}/service/discovery`, payload);
  }
  const reply = response(input.outcome);
  if (reply.error) return {result:reply.error};
  if (args.action === 'invoke' && input.step === 1) {
    if (!args.payload || typeof args.payload !== 'object' || Array.isArray(args.payload))
      throw new Error('Invalid method payload');
    return request(method(reply.value, args.method), {payload:JSON.stringify(args.payload)});
  }
  return {result:{data:reply.value,untrusted:true}};
}

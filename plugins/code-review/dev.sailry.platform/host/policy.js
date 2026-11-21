export const limits = Object.freeze({page:100,
  value:256*1024, shard:128*1024, transaction:2*1024*1024, output:128*1024});

export function fail(message, code = 'invalid_request', details) {
  const error = new Error(message);
  error.code = code;
  error.details = details;
  throw error;
}
export function bytes(text) {
  let count = 0;
  for (const character of text) {
    const point = character.codePointAt(0);
    count += point < 0x80 ? 1 : point < 0x800 ? 2 : point < 0x10000 ? 3 : 4;
  }
  return count;
}
export const size = value => bytes(JSON.stringify(value));
export const clone = value => JSON.parse(JSON.stringify(value));
export const equal = (left, right) => JSON.stringify(left) === JSON.stringify(right);
export function fields(value, allowed, required = []) {
  if (!value || typeof value !== 'object' || Array.isArray(value)
      || Object.keys(value).some(key => !allowed.includes(key))
      || required.some(key => !Object.hasOwn(value, key))) fail('Invalid review arguments');
  return value;
}
export function text(value, label, minimum = 1) {
  if (typeof value !== 'string' || value.includes('\0') || value.trim().length < minimum) fail(`Invalid ${label}`);
  return value;
}
export function integer(value, label, minimum, maximum) {
  if (!Number.isSafeInteger(value) || value < minimum || value > maximum) fail(`Invalid ${label}`);
  return value;
}
export function identifier(value) {
  if (typeof value !== 'string' || !/^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i.test(value)) {
    fail('Invalid review identifier');
  }
  return value;
}
export function path(value) {
  if (typeof value !== 'string' || !value.length || value.includes('\0')) fail('Invalid review path');
  if (value.startsWith('/') || value.includes('\\')
      || value.split('/').some(part => !part || part === '.' || part === '..')) fail('Invalid review path');
  return value;
}
export function revision(value) { return text(value, 'file revision'); }
export function result(error) {
  return {isError:true,error:{code:error.code ?? 'internal',message:error.message},
    ...(error.details ? {remaining:error.details} : {})};
}

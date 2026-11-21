export class ResponseError extends Error {
  constructor(status,body) {
    const value = typeof body === 'string' ? parse(body) : body;
    const message = typeof value?.error?.message === 'string' ? value.error.message
      : typeof value?.error === 'string' ? value.error
      : typeof value?.message === 'string' ? value.message : '';
    super(message);
    this.status = status;
  }
}

function parse(value) {
  try {return JSON.parse(value);} catch (_) {return null;}
}

export function record(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

export function response(input) {
  if (!Number.isInteger(input.status) || input.status < 200 || input.status >= 300) {
    throw new ResponseError(input.status,input.body);
  }
  const value = typeof input.body === 'string' ? JSON.parse(input.body) : input.body;
  if (!record(value)) throw new Error('Invalid provider usage response');
  if (value.error) throw new ResponseError(input.status,value);
  return value;
}

export function number(value) {
  if (value == null) return null;
  if (typeof value === 'string' && /^\d+(?:\.\d+)?$/.test(value)) value = Number(value);
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) {
    throw new Error('Invalid provider usage number');
  }
  return value;
}

export function instant(value) {
  if (value == null) return null;
  const parsed = typeof value === 'string' ? Date.parse(value) : NaN;
  if (!Number.isSafeInteger(parsed) || parsed <= 0) throw new Error('Invalid provider usage reset');
  return parsed;
}

export function milliseconds(seconds) {
  const value = number(seconds);
  if (value == null) return null;
  const result = value * 1000;
  if (!Number.isSafeInteger(result)) throw new Error('Invalid provider usage reset');
  return result;
}

// SPDX-License-Identifier: MIT
// Public error fields follow the fixed upstream references in NOTICE.
function display(value) {
  if (typeof value === 'string' && value.trim()) return value;
  if (!value || typeof value !== 'object') return null;
  for (const key of ['zh','zh-CN','en','en-US']) {
    if (typeof value[key] === 'string' && value[key].trim()) return value[key];
  }
  return null;
}

export function failure(body) {
  let value;
  try { value = typeof body === 'string' ? JSON.parse(body) : body; } catch { value = null; }
  const layers = [];
  function visit(value,depth) {
    if (!value || typeof value !== 'object' || Array.isArray(value) || depth > 4) return;
    layers.push(value);
    for (const key of ['error','data','details','Response','Error']) visit(value[key],depth+1);
  }
  visit(value,0);
  const code = layers.flatMap(value=>[value.code,value.bizcode,value.bizCode,value.Code])
    .filter(value=>typeof value === 'string' || typeof value === 'number')
    .map(String).find(value=>value && value !== '0' && value !== '200') ?? null;
  let message = null;
  for (const names of [['displayTips','displayMsg'],['message','msg','Message']]) {
    message = layers.flatMap(value=>names.map(name=>display(value[name]))).find(Boolean) ?? null;
    if (message) break;
  }
  const type = layers.map(value=>value.type).find(value=>typeof value === 'string') ?? null;
  return {code,message,type};
}

export class ResponseError extends Error {
  constructor(status,body) {
    const {code,message} = failure(body);
    super(message ?? '');
    this.name = 'ResponseError';
    this.status = status;
    this.code = code;
  }
}

import {ResponseError} from '../errors.js';
export {ResponseError} from '../errors.js';
// MIT: billing wire fields reference the fixed revisions recorded in NOTICE.
export function range({now = Date.now()} = {}) {
  const aligned = Math.floor(now / 60000) * 60000;
  const start = new Date(aligned), end = new Date(aligned + 365 * 101 * 24 * 3600 * 1000);
  const format = value => `${value.getFullYear()}-${String(value.getMonth()+1).padStart(2,'0')}-${String(value.getDate()).padStart(2,'0')} ${String(value.getHours()).padStart(2,'0')}:${String(value.getMinutes()).padStart(2,'0')}:${String(value.getSeconds()).padStart(2,'0')}`;
  return {begin:format(start),end:format(end)};
}

function number(value) {
  if (typeof value === 'string' && /^[0-9]+(?:\.[0-9]+)?$/.test(value)) value = Number(value);
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) {
    throw new Error('Invalid WorkBuddy credit amount');
  }
  return value;
}

function field(account,name) {
  return number(account[`${name}Precise`] ?? account[name]);
}

export function decode(input) {
  if (input.status < 200 || input.status >= 300) throw new ResponseError(input.status,input.body);
  const value = typeof input.body === 'string' ? JSON.parse(input.body) : input.body;
  if (!value || typeof value !== 'object' || value.code !== 0) throw new ResponseError(input.status,value);
  const rows = value.data?.Response?.Data?.Accounts;
  if (rows === null) return {remaining:0,accounts:[]};
  if (!Array.isArray(rows)) throw new Error('Invalid WorkBuddy credits response');
  // An empty personal-resource list cannot establish another account type's balance.
  if (!rows.length) return {remaining:null,accounts:[]};
  let remaining = 0;
  const accounts = rows.map(account => {
    if (!account || typeof account !== 'object') throw new Error('Invalid WorkBuddy credit package');
    const optional = name => account[`${name}Precise`] != null || account[name] != null ? field(account,name) : null;
    const cycleRemain=optional('CycleCapacityRemain'),cycleSize=optional('CycleCapacitySize');
    const cycleUsed=optional('CycleCapacityUsed');
    const cycle=(cycleSize ?? 0)>0 || (cycleRemain ?? 0)>0 || (cycleUsed ?? 0)>0;
    const amount = cycle ? field(account,'CycleCapacityRemain') : field(account,'CapacityRemain');
    const capacity = cycle ? cycleSize : optional('CapacitySize');
    remaining += amount;
    if (!Number.isFinite(remaining)) throw new Error('Invalid WorkBuddy credit total');
    return {name:typeof account.PackageName === 'string' ? account.PackageName : null,remaining:amount,capacity};
  });
  return {remaining,accounts};
}

export function amount(value) {
  return value.toFixed(2).replace(/\.00$/,'').replace(/(\.[0-9])0$/,'$1');
}

export function metric(balance,text) {
  if (!balance || balance.remaining == null) return {value:'—',details:[]};
  return {value:amount(balance.remaining),details:[
    {label:{label:text.remaining},value:amount(balance.remaining)},
    // Keep the complete total when individual packages exceed the shared detail budget.
    ...(balance.accounts.length > 31 ? [] : balance.accounts).map(account => ({label:{label:account.name || text.package},
      value:account.capacity == null ? amount(account.remaining) : `${amount(account.remaining)} / ${amount(account.capacity)}`,
      ...(account.capacity > 0 && account.remaining <= account.capacity
        ? {progress:account.remaining / account.capacity * 100} : {})})),
  ]};
}

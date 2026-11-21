import {response,record,number,instant} from './response.js';

const windows = ['five_hour','seven_day','seven_day_sonnet','seven_day_opus','seven_day_oauth_apps','seven_day_cowork'];

export function decode(input) {
  const value = response(input);
  if (!windows.some(key=>Object.hasOwn(value,key)) && !Object.hasOwn(value,'extra_usage')) {
    throw new Error('Claude usage windows are missing');
  }
  const rows = [];
  for (const key of windows) {
    const window = value[key];
    if (window == null) continue;
    if (!record(window)) throw new Error('Invalid Claude usage window');
    rows.push({key,used:number(window.utilization),reset:instant(window.resets_at)});
  }
  const extra = value.extra_usage;
  if (extra != null && !record(extra)) throw new Error('Invalid Claude extra usage');
  return {windows:rows,credits:null,extra:extra == null ? null : {
    used:number(extra.used_credits),limit:number(extra.monthly_limit),
    currency:typeof extra.currency === 'string' ? extra.currency : null,
    enabled:typeof extra.is_enabled === 'boolean' ? extra.is_enabled : null,
  }};
}

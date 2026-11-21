import {response,record,number,milliseconds} from './response.js';

export function decode(input,now = Date.now()) {
  const value = response(input);
  if (!Object.hasOwn(value,'rate_limit') && !Object.hasOwn(value,'credits')) {
    throw new Error('GPT usage windows are missing');
  }
  const limit = value.rate_limit;
  if (limit != null && !record(limit)) throw new Error('Invalid GPT rate limit');
  const windows = [];
  for (const name of ['primary_window','secondary_window']) {
    const window = limit?.[name];
    if (window == null) continue;
    if (!record(window)) throw new Error('Invalid GPT usage window');
    const duration = number(window.limit_window_seconds);
    const key = duration === 18000 ? 'five_hour' : duration === 604800 ? 'seven_day'
      : name === 'primary_window' ? 'primary' : 'secondary';
    let reset = milliseconds(window.reset_at);
    if (reset == null && window.reset_after_seconds != null) {
      reset = now + milliseconds(window.reset_after_seconds);
      if (!Number.isSafeInteger(reset)) throw new Error('Invalid GPT usage reset');
    }
    windows.push({key,used:number(window.used_percent),reset});
  }
  const credits = value.credits;
  if (credits != null && !record(credits)) throw new Error('Invalid GPT credits');
  return {windows,extra:null,credits:credits == null ? null : {
    balance:number(credits.balance),unlimited:typeof credits.unlimited === 'boolean' ? credits.unlimited : null,
    available:typeof credits.has_credits === 'boolean' ? credits.has_credits : null,
  }};
}

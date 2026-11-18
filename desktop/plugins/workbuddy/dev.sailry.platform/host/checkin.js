import {requestProviderHttp,prepareProviderHttp,forgetRequest,readSettings} from 'sailry/sdk';
import {day,status,claimed} from './protocol.js';
import {channel,read,write,complete,notify,report} from './records.js';
import {messages} from '../desktop/locales.js';
import {ResponseError} from '../errors.js';

const active = new Map();

export async function checkin(input) {
  const provider = input?.provider;
  if (!channel(provider)) return {status:'unsupported'};
  if (input.automatic === true && (await readSettings()).values.checkin_enabled !== true) {
    return {status:'disabled',skipped:true};
  }
  if (active.has(provider.id)) return active.get(provider.id);
  const pending = run(provider,input.locale);
  active.set(provider.id,pending);
  try {return await pending;} finally {active.delete(provider.id);}
}

async function run(provider,locale) {
  const key = `checkin/day/${provider.id}`,today = day();
  const text = messages(locale);
  let entry = await read(key);
  let previous = entry.present && entry.value.day === today ? entry.value.status : null;
  if (previous === 'failed' && entry.value.submitted === false) previous = null;
  if (previous && !['pending','unknown'].includes(previous)) return {status:previous,skipped:true};
  const save = async value => {
    const saved = await write(key,{day:today,...value},entry.revision);
    entry = {present:true,revision:saved.revision,value:{day:today,...value}};
  };
  let submitted = false;
  try {
    const current = status(await requestProviderHttp(provider.id,provider.revision,'checkin_status',{}));
    if (previous) {
      if (current.checked) {
        try {await save({status:'already'});} catch (_) { /* The upstream observation remains authoritative. */ }
        return {status:'already',reconciled:true};
      }
      return {status:previous,skipped:true};
    }
    if (current.checked || !current.active) {
      const result = {status:current.checked?'already':'closed'};
      await save(result);
      return result;
    }
    const id = prepareProviderHttp(provider.id,provider.revision,'checkin_claim',{});
    try {await save({status:'pending'});}
    catch (error) {forgetRequest(id);throw error;}
    submitted = true;
    const result = claimed(await complete(id));
    // This observation cannot turn a confirmed upstream result into another claim.
    try {await save(result);} catch (_) { /* Pending still prevents a same-day replay. */ }
    if (result.status === 'claimed') {
      const amount = result.credit === null ? '' : ` ${result.credit}`;
      try {await notify(text.credits,`${text.received}${amount}`,'success');} catch (_) { /* The claim remains confirmed. */ }
    }
    return result;
  } catch (error) {
    if (error.code === 'revision_conflict') return {status:'conflict',skipped:true};
    const uncertain = !!previous || (submitted && !(error instanceof ResponseError));
    const message = error instanceof ResponseError && error.message ? error.message
      : text[uncertain?'checkin_unknown':'checkin_failed'];
    const result = {status:previous ?? (uncertain?'unknown':'failed'),error:{code:error.code ?? 'invalid_request',message}};
    if (!submitted && !previous) {
      if (entry.present && entry.value.day === today && entry.value.submitted === false
        && JSON.stringify(entry.value.error) === JSON.stringify(result.error)) return result;
      try {
        await report(key,{day:today,...result,submitted:false},entry.revision,
          text.credits,message,'error');
      } catch (failure) {if (failure.code === 'revision_conflict') return {status:'conflict',skipped:true};}
      return result;
    }
    if (submitted) try {await save(result);} catch (_) { /* Never repeat a possibly submitted claim. */ }
    if (!previous) try {await notify(text.credits,message,uncertain?'warning':'error');} catch (_) { /* The result remains available in the job. */ }
    return result;
  }
}

import {context,newId,prepareRequest,prepareTransaction,readSettings} from 'sailry/sdk';
import {channel,read,complete} from './records.js';

async function query(kind) {
  const result = await complete(prepareRequest({kind:'dispatch',data:{package:context().package,action:{kind}}}));
  return result.data;
}
function operation(kind,data) {return {kind:'dispatch',data:{kind,data}};}
function handler(name,revision,topic,callback,input=null) {
  return operation('save_handler',{name,revision:String(revision),enabled:true,
    source:{package:context().package.name,topic},queue:name,
    callback:{scope:{worktree:null,session:null},completion:'command',
      command:{kind:'call_plugin',data:{handler:callback,input}},
      bindings:callback === 'checkin' ? {'/data/input':'/payload'} : {}}});
}
async function configuration() {
  const settings = await readSettings();
  const enabled = settings.values.checkin_enabled;
  if (enabled !== undefined && typeof enabled !== 'boolean') throw new Error('Invalid automatic check-in setting');
  const time = Object.hasOwn(settings.values,'checkin_time') ? settings.values.checkin_time : '09:05';
  if (typeof time !== 'string' || time.length !== 5 || !/^(?:[01][0-9]|2[0-3]):[0-5][0-9]$/.test(time)) {
    throw new Error('Invalid daily check-in time');
  }
  const [hour,minute] = time.split(':').map(Number);
  return {enabled,time,hour,minute,revision:String(context().package.settings_revision ?? 0)};
}
function outdated(old,config) {
  return old.present && BigInt(old.value.settings_revision ?? '0') > BigInt(config.revision);
}
function unchanged(old,provider,config) {
  return old.present && old.value.provider.revision === provider.revision
    && (old.value.enabled ?? true) === (config.enabled ?? old.value.enabled ?? true) && old.value.time === config.time && old.value.settings_revision === config.revision;
}
function changes(key,old,payload,id,schedule,config) {
  // Only the master switch can resume its own pauses, never a manual disable.
  const eligible = schedule?.enabled !== false || (old.present && old.value.paused === true);
  const paused = !config.enabled && eligible;
  return [
    {kind:'write',data:{key,value:{id,...payload,enabled:config.enabled,...(paused?{paused:true}:{}),time:config.time,settings_revision:config.revision},expected_revision:old.revision}},
    operation('save_schedule',{id,revision:String(schedule?.revision ?? 0),enabled:config.enabled && eligible,topic:id,payload,
      timing:{kind:'weekly',data:{timezone:'Asia/Shanghai',weekdays:[1,2,3,4,5,6,7],hour:config.hour,minute:config.minute}},next_ms:null}),
  ];
}

function callbackChanges(id,existing,enabled) {
  return enabled ? [handler(id,existing?.revision ?? 0,id,'checkin')]
    : existing ? [operation('remove_handler',{name:id,expected_revision:String(existing.revision),cancel_pending:true})] : [];
}

export async function register(input) {
  const provider = input?.provider;
  if (!channel(provider)) return {registered:false};
  const config = await configuration(),key = `checkin/schedule/${provider.id}`;
  const old = await read(key);
  if (outdated(old,config)) return {registered:true,skipped:true};
  if (unchanged(old,provider,config)) return {registered:true};
  const handlers = await query('list_handlers'),operations = [];
  const id = old.present ? old.value.id : newId();
  const schedules = await query('list_schedules');
  const schedule = schedules.find(item=>item.id === id),callback = handlers.find(item=>item.name === id);
  // Check-in is opt-in: without a setting or a saved choice it stays off.
  config.enabled ??= (old.present ? old.value.enabled : undefined) ?? false;
  const payload = {provider,locale:input.locale ?? 'en',automatic:true};
  const listener = handlers.find(item=>item.name === `${id}.settings`);
  operations.push(...changes(key,old,payload,id,schedule,config),...callbackChanges(id,callback,config.enabled),
      handler(`${id}.settings`,listener?.revision ?? 0,'settings.changed','reconcile',{key}));
  // A settings update never creates another claim job.
  if (!old.present && config.enabled) operations.push(operation('publish',{topic:id,payload}));
  if (operations.length) try {await complete(prepareTransaction(operations));}
  catch (error) {if (error.code === 'revision_conflict') return {registered:false,conflict:true};throw error;}
  return {registered:true};
}

export async function reconcile(input) {
  const key = input?.key;
  if (typeof key !== 'string' || !key.startsWith('checkin/schedule/')) return {updated:0};
  const config = await configuration(),old = await read(key);
  if (!old.present || outdated(old,config) || !channel(old.value.provider) || unchanged(old,old.value.provider,config)) return {updated:0};
  const schedules = await query('list_schedules');
  const schedule = schedules.find(item=>item.id === old.value.id);
  // Reconcile known registrations only; a removed schedule is not recreated.
  if (!schedule) return {updated:0};
  config.enabled ??= old.value.enabled ?? false;
  const payload = {provider:old.value.provider,locale:old.value.locale ?? 'en',automatic:true};
  const operations = changes(key,old,payload,old.value.id,schedule,config);
  if ((old.value.enabled ?? true) !== config.enabled) {
    const handlers = await query('list_handlers');
    operations.push(...callbackChanges(old.value.id,handlers.find(item=>item.name === old.value.id),config.enabled));
  }
  try {await complete(prepareTransaction(operations));}
  catch (error) {if (error.code === 'revision_conflict') return {updated:0,conflict:true};throw error;}
  return {updated:1};
}

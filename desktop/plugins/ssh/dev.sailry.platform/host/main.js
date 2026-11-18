import {readTurnState,stageTurnState} from 'sailry/sdk';
import {parameters} from './tools.js';
export {connections,save,saved} from './connections.js';

export function initialize({tools,connections,resources}) {
  const profiles = connections.ssh;
  const primary = connections.bound?.kind === 'ssh' && profiles.some(profile=>profile.id === connections.bound.id)
    ? connections.bound.id : null;
  stageTurnState({connections:profiles.map(profile=>profile.id),names:Object.fromEntries(profiles.map(profile=>[profile.id,profile.name]))});
  const selected = tools;
  return {tools:selected,instruction:primary
    ? `The primary SSH connection is ${primary}. Use it by default unless the user requests another saved connection; it does not restrict other SSH operations.` : '',
    parameters:Object.fromEntries(selected.map(name=>[name,parameters(name,profiles,primary)]))};
}
const invalid = () => ({isError:true,error:{code:'invalid_request',message:'Invalid SSH tool arguments'}});
function target(args,fields) {
  return args && typeof args === 'object' && !Array.isArray(args)
    && !Object.keys(args).some(key=>!fields.includes(key))
    && typeof args.connection === 'string';
}
const unavailable = () => ({isError:true,error:{code:'permission_denied',message:'SSH connection is unavailable to this session; use ssh_connections'}});
export function run(args) {
  if (!target(args,['connection','command','timeout_ms']) || typeof args.command !== 'string'
      || (args.timeout_ms != null && (!Number.isSafeInteger(args.timeout_ms) || args.timeout_ms < 1 || args.timeout_ms > 600000))) return invalid();
  if (!readTurnState().connections.includes(args.connection)) return unavailable();
  return {...args,timeout_ms:args.timeout_ms ?? 120000};
}
export function transfer(args) {
  if (!target(args,['connection','path','remote_path','direction'])
      || typeof args.path !== 'string' || typeof args.remote_path !== 'string'
      || !['upload','download'].includes(args.direction)) return invalid();
  if (!readTurnState().connections.includes(args.connection)) return unavailable();
  return {...args,timeout_ms:120000};
}

export function completed({arguments:args,output}) {
  const name=readTurnState().names[args.connection];
  return typeof name === 'string' && name.trim() && output && typeof output === 'object' && !Array.isArray(output)
    ? {...output,connection_name:name} : output;
}

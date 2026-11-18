import {readTurnState,stageTurnState} from 'sailry/sdk';

const uuid = /^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$/;
const fields = ['connection','expected_revision','name','host','port','username','authentication','credential','key_path'];
const invalid = () => ({isError:true,error:{code:'invalid_request',message:'Invalid SSH connection arguments'}});
const object = value => value && typeof value === 'object' && !Array.isArray(value);
const text = value => typeof value === 'string' && value.trim().length > 0;

export function connections(args) {
  return object(args) && Object.keys(args).length === 0 ? {} : invalid();
}

export function save(args) {
  if (!object(args) || Object.keys(args).some(key=>!fields.includes(key))) return invalid();
  const create = args.connection == null && args.expected_revision == null;
  if (!create && (!uuid.test(args.connection) || !readTurnState().connections.includes(args.connection)
      || !Number.isSafeInteger(args.expected_revision) || args.expected_revision < 1)) return invalid();
  for (const field of ['name','host','username','key_path']) {
    if (field in args && !text(args[field])) return invalid();
  }
  if (('authentication' in args && !['password','private_key','key_path','agent'].includes(args.authentication))
      || ('port' in args && (!Number.isInteger(args.port) || args.port < 1 || args.port > 65535))
      || ('key_path' in args && args.authentication != null && args.authentication !== 'key_path')
      || ('credential' in args && 'key_path' in args)) return invalid();
  if ('credential' in args && (!credential(args.credential)
      || (args.authentication != null && args.credential.kind !== args.authentication))) return invalid();
  if (create && (!text(args.name) || !text(args.host) || !Number.isInteger(args.port)
      || !text(args.username) || !args.authentication)) return invalid();
  if (create && args.authentication !== 'agent' && !('credential' in args)
      && !(args.authentication === 'key_path' && text(args.key_path))) return invalid();
  if (!create) return {...args};
  const {connection,expected_revision,...values} = args;
  return values;
}

function credential(value) {
  if (!object(value) || !['password','private_key','key_path','agent'].includes(value.kind)) return false;
  const fields = {password:['password'],private_key:['key','passphrase'],key_path:['path','passphrase'],agent:[]}[value.kind];
  if (!fields || Object.keys(value).some(key=>key !== 'kind' && !fields.includes(key))) return false;
  const required = {password:'password',private_key:'key',key_path:'path'}[value.kind];
  return (!required || typeof value[required] === 'string')
    && (!('passphrase' in value) || value.passphrase === null || typeof value.passphrase === 'string');
}

export function saved({arguments:args,output}) {
  if (output && !output.isError && !output.error && output.kind === 'ssh_profile' && uuid.test(output.data?.id)
      && (args.connection == null || args.connection === output.data.id)) {
    const state = readTurnState();
    const names={...state.names};
    if (text(output.data.name)) names[output.data.id]=output.data.name;
    stageTurnState({...state,connections:[...new Set([...state.connections,output.data.id])],names});
  }
  return output;
}

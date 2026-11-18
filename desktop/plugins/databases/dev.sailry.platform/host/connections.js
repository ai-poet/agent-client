import {readTurnState,stageTurnState} from 'sailry/sdk';

const uuid = /^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$/;
const fields = ['connection','expected_revision','name','engine','path','host','port','username','database','read_only'];
const invalid = (message='Invalid database connection arguments') => ({isError:true,error:{code:'invalid_request',message}});
const object = value => value && typeof value === 'object' && !Array.isArray(value);
const text = value => typeof value === 'string' && value.trim().length > 0;

export function connections(args) {
  return object(args) && Object.keys(args).length === 0 ? {} : invalid();
}

export function save(args) {
  if (!object(args) || Object.keys(args).some(key=>!fields.includes(key))) return invalid();
  const create = args.connection == null && args.expected_revision == null;
  if (!create && (!uuid.test(args.connection)
      || !Number.isSafeInteger(args.expected_revision) || args.expected_revision < 1)) return invalid();
  if (!create && !readTurnState().connections.includes(args.connection)) {
    return invalid('Connection is not available in this session; use database_connections or create a connection without connection and expected_revision');
  }
  for (const field of ['name','path','host','username','database']) {
    if (field in args && (typeof args[field] !== 'string' || (field !== 'database' && !text(args[field])))) return invalid();
  }
  if (('engine' in args && !['sqlite','mysql','postgres'].includes(args.engine))
      || ('port' in args && (!Number.isInteger(args.port) || args.port < 1 || args.port > 65535))
      || ('read_only' in args && typeof args.read_only !== 'boolean')) return invalid();
  if (create && (!text(args.name) || !args.engine || (args.engine === 'sqlite'
      ? !text(args.path) : !text(args.host) || !Number.isInteger(args.port)
        || !text(args.username) || typeof args.database !== 'string'))) return invalid();
  if (!create) return {...args};
  const {connection,expected_revision,...values} = args;
  return {...values,read_only:args.read_only ?? true};
}

export function saved({arguments:args,output}) {
  if (output && !output.isError && !output.error && output.kind === 'database_profile' && uuid.test(output.data?.id)
      && (args.connection == null || args.connection === output.data.id)) {
    const state = readTurnState();
    stageTurnState({...state,connections:[...new Set([...state.connections,output.data.id])]});
  }
  return output;
}

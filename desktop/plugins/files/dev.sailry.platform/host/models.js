// OCR discovery and model lifecycle stay with the execution Node.
import * as models from 'sailry/models';

const object = (value,fields) => value && typeof value === 'object' && !Array.isArray(value)
  && Object.keys(value).every(key=>fields.includes(key));
const invalid = () => ({isError:true,error:{code:'invalid_request',message:'Invalid OCR model arguments'}});

export async function ocr_models(args) {
  if (!object(args,['installed_only'])
      || ('installed_only' in args && typeof args.installed_only !== 'boolean')) return invalid();
  const entries = await models.list({target:'node',kind:'ocr'});
  return {models:args.installed_only ? entries.filter(model=>model.state.kind === 'installed') : entries};
}

function prepare(args) {
  if (!object(args,['id']) || typeof args.id !== 'string' || !/^[0-9a-f]{64}$/.test(args.id)) return invalid();
  return {model:{target:'node',kind:'ocr',id:args.id}};
}
export function ocr_install(args) { return prepare(args); }
export function ocr_remove(args) { return prepare(args); }

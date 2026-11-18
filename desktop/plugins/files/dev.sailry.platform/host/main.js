// Argument policy from Sailry 5bfeaf05 plugins/builtin/files/agent and office/agent.rs.
// Confined file access, approval, revisions and execution remain in the Node.
export {ocr_models,ocr_install,ocr_remove} from './models.js';

function object(value, keys) {
  return value !== null && typeof value === 'object' && !Array.isArray(value) &&
    Object.keys(value).every(key => keys.includes(key));
}

function invalid(message = 'Invalid file tool arguments') {
  return {isError:true,error:{code:'invalid_request',message}};
}

function revision(value) {
  return value === undefined || value === null || typeof value === 'string';
}

export function list(args) {
  if (!object(args,['path','cursor']) ||
      (args.path !== undefined && typeof args.path !== 'string')) return invalid();
  const {path = '',cursor = null} = args;
  if (cursor !== null && (!object(cursor,['revision','directory','name']) ||
      typeof cursor.revision !== 'string' || typeof cursor.directory !== 'boolean' ||
      typeof cursor.name !== 'string')) return invalid();
  return {path:path === '.' ? '' : path,after:cursor};
}

export function read(args) {
  if (!object(args,['path','offset','limit']) || typeof args.path !== 'string') return invalid();
  const {offset = 0,limit = 2000} = args;
  if (!Number.isSafeInteger(offset) || offset < 0) return invalid('offset must be a non-negative safe integer');
  if (!Number.isSafeInteger(limit) || limit < 1) return invalid('limit must be a positive safe integer');
  return {path:args.path,offset,limit};
}

export function write(args) {
  if (!object(args,['path','text','expected_revision']) || typeof args.path !== 'string' ||
      typeof args.text !== 'string' || !revision(args.expected_revision)) return invalid();
  return {path:args.path,text:args.text,expected_revision:args.expected_revision ?? null};
}

export function edit(args) {
  if (!object(args,['path','old_text','new_text','replace_all','edits','expected_revision'])) {
    return invalid('Expected edit arguments with supported fields');
  }
  if (typeof args.path !== 'string') return invalid('path must be a string');
  if (!current(args.expected_revision)) return invalid('expected_revision must be the 64-character revision from read_file');
  if (args.edits !== undefined) {
    if (args.old_text !== undefined || args.new_text !== undefined) return invalid('Use either edits or old_text and new_text');
    if (args.replace_all !== undefined && args.replace_all !== false) return invalid('Set replace_all on each edits entry');
    if (!Array.isArray(args.edits) || args.edits.length < 1) return invalid('edits must be a nonempty array');
    let error;
    args.edits.some((change,index) => {
      const field = `edits[${index}]`;
      if (!object(change,['old_text','new_text','replace_all'])) error = `${field} must contain supported edit fields`;
      else if (typeof change.old_text !== 'string' || !change.old_text) error = `${field}.old_text must be a nonempty string`;
      else if (typeof change.new_text !== 'string') error = `${field}.new_text must be a string`;
      else if (change.replace_all !== undefined && typeof change.replace_all !== 'boolean') error = `${field}.replace_all must be a boolean`;
      return error !== undefined;
    });
    if (error) return invalid(error);
    return {path:args.path,edits:args.edits,expected_revision:args.expected_revision};
  }
  if (typeof args.old_text !== 'string' || !args.old_text) return invalid('old_text must be a nonempty string');
  if (typeof args.new_text !== 'string') return invalid('new_text must be a string');
  if (args.replace_all !== undefined && typeof args.replace_all !== 'boolean') return invalid('replace_all must be a boolean');
  return {path:args.path,old_text:args.old_text,new_text:args.new_text,
    ...(args.replace_all !== undefined ? {replace_all:args.replace_all} : {}),expected_revision:args.expected_revision};
}

function current(value) {
  return typeof value === 'string' && /^[0-9a-fA-F]{64}$/.test(value);
}

// The Node parses the patch, reads its files and validates every change before approval.
export function apply(args) {
  if (!object(args,['input']) || typeof args.input !== 'string' || !args.input) {
    return invalid('input must be the whole patch text');
  }
  return {input:args.input};
}

export function remove(args) {
  if (!object(args,['path','expected_revision']) || typeof args.path !== 'string' ||
      !current(args.expected_revision)) return invalid();
  return {path:args.path,expected_revision:args.expected_revision};
}

export function move(args) {
  if (!object(args,['path','destination','expected_revision']) || typeof args.path !== 'string' ||
      typeof args.destination !== 'string' || !current(args.expected_revision)) return invalid();
  return {path:args.path,destination:args.destination,expected_revision:args.expected_revision};
}

export function mkdir(args) {
  if (!object(args,['path']) || typeof args.path !== 'string') return invalid();
  return {path:args.path};
}

export function find(args) {
  if (!object(args,['globs','path','limit','offset']) || !Array.isArray(args.globs) || args.globs.length < 1 ||
      args.globs.some(glob => typeof glob !== 'string' || !glob)) return invalid();
  const {path = '',limit = 500,offset = 0} = args;
  if (typeof path !== 'string') return invalid('path must be a string');
  if (!Number.isSafeInteger(limit) || limit < 1) return invalid('limit must be a positive safe integer');
  if (!Number.isSafeInteger(offset) || offset < 0) return invalid('offset must be a non-negative safe integer');
  return {globs:args.globs,path:path === '.' ? '' : path,limit,offset};
}

export function search(args) {
  if (!object(args,['query','regex','case_sensitive','globs','path','mode','context','type','limit','offset','multiline'])) {
    return invalid('Expected search arguments with supported fields');
  }
  const {query,regex = false,case_sensitive = false,globs = [],path = '',mode = 'files',context = 0,
    type = null,limit = 100,offset = 0,multiline = false} = args;
  if (typeof query !== 'string' || !query) return invalid('query must be a nonempty string');
  if (!Number.isSafeInteger(context) || context < 0) return invalid('context must be a non-negative safe integer');
  if (!Number.isSafeInteger(limit) || limit < 1) return invalid('limit must be a positive safe integer');
  if (!Number.isSafeInteger(offset) || offset < 0) return invalid('offset must be a non-negative safe integer');
  if (typeof regex !== 'boolean' ||
      typeof case_sensitive !== 'boolean' || !Array.isArray(globs) ||
      globs.some(glob => typeof glob !== 'string') || typeof path !== 'string' ||
      !['content','files','count'].includes(mode) ||
      (type !== null && typeof type !== 'string') || typeof multiline !== 'boolean') return invalid();
  return {query,regex,case_sensitive,globs,path:path === '.' ? '' : path,mode,context,type,limit,offset,multiline};
}

export function office(args) {
  if (!object(args,['source','offset'])) return invalid();
  const {source,offset = 0} = args;
  if (!object(source,['kind','value']) || !['path','attachment'].includes(source.kind) || typeof source.value !== 'string' || !source.value || !Number.isSafeInteger(offset) || offset < 0) return invalid();
  return {source,offset};
}


export function notebook_read(args) {
  if (!object(args,['path','offset','limit']) || typeof args.path !== 'string') return invalid();
  const {offset = 0,limit = 20} = args;
  if (!Number.isSafeInteger(offset) || offset < 0) return invalid('offset must be a non-negative safe integer');
  if (!Number.isSafeInteger(limit) || limit < 1) return invalid('limit must be a positive safe integer');
  return {path:args.path,offset,limit};
}

export function notebook_edit(args) {
  if (!object(args,['path','expected_revision','edits']) || typeof args.path !== 'string' ||
      !current(args.expected_revision) || !Array.isArray(args.edits) || args.edits.length < 1) return invalid();
  for (const edit of args.edits) {
    const keys = edit?.kind === 'insert' ? ['kind','index','cell_type','source'] :
      edit?.kind === 'replace' ? ['kind','index','source'] : ['kind','index'];
    if (!object(edit,keys) || !Number.isSafeInteger(edit.index) || edit.index < 0 ||
        !['replace','insert','delete'].includes(edit.kind) ||
        (edit.kind !== 'delete' && (typeof edit.source !== 'string' || edit.source.includes('\0'))) ||
        (edit.kind === 'insert' && !['code','markdown','raw'].includes(edit.cell_type))) return invalid();
  }
  return {path:args.path,expected_revision:args.expected_revision,edits:args.edits};
}

import {bound, fit, invalid, omitted} from './limits.js';

const MAX_ITEMS = 50;
const DEFAULT_ITEMS = 20;
const MAX_BYTES = 16 * 1024;
const MAX_PATHS = 64;
const MAX_TARGETS = 10;

/** Validates a Node graph page for the requested view. */
export function readPage(outcome, view) {
  const data = outcome?.data;
  if (outcome?.kind !== 'code_graph' || !data || data.view !== view
      || !Number.isSafeInteger(data.index_revision) || data.index_revision < 0
      || !['targets','callers','callees','impact','map','issues'].every(key => Array.isArray(data[key]))) {
    throw new Error('Invalid code graph result');
  }
  return data;
}

/** Model-facing definitions use one-based inclusive lines. */
export function definition(target) {
  return {path:target.path,name:target.name,kind:target.kind,start_line:target.start_line+1,end_line:target.end_line+1};
}

export function caller(reference) {
  const item = {path:reference.path,line:reference.line+1,kind:reference.kind};
  if (reference.scope) item.scope = reference.scope.name;
  if (reference.count > 1) item.count = reference.count;
  item.confidence = reference.confidence;
  if (reference.ambiguous) item.ambiguous = true;
  return item;
}

export function callee(reference) {
  const item = {name:reference.name,line:reference.line+1,kind:reference.kind};
  if (reference.count > 1) item.count = reference.count;
  item.confidence = reference.confidence;
  if (reference.ambiguous) {
    item.ambiguous = true;
    item.candidates = reference.candidates;
  } else if (reference.targets[0]) {
    item.definition = {path:reference.targets[0].path,line:reference.targets[0].start_line+1};
  }
  return item;
}

export function impacted(file, symbols=Infinity) {
  const item = {path:file.path,depth:file.depth,confidence:file.confidence};
  if (file.ambiguous) item.ambiguous = true;
  item.symbols = file.symbols.slice(0,symbols).map(symbol => {
    const affected = {name:symbol.name,kind:symbol.kind,start_line:symbol.start_line+1,depth:symbol.depth};
    if (symbol.confidence !== file.confidence) affected.confidence = symbol.confidence;
    if (symbol.ambiguous) affected.ambiguous = true;
    return affected;
  });
  if (file.truncated || file.symbols.length > item.symbols.length) item.truncated = true;
  return item;
}

function ranked(symbol) {
  const item = {name:symbol.name,kind:symbol.kind,start_line:symbol.start_line+1,end_line:symbol.end_line+1};
  if (typeof symbol.signature === 'string') item.signature = symbol.signature;
  if (symbol.references > 0) item.references = symbol.references;
  return item;
}

function status(data) {
  const item = {total:data.total,index_revision:data.index_revision,partial:data.partial === true,truncated:data.truncated === true};
  if (data.next_offset != null) item.next_offset = data.next_offset;
  if (data.issues.length) {
    item.skipped = data.skipped;
    item.issues = data.issues.slice(0,10);
  }
  return item;
}

function object(input, label) {
  if (!input || typeof input !== 'object' || Array.isArray(input)) throw new Error(`Invalid ${label} query`);
  return input;
}

function paths(value) {
  if (value === undefined) return [];
  if (!Array.isArray(value) || value.length > MAX_PATHS || !value.every(path => typeof path === 'string' && path)) {
    throw new Error('Invalid code graph paths');
  }
  return value;
}

function paging(input, query) {
  query.limit = bound(input.limit,DEFAULT_ITEMS,MAX_ITEMS);
  query.offset = bound(input.offset,0,10000,0);
  return query;
}

function impactQuery(input) {
  object(input,'code impact');
  if (input.path !== undefined && input.paths !== undefined) throw new Error('Use path or paths, not both');
  const selected = input.path === undefined ? paths(input.paths) : paths([input.path]);
  const query = paging(input,{view:'impact',depth:bound(input.depth,3,3)});
  if (input.name !== undefined) {
    if (typeof input.name !== 'string' || !input.name.trim()) throw new Error('Invalid code impact name');
    query.name = input.name;
  }
  if (selected.length) query.paths = selected;
  if (input.line !== undefined) {
    if (!Number.isSafeInteger(input.line) || input.line < 1 || selected.length !== 1) {
      throw new Error('A line needs exactly one path');
    }
    query.line = input.line-1;
  }
  if (query.name === undefined && query.line === undefined && !selected.length) {
    throw new Error('Name a symbol, a file line or changed paths');
  }
  return query;
}

/** One graph read, shaped into a bounded impact report. */
export function impact({arguments:input={},step,state,outcome}) {
  try {
    if (step === 0) {
      const query = impactQuery(input);
      return {call:{operation:'code.graph',arguments:query},state:{budget:bound(input.max_bytes,MAX_BYTES,MAX_BYTES,1024),
        offset:query.offset}};
    }
    if (step !== 1 || !Number.isSafeInteger(state?.budget)) return invalid('Invalid code impact continuation');
    if (outcome?.isError) return {result:outcome};
    const data = readPage(outcome,'impact');
    const result = {targets:data.targets.slice(0,MAX_TARGETS).map(definition),files:data.impact.map(file => impacted(file)),
      ...status(data)};
    if (data.targets.length > MAX_TARGETS) omitted(result);
    return {result:fit(result,'files',state.budget,state.offset)};
  } catch (error) {
    return invalid(error.message);
  }
}

/** One graph read, shaped into a bounded ranked file map. */
export function map({arguments:input={},step,state,outcome}) {
  try {
    if (step === 0) {
      object(input,'code map');
      const query = paging(input,{view:'map'});
      const focus = paths(input.paths);
      if (focus.length) query.paths = focus;
      return {call:{operation:'code.graph',arguments:query},state:{budget:bound(input.max_bytes,MAX_BYTES,MAX_BYTES,1024),
        offset:query.offset}};
    }
    if (step !== 1 || !Number.isSafeInteger(state?.budget)) return invalid('Invalid code map continuation');
    if (outcome?.isError) return {result:outcome};
    const data = readPage(outcome,'map');
    const result = {files:data.map.map(entry => ({path:entry.path,score:entry.score,symbols:entry.symbols.map(ranked)})),
      ...status(data)};
    return {result:fit(result,'files',state.budget,state.offset)};
  } catch (error) {
    return invalid(error.message);
  }
}

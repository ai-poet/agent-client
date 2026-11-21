import {bound, invalid, omitted, size} from './limits.js';
import {callee, caller, readPage, impacted} from './graph.js';

const MAX_SYMBOLS = 20;
const MAX_SNIPPETS = 5;
const MAX_BYTES = 16 * 1024;
const MAX_LINES = 80;
const GRAPH_ITEMS = 8;
const IMPACT_FILES = 5;
const IMPACT_SYMBOLS = 3;
const REASON_CHARS = 160;
/** Steps before the first source read: search, then graph. */
const READS = 2;

function options(input) {
  if (!input || typeof input !== 'object' || Array.isArray(input)) throw new Error('Invalid code context query');
  const query = {limit:bound(input.limit,MAX_SYMBOLS,MAX_SYMBOLS)};
  for (const key of ['query','name','path','language']) {
    if (input[key] !== undefined) {
      if (typeof input[key] !== 'string') throw new Error('Invalid code context filter');
      query[key] = input[key];
    }
  }
  return {query, snippets:bound(input.max_snippets,MAX_SNIPPETS,MAX_SNIPPETS),
    budget:bound(input.max_bytes,MAX_BYTES,MAX_BYTES,512)};
}

function valid(symbol) {
  const range = symbol?.range;
  return typeof symbol?.path === 'string' && typeof symbol.file_revision === 'string'
    && symbol.file_revision.length > 0 && range
    && ['start_byte','end_byte','start_line','start_column','end_line','end_column']
      .every(key => Number.isSafeInteger(range[key]) && range[key] >= 0)
    && range.end_byte >= range.start_byte && range.end_line >= range.start_line;
}

function metadata(page, limit, budget) {
  if (!page || !Array.isArray(page.symbols) || !page.symbols.every(valid)
      || !Number.isSafeInteger(page.index_revision) || page.index_revision < 0
      || !Array.isArray(page.issues)) throw new Error('Invalid code search result');
  const data = {symbols:page.symbols.slice(0,limit),snippets:[],index_revision:page.index_revision,
    partial:page.partial === true,truncated:page.truncated === true,
    visited:page.visited,skipped:page.skipped,issues:page.issues.slice(0,10)};
  if (page.symbols.length > data.symbols.length || page.issues.length > data.issues.length) omitted(data);
  while (size(data) > budget && data.issues.length) {
    data.issues.pop();
    omitted(data);
  }
  while (size(data) > budget && data.symbols.length) {
    data.symbols.pop();
    omitted(data);
  }
  if (size(data) > budget) throw new Error('Code context metadata exceeds the output budget');
  return data;
}

/** Model-facing symbols use one-based inclusive lines without byte ranges. */
function compact(symbol) {
  const range = symbol.range;
  const start = range.start_line+1;
  const item = {path:symbol.path,language:symbol.language,name:symbol.name,kind:symbol.kind,depth:symbol.depth,
    start_line:start,end_line:Math.max(start,range.end_line+(range.end_column > 0 ? 1 : 0))};
  if (typeof symbol.signature === 'string') item.signature = symbol.signature;
  return item;
}

function ranges(data, limit) {
  const files = new Map();
  for (const symbol of data.symbols) {
    const key = JSON.stringify([symbol.path,symbol.file_revision]);
    const range = symbol.range;
    const start = Math.max(0,range.start_line-2);
    const end = Math.max(range.start_line+1,range.end_line+(range.end_column > 0 ? 1 : 0))+2;
    if (!files.has(key)) files.set(key,[]);
    files.get(key).push({path:symbol.path,file_revision:symbol.file_revision,start,end});
  }
  const merged = [];
  for (const file of files.values()) {
    file.sort((left,right) => left.start-right.start || left.end-right.end);
    let previous;
    for (const range of file) {
      if (previous && range.start <= previous.end) previous.end = Math.max(previous.end,range.end);
      else {
        previous = {...range};
        merged.push(previous);
      }
    }
  }
  if (merged.length > limit) omitted(data);
  return merged.slice(0,limit).map(range => {
    const length = range.end-range.start;
    if (length > MAX_LINES) omitted(data);
    return {...range,limit:Math.min(length,MAX_LINES),truncated:length > MAX_LINES};
  });
}

/** The first symbol's graph neighborhood: one revision-free, name-matched, optional read. */
function neighbors(symbol) {
  return {call:{operation:'code.graph',arguments:{view:'context',name:symbol.name,paths:[symbol.path],
    line:symbol.range.start_line,limit:GRAPH_ITEMS}},optional:true};
}

/** Graph data is supplementary: a failed graph keeps the symbols and snippets. */
function relate(state, outcome) {
  let page;
  try {
    if (outcome?.isError) throw new Error(outcome.error?.message || 'Code graph failed');
    page = readPage(outcome,'context');
  } catch (error) {
    const reason = Array.from(String(error.message || 'Code graph failed'));
    state.data.partial = true;
    state.data.graph_unavailable = reason.length > REASON_CHARS
      ? `${reason.slice(0,REASON_CHARS-1).join('')}…` : reason.join('');
    return;
  }
  const data = state.data;
  const callers = page.callers.slice(0,GRAPH_ITEMS);
  // Calls without an indexed definition belong to dependencies or the runtime.
  const callees = page.callees.filter(reference => reference.confidence).slice(0,GRAPH_ITEMS);
  const files = page.impact.slice(0,IMPACT_FILES);
  data.callers = callers.map(caller);
  data.callees = callees.map(callee);
  data.impact = {total:page.total,files:files.map(file => impacted(file,IMPACT_SYMBOLS))};
  if (page.partial === true) data.partial = true;
  if (page.truncated === true || page.callers.length > callers.length || page.total > files.length) data.truncated = true;
  for (const key of ['callers','callees']) {
    while (size(data)+128 > state.budget && data[key].length) {
      data[key].pop();
      omitted(data);
    }
  }
  while (size(data)+128 > state.budget && data.impact.files.length) {
    data.impact.files.pop();
    omitted(data);
  }
}

function read(state) {
  const range = state.ranges[state.next];
  return {call:{operation:'files.read_lines',arguments:{path:range.path,offset:range.start,
    limit:range.limit,expected_revision:range.file_revision}},optional:true,state};
}

function append(state, lines) {
  const range = state.ranges[state.next];
  if (!lines || lines.path !== range.path || lines.revision !== range.file_revision
      || lines.offset !== range.start || !Array.isArray(lines.lines)
      || lines.lines.length > range.limit
      || !lines.lines.every((line,index) => line.number === range.start+index+1 && typeof line.text === 'string')) {
    throw new Error('Invalid revision-checked source result');
  }
  const incomplete = lines.next_offset != null && lines.next_offset < range.start+range.limit;
  if (incomplete) omitted(state.data);
  if (!lines.lines.length) {
    state.data.partial = true;
    return;
  }
  const snippet = {path:range.path,file_revision:range.file_revision,
    start_line:lines.lines[0].number,end_line:lines.lines[lines.lines.length-1].number,
    text:lines.lines.map(line => line.text).join(''),truncated:range.truncated || incomplete};
  state.data.snippets.push(snippet);
  if (size(state.data) <= state.budget) return;
  omitted(state.data);
  snippet.truncated = true;
  const characters = Array.from(snippet.text);
  let low = 0, high = characters.length;
  while (low < high) {
    const middle = Math.ceil((low+high)/2);
    snippet.text = characters.slice(0,middle).join('');
    if (size(state.data) <= state.budget) low = middle;
    else high = middle-1;
  }
  snippet.text = characters.slice(0,low).join('');
  if (!snippet.text || size(state.data) > state.budget) state.data.snippets.pop();
  else snippet.end_line = snippet.start_line+snippet.text.split('\n').length-1-(snippet.text.endsWith('\n') ? 1 : 0);
}

/** One symbol search, the first symbol's graph, then at most five version-checked source reads. */
export function context({arguments:input={},step,state,outcome}) {
  try {
    if (!Number.isInteger(step) || step < 0 || step > MAX_SNIPPETS+READS) return invalid('Invalid code context continuation');
    if (step === 0) {
      const limits = options(input);
      return {call:{operation:'code.search',arguments:limits.query},state:{...limits,next:0}};
    }
    if (outcome?.isError && step === 1) return {result:outcome};
    if (step === 1) {
      const data = metadata(outcome?.data,state.query.limit,state.budget);
      state = {...state,data,ranges:ranges(data,state.snippets)};
      const first = data.symbols[0];
      data.symbols = data.symbols.map(compact);
      if (first) return {...neighbors(first),state};
    } else if (step === READS) {
      if (!state?.ranges || state.next !== 0) return invalid('Invalid code context continuation');
      relate(state,outcome);
    } else {
      if (!state?.ranges || state.next !== step-READS-1) return invalid('Invalid code context continuation');
      // A file edited after indexing fails its revision-checked read; keep the
      // symbols and other snippets and mark the result partial.
      if (outcome?.isError) state.data.partial = true;
      else append(state,outcome?.data);
      state.next++;
    }
    if (state.next >= state.ranges.length || size(state.data)+128 >= state.budget) {
      if (state.next < state.ranges.length) omitted(state.data);
      return {result:state.data};
    }
    return read(state);
  } catch (error) {
    return invalid(error.message);
  }
}

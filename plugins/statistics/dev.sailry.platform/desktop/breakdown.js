// Breakdown rows: models and their providers come from model groups; owners from project groups.
import {integer, ratio, amount} from './format.js';
import {parts} from './charts.js';

/** Rows shown before the list is expanded. */
export const VISIBLE = 6;
/** Least width of the name column; its text has that less the cell's padding. */
export const NAME_WIDTH = 280;
const NAME_ROOM = NAME_WIDTH - 20;

// Estimated width of 14 px interface text: wide characters count as full-width.
const width = text => Array.from(text).reduce((sum, char) => sum + (char.codePointAt(0) > 0x2e7f ? 15 : 8), 0);

/** Whether every row's name and note fit the name column untruncated: notes show for all rows or none. */
export const annotated = rows => rows.every(row => !row.note || width(row.name) + 6 + width(row.note) <= NAME_ROOM);

function add(row, metrics) {
  row.responses += integer(metrics.responses);
  if (metrics.tokens) {
    if (!row.tokens) row.tokens = {input:0n, output:0n, cached_input:0n, reasoning:0n};
    for (const key of Object.keys(row.tokens)) row.tokens[key] += integer(metrics.tokens[key]);
  }
  if (metrics.cost) {
    if (!row.cost) row.cost = {usd_micros:0n, responses:0n};
    row.cost.usd_micros += integer(metrics.cost.usd_micros);
    row.cost.responses += integer(metrics.cost.responses);
  }
}

function identity(group, mode, names, text) {
  const {kind, data} = group.key;
  const prefix=group.node ? `${group.node}:` : '';
  const provider = id => group.name ?? names.providers.get(`${prefix}${id}`) ?? names.providers.get(id) ?? id.slice(0, 8);
  if (kind === 'model') {
    const host=group.node ? names.hosts?.get(group.node) : null;
    if (mode === 'model') return {id:`${prefix}${data.provider}:${data.model}`, name:data.model, note:[provider(data.provider),host].filter(Boolean).join(' · '), provider:data.provider};
    if (mode === 'provider') return {id:`${prefix}${data.provider}`, name:provider(data.provider), note:host, provider:data.provider};
    return null;
  }
  if (mode !== 'project') return null;
  // Conversations without a project, connection or assistant share one unassigned row.
  if (kind === 'session') return {id:'session', name:text.usage_unassigned, note:null};
  if (kind === 'project') return {id:`${prefix}project:${data}`, name:group.name ?? names.projects.get(`${prefix}${data}`) ?? names.projects.get(data) ?? data.slice(0, 8), note:group.node ? names.hosts?.get(group.node) : null};
  if (kind === 'connection' || kind === 'assistant')
    return {id:`${prefix}${kind}:${JSON.stringify(data)}`, name:group.name ?? text.usage_unassigned, note:group.node ? names.hosts?.get(group.node) : null};
  return null;
}

/** Breakdown rows sorted by token totals, with shares of the listed total. */
export function breakdown(groups, mode, names, text) {
  const rows = new Map();
  for (const group of groups) {
    const key = identity(group, mode, names, text);
    if (!key) continue;
    if (!rows.has(key.id)) rows.set(key.id, {...key, responses:0n, tokens:null, cost:null});
    add(rows.get(key.id), group.metrics);
  }
  const list = [...rows.values()].map(row => {
    const values = parts(row.tokens);
    return {...row,
      total:values ? values.reduce((sum, value) => sum + value, 0n) : null,
      hit:values ? ratio(values[0], row.tokens.input) : null,
      cost:row.cost ? amount(row.cost.usd_micros) + (row.cost.responses < row.responses ? '+' : '') : null};
  });
  const sum = list.reduce((sum, row) => sum + (row.total ?? 0n), 0n);
  for (const row of list) row.share = ratio(row.total, sum);
  return list.sort((left, right) => {
    if (left.total !== right.total) {
      if (left.total === null) return 1;
      if (right.total === null) return -1;
      return left.total > right.total ? -1 : 1;
    }
    for (const [a, b] of [[left.name, right.name], [left.id, right.id]]) if (a !== b) return a < b ? -1 : 1;
    return 0;
  });
}

// Counters cross the SDK as decimal strings so exact values survive the script boundary.
export const integer = value => value == null ? null : BigInt(value);
export const total = usage => usage == null ? null : integer(usage.input) + integer(usage.output);
export function compact(value) {
  if (value == null) return null;
  value = integer(value);
  for (const [scale,suffix] of [[1000000000000n,'T'],[1000000000n,'B'],[1000000n,'M'],[1000n,'k']]) {
    if (value >= scale) return (Number(value) / Number(scale)).toFixed(1) + suffix;
  }
  return String(value);
}
/** Unscaled token counts retain their unit while compact counts keep their scale. */
export function tokenCount(value,template) {
  if(value==null)return null;
  const count=compact(value);
  return integer(value)<1000n ? template.replace('%{count}',count) : count;
}
/** Exact integer with thousands separators. */
export function grouped(value) {
  return value == null ? null : String(integer(value)).replace(/\B(?=(\d{3})+(?!\d))/g, ',');
}
/** Percentage of `part` in `whole`, or null without a positive whole. */
export function ratio(part,whole) {
  return part == null || whole == null || integer(whole) <= 0n ? null : Number(integer(part)) * 100 / Number(integer(whole));
}
export function amount(micros) {
  if (micros == null) return null;
  micros = integer(micros);
  return `${micros / 1000000n}.${String(micros % 1000000n / 10000n).padStart(2,'0')}`;
}
/** Per-response cost keeps sub-cent precision. */
export function price(micros) {
  if (micros == null) return null;
  micros = integer(micros);
  return `${micros / 1000000n}.${String(micros % 1000000n).padStart(6,'0').slice(0,4)}`;
}
export function cache(usage) {
  if (!usage || integer(usage.input) === 0n) return null;
  return (100 * Number(usage.cached_input) / Number(usage.input)).toFixed(0) + '%';
}
export function speed(generation) {
  return generation && integer(generation.elapsed_us) > 0n
    ? (Number(generation.output_tokens) * 1000000 / Number(generation.elapsed_us)).toFixed(1) : null;
}
export function duration(value,text) {
  return value == null ? text.composer_metric_unknown : Number(value) < 1000000
    ? text.usage_duration_ms.replace('%{value}',(Number(value)/1000).toFixed(1))
    : text.usage_duration_value.replace('%{value}',(Number(value)/1000000).toFixed(2));
}
export function cost(statistics) {
  if (!statistics?.cost) return null;
  return amount(statistics.cost.usd_micros) + (integer(statistics.cost.responses) < integer(statistics.responses) ? '+' : '');
}
export function context(tokens,limit) {
  return tokens != null && limit != null && Number(limit) > 0 ? 100 * Number(tokens) / Number(limit) : null;
}
export function severity(percent) {
  return percent == null || percent < 80 ? 'muted' : percent < 100 ? 'warning' : 'danger';
}

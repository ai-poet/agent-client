import {compact,price,integer,duration,ratio} from './format.js';

/** Cache hit rates below this percentage are highlighted. */
export const LOW_CACHE = 50;
/** Time to first token later than this many microseconds is highlighted. */
export const SLOW_FIRST_TOKEN = 10000000n;
/** Responses taking at least this many microseconds are highlighted. */
export const SLOW_RESPONSE = 15000000n;

const pad=(value,width=2)=>String(value).padStart(width,'0');
const sameDay=(left,right)=>left.getFullYear()===right.getFullYear()&&left.getMonth()===right.getMonth()&&left.getDate()===right.getDate();
/** Local time for a request row: time only today, month and day otherwise. */
export function when(value,now = Date.now()) {
  const date=new Date(Number(value));
  if(!Number.isFinite(date.getTime()))return null;
  const time=`${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
  return sameDay(date,new Date(now)) ? time : `${pad(date.getMonth()+1)}/${pad(date.getDate())} ${time}`;
}
function exactTime(value) {
  const date=new Date(Number(value));
  if(!Number.isFinite(date.getTime()))return null;
  const offset=-date.getTimezoneOffset(),zone=`${offset<0?'-':'+'}${pad(Math.floor(Math.abs(offset)/60))}:${pad(Math.abs(offset)%60)}`;
  return `${date.getFullYear()}/${pad(date.getMonth()+1)}/${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}.${pad(date.getMilliseconds(),3)} ${zone}`;
}
/** Whole-percent cache hit rate for one response, or null without input. */
export function hits(tokens) {
  if(tokens?.input==null || tokens.cached_input==null)return null;
  const input=integer(tokens.input),cached=integer(tokens.cached_input);
  const value=ratio(cached<input?cached:input,input);
  return value==null ? null : Math.floor(value);
}
export const lateStart=value=>value!=null && integer(value)>SLOW_FIRST_TOKEN;
export const longResponse=value=>value!=null && integer(value)>=SLOW_RESPONSE;

/** Table rows: cells are plain text; copied rows and tooltips keep the scope and the provider. */
export function formatRows(items,text,now = Date.now()) {
  const unknown=text.composer_metric_unknown;
  const exact=value=>value==null?unknown:String(value);
  const missing={text:unknown,text_tone:'muted'};
  const rows=[],cells=[],details=[],row_ids=[];
  for(const request of items) {
    const tokens=request.tokens ?? {},rate=hits(tokens),low=rate!=null&&rate<LOW_CACHE;
    const title=request.session_title||text.usage_untitled,scope=request.scope_name||text.usage_unassigned;
    const time=when(request.position.timestamp_ms,now)??unknown,full=exactTime(request.position.timestamp_ms)??unknown;
    const first=request.first_token_us,elapsed=request.elapsed_us;
    const micros=integer(request.usd_micros),cost=micros==null?unknown:`${micros/1000000n}.${String(micros%1000000n).padStart(6,'0')}`;
    const timing=[first,elapsed].map(value=>value==null?unknown:text.usage_duration_us.replace('%{value}',value));
    rows.push([full,`${title} · ${scope}`,`${request.model} · ${request.provider_name}`,
      exact(tokens.input),rate==null?unknown:`${rate}%`,exact(tokens.output),timing[0],timing[1],cost]);
    details.push([full,`${title}\n${scope}`,
      `${text.usage_column_model}: ${request.model}\n${text.usage_column_provider}: ${request.provider_name}`,
      `${text.usage_column_input}: ${exact(tokens.input)}`,`${text.usage_column_cached}: ${exact(tokens.cached_input)}`,
      `${text.usage_column_output}: ${exact(tokens.output)}\n${text.usage_column_reasoning}: ${exact(tokens.reasoning)}`,
      `${text.usage_column_first_token}: ${timing[0]}`,`${text.usage_column_duration}: ${timing[1]}`,cost]);
    cells.push([
      {primary:{text:time,text_tone:'muted'}},
      {primary:{text:title,...request.session_title?{}:{text_tone:'muted'}}},
      {primary:{text:request.model}},
      {primary:{text:compact(tokens.input)??unknown}},
      {primary:rate==null?missing:{text:`${rate}%`,...low?{text_tone:'warning'}:{}}},
      {primary:{text:compact(tokens.output)??unknown}},
      {primary:first==null?missing:{text:duration(first,text),...lateStart(first)?{text_tone:'warning'}:{}}},
      {primary:elapsed==null?missing:{text:duration(elapsed,text),...longResponse(elapsed)?{text_tone:'warning'}:{}}},
      {primary:micros==null?missing:{text:`$${price(micros)}`}},
    ]);
    row_ids.push(`${request.position.node}:${request.position.timestamp_ms}:${request.position.sequence}`);
  }
  return {rows,cells,details,row_ids};
}

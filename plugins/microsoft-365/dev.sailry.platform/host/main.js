import {text, id, query, limit, response, call} from 'sailry/http';

function next(value, path) {
  if (value === undefined || value === null || value === '') return null;
  const prefix = `https://graph.microsoft.com/v1.0${path}?`;
  if (typeof value !== 'string' || !value.startsWith(prefix)
      || /[\u0000-\u0020#]/.test(value)) throw new Error('Invalid Graph page cursor');
  return value;
}
export function request(input) {
  const args = input.arguments ?? {};
  if (input.step === 0) {
    let path;
    // Messages and calendarView support 1000; other lists use a local page budget.
    // https://learn.microsoft.com/en-us/graph/api/user-list-messages
    // https://learn.microsoft.com/en-us/graph/api/calendar-list-calendarview
    const pagination = limit(args.limit,args.action === 'files' ? 200 : 1000);
    const values = {'$top':pagination.page_size};
    switch (args.action) {
      case 'messages':
        path = '/me/messages'; values['$select'] = 'id,subject,from,receivedDateTime,bodyPreview,webLink';
        if (args.query) values['$search'] = JSON.stringify(text(args.query, 'query')); break;
      case 'message':
        return call(`https://graph.microsoft.com/v1.0/me/messages/${id(args.id)}`, 'microsoft');
      case 'events':
        if (args.time_min || args.time_max) {
          path = '/me/calendarView'; values.startDateTime = text(args.time_min, 'start time');
          values.endDateTime = text(args.time_max, 'end time');
        } else path = '/me/events';
        break;
      case 'event': return call(`https://graph.microsoft.com/v1.0/me/events/${id(args.id)}`, 'microsoft');
      case 'files':
        path = args.query ? `/me/drive/root/search(q='${id(text(args.query, 'query').replace(/'/g, "''"), 'query')}')` : '/me/drive/root/children';
        break;
      case 'file': return call(`https://graph.microsoft.com/v1.0/me/drive/items/${id(args.id)}`, 'microsoft');
      default: throw new Error('Unknown Microsoft 365 action');
    }
    const cursor = next(args.cursor,path);
    const url = cursor ?? `https://graph.microsoft.com/v1.0${path}?${query(values)}`;
    // Continuations are opaque server URLs; do not rewrite their page size.
    return call(url, 'microsoft', {path,...(cursor ? {} : {pagination})});
  }
  const reply = response(input.outcome);
  if (reply.error) return {result:reply.error};
  const cursor = next(reply.value['@odata.nextLink'], input.state?.path);
  return {result:{data:reply.value,next:cursor,partial:cursor !== null,untrusted:true,
    ...(input.state?.pagination ? {pagination:input.state.pagination} : {})}};
}

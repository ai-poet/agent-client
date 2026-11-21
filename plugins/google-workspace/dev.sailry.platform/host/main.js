import {id, query, limit, failure, response, call} from 'sailry/http';

export function request(input) {
  const args = input.arguments ?? {};
  if (input.step === 0) {
    // Official list APIs: Gmail 500, Calendar 2500, Drive 1000 results per page.
    // https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.messages/list
    // https://developers.google.com/workspace/calendar/api/v3/reference/events/list
    // https://developers.google.com/workspace/drive/api/reference/rest/v3/files/list
    const maximum = args.action === 'events' ? 2500 : args.action === 'files' ? 1000 : 500;
    const pagination = limit(args.limit,maximum);
    const values = {pageToken:args.cursor,maxResults:pagination.page_size};
    let path, origin = 'https://www.googleapis.com', connector = 'google';
    switch (args.action) {
      case 'messages':
        origin = 'https://gmail.googleapis.com'; connector = 'google';
        path = '/gmail/v1/users/me/messages'; values.q = args.query; break;
      case 'message':
        return call(`https://gmail.googleapis.com/gmail/v1/users/me/messages/${id(args.id)}?format=full`, 'google');
      case 'events':
        path = `/calendar/v3/calendars/${id(args.calendar ?? 'primary')}/events`;
        Object.assign(values,{q:args.query,timeMin:args.time_min,timeMax:args.time_max,singleEvents:true}); break;
      case 'event':
        return call(`https://www.googleapis.com/calendar/v3/calendars/${id(args.calendar ?? 'primary')}/events/${id(args.id)}`, connector);
      case 'files':
        path = '/drive/v3/files';
        delete values.maxResults;
        Object.assign(values,{q:args.query,pageSize:pagination.page_size,fields:'nextPageToken,incompleteSearch,files(id,name,mimeType,webViewLink,modifiedTime,description)'}); break;
      case 'file':
        return call(`https://www.googleapis.com/drive/v3/files/${id(args.id)}?fields=id%2Cname%2CmimeType%2CwebViewLink%2CmodifiedTime%2Cdescription`, connector);
      case 'export_doc':
        return call(`https://www.googleapis.com/drive/v3/files/${id(args.id)}/export?mimeType=text%2Fplain`, connector, {plain:true});
      default: throw new Error('Unknown Google Workspace action');
    }
    return call(`${origin}${path}?${query(values)}`, connector, {pagination});
  }
  if (input.state?.plain && !input.outcome?.isError) {
    const value = input.outcome?.data;
    if (!value || typeof value.body !== 'string' || value.status < 200 || value.status >= 300)
      return {result:failure('Document export failed', value?.status ?? null)};
    return {result:{text:value.body,untrusted:true}};
  }
  const reply = response(input.outcome);
  return {result:reply.error ?? {data:reply.value,next:reply.value.nextPageToken ?? null,
    partial:reply.value.incompleteSearch === true,untrusted:true,
    ...(input.state?.pagination ? {pagination:input.state.pagination} : {})}};
}

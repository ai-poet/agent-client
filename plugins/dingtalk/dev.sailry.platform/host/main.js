import {text, id, query, limit, response, call} from 'sailry/http';

import {readSettings} from 'sailry/sdk';

export async function request(input) {
  const args = input.arguments ?? {};
  if (input.step === 0) {
    const settings = await readSettings();
    const operator = text(args.operator ?? settings.values.operator, 'operator');
    const pagination = limit(args.limit);
    const values = {operatorId:operator,nextToken:args.cursor,maxResults:pagination.page_size};
    let path;
    switch (args.action) {
      case 'workspaces': path = '/v2.0/wiki/mineWorkspaces'; delete values.nextToken; delete values.maxResults; break;
      case 'nodes': path = '/v2.0/wiki/nodes'; values.parentNodeId = text(args.parent, 'parent node'); break;
      case 'node':
        path = `/v2.0/wiki/nodes/${id(args.id)}`; delete values.nextToken; delete values.maxResults; break;
      case 'files':
        path = `/v1.0/drive/spaces/${id(args.space, 'space')}/files`;
        delete values.operatorId; values.unionId = operator; values.parentId = text(args.parent, 'parent file'); break;
      default: throw new Error('Unknown DingTalk action');
    }
    return call(`https://api.dingtalk.com${path}?${query(values)}`, 'dingtalk',
      ['nodes','files'].includes(args.action) ? {pagination} : null);
  }
  const reply = response(input.outcome);
  return {result:reply.error ?? {data:reply.value,next:reply.value.nextToken ?? null,
    partial:reply.value.hasMore === true,untrusted:true,
    ...(input.state?.pagination ? {pagination:input.state.pagination} : {})}};
}

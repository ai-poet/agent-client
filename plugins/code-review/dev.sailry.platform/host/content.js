import {fail,fields,identifier,integer,limits,path,size} from './policy.js';
import {head,identity,key,manifest,record} from './records.js';

export function available(file) {
  if (!file.content || !['diff','old','new'].every(part => {
    const range=file.content[part];
    return range && Number.isSafeInteger(range.start) && range.start >= 0
      && Number.isSafeInteger(range.count) && range.count > 0;
  })) fail('Review content is unavailable; create a new review scope','not_found');
}

export async function* pages(value,file,part,captured=null) {
  available(file);
  const range=file.content[part];
  let offset=0;
  for (let index=0;index<range.count;index++) {
    const name=key(value.id,`data/${range.start+index}`),current=captured===null?await record(name):captured.get(name);
    const page=current.value;
    if (!current.present || current.revision !== '1' || page?.v !== 1 || page.kind !== 'content'
        || page.capture !== value.id || page.path !== file.path || page.part !== part
        || page.revision !== file.revision || page.offset !== offset) fail('Review content is incomplete','internal');
    const rows=part === 'diff' ? page.lines : typeof page.text === 'string'
      ? page.text.match(/[^\n]*\n|[^\n]+$/g) ?? [] : null;
    if (!Array.isArray(rows) || (part === 'diff' && rows.some(line =>
      !line || !['added','deleted','context'].includes(line.kind) || typeof line.text !== 'string'
      || line.text.includes('\n') || (line.kind !== 'deleted' && (!Number.isSafeInteger(line.new_line) || line.new_line < 1))
      || (line.kind !== 'added' && (!Number.isSafeInteger(line.old_line) || line.old_line < 1))))) {
      fail('Review content is invalid','internal');
    }
    const next=offset+rows.length;
    if (index+1 === range.count ? page.next !== null : page.next !== next || next <= offset) {
      fail('Review content cursor is incomplete','internal');
    }
    yield {...page,rows};
    offset=next;
  }
}

export async function verify(value,file) {
  available(file);
  const parts=['diff','old','new'],names=parts.flatMap(part=>Array.from({length:file.content[part].count},(_,index)=>key(value.id,`data/${file.content[part].start+index}`)));
  const captured=new Map(await Promise.all(names.map(async name=>[name,await record(name)])));
  for (const part of parts) for await (const _page of pages(value,file,part,captured)) { /* Validate every captured page. */ }
}

export async function read(input) {
  const args=fields(input,['review_id','path','part','offset','limit'],['review_id','path']);
  identifier(args.review_id); path(args.path);
  const part=args.part ?? 'diff';
  if (!['diff','old','new'].includes(part)) fail('Invalid review content part');
  const offset=integer(args.offset ?? 0,'review content offset',0,Number.MAX_SAFE_INTEGER);
  const limit=integer(args.limit ?? 500,'review content limit',1,Number.MAX_SAFE_INTEGER);
  const current=await head(args.review_id,await identity());
  const scope=await manifest(current.value), file=scope.files.find(file => file.path === args.path);
  if (!file) fail('Review file is outside the scope');
  const result={review_id:args.review_id,snapshot:current.value.snapshot,path:file.path,
    old_path:file.old_path,metadata_revision:file.metadata_revision,revision:file.revision,
    part,offset,next:null,...(part === 'diff' ? {lines:[]} : {text:''})};
  let position=0, count=0;
  for await (const page of pages(current.value,file,part)) {
    for (const row of page.rows) {
      if (position++ < offset) continue;
      if (count === limit) { result.next=offset+count; return result; }
      const previous=part === 'diff' ? null : result.text;
      if (part === 'diff') result.lines.push(row); else result.text+=row;
      if (size(result) > limits.output) {
        if (part === 'diff') result.lines.pop(); else result.text=previous;
        if (!count) fail('Review source line exceeds the output budget');
        result.next=offset+count;
        return result;
      }
      count++;
    }
  }
  if (offset > position) fail('Review content offset exceeds the source');
  return result;
}

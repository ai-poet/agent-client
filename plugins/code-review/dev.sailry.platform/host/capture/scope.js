import {append,flush} from '../capture.js';
import {changePage,contextPage,inventoryPage} from '../git.js';
import {excluded} from '../filter.js';
import {groups as partition} from '../groups.js';
import {rule} from '../matching.js';
import {clone,fail,limits,size} from '../policy.js';
import {key,record,shards} from '../records.js';

// Leave time for a bounded native read and the durable checkpoint before the host deadline.
const reads=16, duration=25000;

export function initialize(owner,target) {
  owner.value.progress={phase:'inventory',target:clone(target),snapshot:null,cursor:null,
    inventory:[],results:[],buffer:[],final:[],page:0,row:0,current:null,part:'diff',offset:0,
    counts:{files:0,groups:0,skipped:0}};
}

async function rows(owner,pages) {
  const result=[];
  for (const page of pages) {
    const stored=await record(key(owner.id,`scope/${page}`));
    if (!stored.present || stored.revision !== '1' || stored.value?.capture !== owner.id
        || stored.value.kind !== 'scope' || !Array.isArray(stored.value.rows)) {
      fail('Review capture is incomplete','internal');
    }
    result.push(...stored.value.rows);
  }
  return result;
}

async function store(owner,values) {
  const pages=[];
  for (const operation of shards(owner.id,'scope','scope',values).operations) {
    pages.push(await append(owner,'scope',operation.data.value));
  }
  return pages;
}

async function complete(owner,kind,value) {
  const progress=owner.value.progress;
  const row={kind,value};
  if (progress.buffer.length && size([...progress.buffer,row]) > limits.shard/2) {
    progress.results.push(...await store(owner,progress.buffer));
    progress.buffer=[];
  }
  progress.buffer.push(clone(row));
  progress.counts[kind === 'file' ? 'files' : 'skipped']++;
  progress.current=null;
  progress.row++;
}

function head(owner) {
  const progress=owner.value.progress, actor=owner.actor;
  return {v:1,kind:'head',id:owner.id,creator:actor.session,worktree:actor.worktree,
    package:clone(owner.value.package),snapshot:progress.snapshot,status:'open',pages:progress.final,
    counts:progress.counts,findings:[],reviewed_groups:[],receipts:[]};
}

export async function advance(owner) {
  const progress=owner.value.progress;
  if (!progress || !['inventory','files','groups','verify','publish'].includes(progress.phase)
      || ![progress.inventory,progress.results,progress.buffer,progress.final].every(Array.isArray)) {
    fail('Review capture is incompatible','invalid_request');
  }
  const expires=Date.now()+duration;
  let inventory=null;
  for (let step=0;step<reads && Date.now()<expires;step++) {
    if (progress.phase === 'publish') return head(owner);
    if (progress.phase === 'inventory') {
      const page=await inventoryPage(owner.actor.worktree,progress.target,progress.cursor);
      progress.inventory.push(...await store(owner,page.entries.map(value=>({kind:'entry',value}))));
      progress.snapshot=page.snapshot;
      progress.cursor=page.next;
      if (page.exhausted) progress.phase='files';
    } else if (progress.phase === 'files') {
      if (!progress.current) {
        inventory ??= await rows(owner,progress.inventory.slice(progress.page,progress.page+1));
        if (progress.row === inventory.length) {
          progress.page++;
          progress.row=0;
          inventory=null;
          if (progress.page >= progress.inventory.length) {
            progress.results.push(...await store(owner,progress.buffer));
            progress.buffer=[];
            progress.phase='groups';
          }
          await flush(owner,true);
          continue;
        }
        const row=inventory[progress.row];
        if (row?.kind !== 'entry') fail('Review capture inventory is incomplete','internal');
        const entry=row.value, reason=excluded(entry.path) ?? (entry.old_path ? excluded(entry.old_path) : null);
        if (reason) {
          await complete(owner,'skipped',{path:entry.path,reason});
          await flush(owner,true);
          continue;
        }
        progress.current={path:entry.path,old_path:entry.old_path ?? null,kind:entry.kind,
          metadata_revision:entry.metadata_revision,revision:null,rule:null,group:null,
          content:{diff:{start:owner.value.chunks,count:0}}};
        progress.part='diff';
        progress.offset=0;
      }
      const file=progress.current, part=progress.part;
      if (part === 'diff') {
        const page=await changePage(owner.actor.worktree,progress.snapshot,file,progress.offset);
        if (page.binary || page.skip_reason) {
          await complete(owner,'skipped',{path:file.path,reason:page.skip_reason ?? 'binary file'});
        } else {
          file.revision=page.revision;
          file.rule ??= rule(file.path,page.first_line);
          await append(owner,'data',{v:1,kind:'content',path:file.path,part,
            revision:file.revision,offset:page.offset,next:page.next,lines:page.lines});
          file.content.diff.count++;
          progress.offset=page.next;
          if (page.next === null) {
            progress.part='old';
            progress.offset=0;
            file.content.old={start:owner.value.chunks,count:0};
          }
        }
      } else {
        const page=await contextPage(owner.actor.worktree,progress.snapshot,file,part,progress.offset);
        await append(owner,'data',{v:1,kind:'content',path:file.path,part,
          revision:file.revision,offset:page.offset,next:page.next,text:page.text});
        file.content[part].count++;
        progress.offset=page.next;
        if (page.next === null) {
          if (part === 'old') {
            progress.part='new';
            progress.offset=0;
            file.content.new={start:owner.value.chunks,count:0};
          } else await complete(owner,'file',file);
        }
      }
    } else if (progress.phase === 'groups') {
      const captured=await rows(owner,progress.results);
      const files=captured.filter(row=>row.kind === 'file').map(row=>row.value);
      const skipped=captured.filter(row=>row.kind === 'skipped');
      const groups=partition(files), byPath=new Map(files.map(file=>[file.path,file]));
      for (const group of groups) for (const path of group.files) byPath.get(path).group=group.id;
      progress.counts.groups=groups.length;
      const final=shards(owner.id,'scope','scope',[...files.map(value=>({kind:'file',value})),
        ...groups.map(value=>({kind:'group',value})),...skipped]).operations;
      // Prepare the immutable scope once. Every automatic transaction flush
      // captures its completed shard identities, including an interrupted tail.
      for (let index=progress.final.length;index<final.length;index++) {
        progress.final.push(await append(owner,'scope',final[index].data.value));
      }
      progress.phase='verify';
      progress.cursor={snapshot:progress.snapshot,after_path:null};
    } else {
      const page=await inventoryPage(owner.actor.worktree,null,progress.cursor);
      progress.cursor=page.next;
      if (page.exhausted) progress.phase='publish';
    }
    // Each page and its exact continuation become durable together.
    await flush(owner,true);
  }
  return progress.phase === 'publish' ? head(owner) : null;
}

export function pending(owner) {
  const progress=owner.value.progress;
  return {review_id:owner.id,status:'capturing',snapshot:progress.snapshot,
    counts:clone(progress.counts),next:null};
}

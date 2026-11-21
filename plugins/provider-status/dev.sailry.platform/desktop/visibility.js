import {nextChange} from 'sailry/sdk';
import {context} from 'sailry';

// Visibility belongs to the captured host; hidden views retain results without polling.
export function observeVisibility(owner,cx) {
  owner.visible=false;
  owner.scheduled=null;
  owner.deferRefresh=at=>{
    if (owner.scheduled?.at<=at) return;
    const scheduled={at};owner.scheduled=scheduled;
    cx.spawn(async cx=>{
      await cx.sleep(Math.max(0,at-Date.now()));
      if (owner.scheduled!==scheduled) return;
      owner.scheduled=null;
      if (owner.visible && owner.state?.connected) owner.refresh(cx,true);
    });
  };
  owner.wake=()=>{
    if (!owner.visible || !owner.state?.connected) return;
    owner.refresh(cx);owner.ensure?.(cx);
  };
  const scope=JSON.parse(context()).worktree ? 'settled' : 'package';
  cx.spawn(async ()=>{
    let cursor='';
    try {while (true) {
      const change=await nextChange(cursor,scope);cursor=change.cursor;
      owner.visible=change.visible!==false && change.connected;
      if (!owner.visible) owner.scheduled=null;
      owner.wake();
    }} catch (_) { /* View release ends visibility observation. */ }
  });
}

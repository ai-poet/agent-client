import {nextChange} from 'sailry/sdk';

// Visibility belongs to the captured host; hidden views retain results without polling.
export function observeVisibility(owner,cx) {
  owner.visible=false;
  let polling=false;
  owner.wake=()=>{
    if (!owner.visible || !owner.state?.connected) return;
    owner.refresh(cx);owner.ensure?.(cx);
    if (polling) return;
    polling=true;
    cx.spawn(async cx=>{
      try {
        while (owner.visible && owner.state?.connected) {
          await cx.sleep(60000);
          if (!owner.visible || !owner.state?.connected) break;
          owner.refresh(cx);
        }
      } catch (_) { /* View release cancels observation. */ }
      finally {polling=false;}
    });
  };
  cx.spawn(async ()=>{
    let cursor='';
    try {while (true) {
      const change=await nextChange(cursor,'package');cursor=change.cursor;
      owner.visible=change.visible!==false && change.connected;
      owner.wake();
    }} catch (_) { /* View release ends visibility observation. */ }
  });
}

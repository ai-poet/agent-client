export function amount(value) {
  return (value ?? 0).toFixed(2).replace(/\.00$/,'').replace(/(\.\d)0$/,'$1');
}

function time(value) {
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) throw new Error('Invalid provider usage reset');
  const pad = value => String(value).padStart(2,'0');
  return `${String(date.getFullYear()).padStart(4,'0')}-${pad(date.getMonth()+1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function remaining(used) {
  return used == null ? null : Math.max(0,100-used);
}

export function metric(snapshot,text) {
  if (!snapshot || (!snapshot.windows.length && !snapshot.credits && !snapshot.extra)) return {value:null,details:[]};
  const measured = snapshot.windows.map(window=>remaining(window.used)).filter(value=>value != null);
  const details = [];
  for (const window of snapshot.windows) {
    const label = text[window.key];
    const value = remaining(window.used);
    if (value != null) details.push({label:{label},value:`${amount(value)}%`,progress:value});
    if (window.reset != null) details.push({label:{label:text.resets},value:time(window.reset)});
  }
  const credits = snapshot.credits?.unlimited === true ? text.unlimited
    : snapshot.credits?.balance != null ? amount(snapshot.credits.balance) : null;
  if (credits != null) details.push({label:{label:text.credits},value:credits});
  if (snapshot.extra) {
    // Claude's extra_usage amounts use minor currency units, unlike dollar windows.
    details.push({label:{label:text.extra},value:`${amount(snapshot.extra.used == null ? null : snapshot.extra.used / 100)} / ${amount(snapshot.extra.limit == null ? null : snapshot.extra.limit / 100)}${snapshot.extra.currency ? ` ${snapshot.extra.currency}` : ''}`});
  }
  return {value:measured.length ? `${amount(Math.min(...measured))}%` : credits,details};
}

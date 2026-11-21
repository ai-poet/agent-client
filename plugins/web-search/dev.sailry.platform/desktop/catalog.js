// Public settings and model choices carry no authorization material.
export const drivers = ['exa','tinyfish','tavily','brave','searxng','bocha','volcengine','deepseek','model'];
const native = ['responses','anthropic','gemini'];
// AI Logo brand marks; TinyFish has no mark in the pinned collection and
// model search is a capability, so those rows use ordinary service icons.
const logos = {exa:'exa',tavily:'tavily',brave:'brave',searxng:'searxng',bocha:'bocha',volcengine:'volcengine',deepseek:'deepseek'};
// Row identifiers carry the service; a reorder event lists them in the new order.
export const row = 'search-driver-';

export function icon(driver) {
  if(Object.hasOwn(logos,driver))return `ailogo:${logos[driver]}-color`;
  if(driver==='tinyfish')return 'globe';
  if(driver==='model')return 'bot';
}

// The saved services in the order a reorder event reports, or null when it is not a permutation of them.
export function reordered(settings,rows) {
  const current=available(settings);
  if(!Array.isArray(rows))return null;
  const order=rows.map(id=>typeof id==='string'&&id.startsWith(row)?id.slice(row.length):'');
  if(order.length!==current.length||new Set(order).size!==order.length||order.some(driver=>!current.includes(driver)))return null;
  return order.every((driver,index)=>driver===current[index])?null:order;
}

export function available(settings) {
  return (settings?.order ?? drivers).filter(driver => drivers.includes(driver) && Object.hasOwn(settings?.providers ?? {},driver));
}

export function needsKey(capability) {
  return capability?.key_required === true;
}

export function models(catalog) {
  return (catalog?.models ?? []).flatMap(item => {
    if(item.kind !== 'provider' || !native.includes(item.api) || item.authentication === 'copilot' || typeof item.id !== 'string')return [];
    const slash=item.id.indexOf('/');
    if(slash < 1 || slash === item.id.length-1)return [];
    return [{provider:item.id.slice(0,slash),provider_name:item.provider,model:item.id.slice(slash+1),default:item.default === true}];
  });
}

// The choices as the model picker's catalog: provider models under their provider's name.
export function popup(choices) {
  return choices.map(item => ({kind:'provider',id:`${item.provider}/${item.model}`,provider:item.provider_name,model:item.model}));
}

// The choice a picker's model ID names, or null for one that is not offered.
export function chosen(choices,id) {
  const model=choices.find(item => `${item.provider}/${item.model}` === id);
  return model ? {provider:model.provider,model:model.model} : null;
}

export function contains(choices,selected) {
  return !!selected && choices.some(item => item.provider === selected.provider && item.model === selected.model);
}

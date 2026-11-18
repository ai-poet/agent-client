// Query choices and pagination are presentation state; core combines canonical reports.
const MINUTE = 60000, HALF_HOUR = 1800000, DAY = 86400000;
/** Range length and the trend bucket that keeps its bars readable. */
export const RANGES = {
  '24h':{days:1, unit:'half_hour'},
  '7d':{days:7, unit:'day'},
  '30d':{days:30, unit:'day'},
  '90d':{days:90, unit:'week'},
  '1y':{days:365, unit:'week'},
};
export const DIMENSIONS = ['model','provider','project'];
const revoked = view => ['not_configured','not_found','permission_denied','revision_conflict'].includes(view?.error?.code);
export const nodeId = value => Array.isArray(value) ? value.map(byte=>byte.toString(16).padStart(2,'0')).join('') : value;
export const resourceId = (node,id) => `${nodeId(node)}:${id}`;
const resource = (value,node) => {const at=value.indexOf(':');return at<0 ? {node,id:value} : {node:value.slice(0,at),id:value.slice(at+1)};};
export const unit = range => RANGES[range]?.unit ?? 'day';
/** The viewer's current offset from UTC; the Node buckets days at this offset. */
export const offset = now => -new Date(now).getTimezoneOffset() || 0;
// Provider rows sum model groups locally; only project rows need another grouping.
export const dimension = breakdown => breakdown === 'project' ? 'project' : 'model';
export function query(range,now = Date.now(),grouping = 'model') {
  const minutes = offset(now), shift = minutes*MINUTE, width = range === '24h' ? HALF_HOUR : DAY;
  // The window ends at the next local boundary so the current bucket is complete.
  const end_ms = (Math.floor((now+shift)/width)+1)*width-shift;
  return {start_ms:end_ms-RANGES[range].days*DAY,end_ms,utc_offset_minutes:minutes,dimension:grouping,
    projects:[],worktrees:[],providers:[],models:[],before:null};
}
export class State {
  constructor(node) {
    this.home=nodeId(node);this.node=this.home;this.project='';this.provider='';this.range='30d';this.breakdown='model';this.expanded=false;this.query=query(this.range);
    this.generation=0;this.cursor='';this.page=1;this.cursors=[null];this.view=null;this.previous=null;this.overview=null;
  }
  get all() {return !this.project && !this.provider;}
  data() {return this.previous ?? this.view?.report ?? null;}
  selected(field) {return field==='project' ? this.project : field==='provider' ? this.provider : this.query.models[0] ?? '';}
  resourceCursor() {return this.overview ? (this.overview.sources ?? []).map(source=>`${nodeId(source.node)}:${source.view?.report?.cursor ?? ''}`).join('|') : this.view?.report?.cursor ?? ''; }
  ready() {return !this.previous && !!this.data() && this.view.connected && !this.view.refreshing && !this.view.partial && !this.view.error;}
  reset() {this.page=1;this.cursors=[null];this.query.before=null;this.previous=null;}
  /** Moves the window to the current time; true when its bounds changed. */
  advance(now = Date.now()) {
    const current=query(this.range,now);
    if(['start_ms','end_ms','utc_offset_minutes'].every(key=>current[key]===this.query[key]))return false;
    Object.assign(this.query,{start_ms:current.start_ms,end_ms:current.end_ms,utc_offset_minutes:current.utc_offset_minutes});
    return true;
  }
  select(field,value) {
    const previous=this.data();
    if (field === 'range') {
      if(!RANGES[value])return false;
      const {dimension,projects,providers,models}=this.query;
      this.range=value;Object.assign(this.query,query(value),{dimension,projects,providers,models});
    }
    else if(field === 'project') {
      const selected=value ? resource(value,this.home) : null;
      this.project=selected ? resourceId(selected.node,selected.id) : '';this.provider='';this.node=selected?.node ?? this.home;
      this.query.projects=selected ? [selected.id] : [];this.query.worktrees=[];this.query.providers=[];this.query.models=[];
    }
    else if(field === 'provider') {
      const selected=value ? resource(value,this.node) : null;
      if(selected && this.project && selected.node!==this.node)return false;
      this.provider=selected ? resourceId(selected.node,selected.id) : '';
      if(!this.project)this.node=selected?.node ?? this.home;
      this.query.providers=selected ? [selected.id] : [];this.query.models=[];
    }
    else if(field === 'model') this.query.models=value ? [value] : [];
    else return false;
    this.reset();this.previous=previous;this.view=null;this.overview=null;return true;
  }
  /** Switches the breakdown; true when the report must be read with another grouping. */
  group(value) {
    if(!DIMENSIONS.includes(value) || value===this.breakdown)return false;
    this.breakdown=value;this.expanded=false;
    if(dimension(value)===this.query.dimension)return false;
    // Request pages do not depend on the grouping, so the current page is kept.
    this.previous=this.data();this.query.dimension=dimension(value);this.view=null;return true;
  }
  turn(page) {
    if(!this.ready() || page<1 || page===this.page || page>this.cursors.length+1)return false;
    const data=this.data();
    if(page>this.cursors.length) {
      if(page!==this.page+1 || !data.requests.has_more || !data.requests.items.length)return false;
      this.cursors.push(data.requests.items.at(-1).position);
    }
    this.previous=data;this.page=page;this.query.before=this.cursors[page-1];this.view=null;return true;
  }
  start() {this.cursor='';return ++this.generation;}
  accept(event,generation) {
    if(generation!==this.generation ||
      this.cursor && (event.cursor.length<this.cursor.length || event.cursor.length===this.cursor.length && event.cursor<this.cursor))return false;
    this.cursor=event.cursor;this.overview=event.all ? event.view : null;
    if(event.all) {
      const sources=event.view?.sources ?? [],summary=event.view?.summary;
      const failure=event.view?.error ?? (!summary ? sources.find(source=>source.view?.error)?.view.error : null);
      this.view={report:summary ? {...summary,query:{...this.query},groups:(summary.groups ?? []).map(item=>({...item.group,node:nodeId(item.node)}))} : null,
        connected:sources.some(source=>source.view?.connected),refreshing:sources.some(source=>source.view?.refreshing),
        partial:!!summary && !event.view.complete,error:failure};
    } else this.view=event.view;
    if(revoked(this.view))this.previous=null;
    if(this.view?.report && !this.view.refreshing)this.previous=null;
    return true;
  }
}
export function choices(state,inventory,resources,text) {
  const projects=new Map([['',text.usage_all_projects]]),providers=new Map([['',text.usage_all_providers]]),models=new Set();
  const inventories=Array.isArray(inventory) ? inventory : inventory ? [inventory] : [];
  const multi=inventories.length>1;
  for(const source of inventories) {
    const node=nodeId(source.node ?? state.node),label=name=>multi ? `${name} · ${source.label ?? node.slice(0,8)}` : name;
    for(const project of source.projects ?? [])projects.set(resourceId(node,project.id),label(project.name));
    if(state.project && node!==state.node)continue;
    for(const provider of source.providers ?? []) {
      providers.set(resourceId(node,provider.id),label(provider.name));
      if(!state.provider || state.provider===resourceId(node,provider.id))for(const model of provider.models)models.add(model);
    }
  }
  for(const key of resources) {
    const node=nodeId(key.node ?? state.node),id=resourceId(node,key.data);
    if(state.project && node!==state.node)continue;
    if(key.kind==='provider' && !providers.has(id))providers.set(id,key.data);
    if(key.kind==='model' && (!state.provider || state.provider===resourceId(node,key.data.provider)))models.add(key.data.model);
  }
  const entries=map=>[...map].map(([id,label])=>({id,label}));
  const keep=(map,value)=>{if(value&&!map.has(value))map.set(value,value);return entries(map);};
  return {project:keep(projects,state.project),provider:keep(providers,state.provider),
    model:keep(new Map([['',text.usage_all_models],...[...models].sort().map(value=>[value,value])]),state.query.models[0])};
}

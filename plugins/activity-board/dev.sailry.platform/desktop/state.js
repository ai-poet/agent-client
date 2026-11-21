// Shared Client supplies lanes; this module owns workbench grouping and previews.
export const lanes = ['waiting','running','completed','idle'];
export const labels = {waiting:'activity_board_waiting',running:'activity_board_running',
  completed:'activity_completed',idle:'activity_idle',failed:'activity_failed'};
export const group = lane => lane === 'failed' ? 'completed' : lane;
export const statusColors = {running:'primary',idle:'muted_foreground',approval:'warning',input:'chart_3',
  completed:'success',failed:'destructive'};
// Read results rest in the idle group and use its neutral color.
export function statusColor(item) {
  if (item.state === 'idle') return statusColors.idle;
  return statusColors[item.lane === 'waiting' ? item.approval ? 'approval' : 'input' : item.lane] ?? statusColors.idle;
}
// Dots follow the run status in the label, so read results keep their outcome color.
export function statusTone(session) {
  const {run,waiting,queued} = session.activity;
  if (queued > 0 && !['running','stopping'].includes(run?.status)) return 'idle';
  if (run?.status === 'running') return waiting === 'approval' ? 'approval' : waiting === 'input' ? 'input' : 'running';
  return {stopping:'running',completed:'completed',failed:'failed'}[run?.status] ?? 'idle';
}
export const dotColor = item => statusColors[item.tone] ?? statusColors.idle;
// The shared Client orders activity items; packages only look up positions.
export function order(catalog) {
  const positions = new Map((catalog.activity_order ?? []).map((key,index) => [key,index]));
  return item => positions.get(`${item.kind}:${item.id}`) ?? positions.size;
}
const defaultAppearance = {icon:'folder',color:'none'};

export function sessionStatus(session,text) {
  const {run,waiting,queued} = session.activity;
  if (queued > 0 && !['running','stopping'].includes(run?.status)) return `${text.turn_queued} ${queued}`;
  const key = run?.status === 'running' ? waiting === 'approval' ? 'activity_approval' : waiting === 'input'
    ? 'activity_input' : 'activity_running' : {queued:'turn_queued',stopping:'chat_stopping',completed:'activity_completed',
      failed:'activity_failed',interrupted:'chat_interrupted',cancelled:'turn_cancelled'}[run?.status] ?? 'activity_idle';
  return queued > 0 ? `${text[key]} · ${text.turn_queued} ${queued}` : text[key];
}

export function clock(timestamp,now = Date.now()) {
  const date = new Date(timestamp), today = new Date(now), pad = value => String(value).padStart(2,'0');
  const time = `${pad(date.getHours())}:${pad(date.getMinutes())}`;
  return date.toDateString() === today.toDateString() ? time : `${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${time}`;
}

// Finished runs report elapsed time; active runs show only their start time.
export function duration(run) {
  if (!Number.isFinite(run?.started_ms) || !Number.isFinite(run?.finished_ms)) return null;
  const seconds = Math.max(0,Math.round((run.finished_ms - run.started_ms) / 1000));
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m${seconds % 60 ? `${seconds % 60}s` : ''}`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h${Math.floor(seconds % 3600 / 60) ? `${Math.floor(seconds % 3600 / 60)}m` : ''}`;
  return `${Math.floor(seconds / 86400)}d${Math.floor(seconds % 86400 / 3600) ? `${Math.floor(seconds % 86400 / 3600)}h` : ''}`;
}

// Previews are Markdown; cards show their plain reading text.
export function plain(text) {
  return text.split('\n')
    .filter(line => !/^\s*\|?\s*:?-{3,}/.test(line))
    .map(line => line.replace(/^\s*(#{1,6}\s+|>\s?|[-*+]\s+(\[[ xX]\]\s+)?|\d+[.)]\s+)/,'').replace(/^\s*\|(.*)\|\s*$/,'$1').replace(/\s*\|\s*/g,' · '))
    .join(' ').replace(/!?\[([^\]]*)\]\([^)]*\)/g,'$1').replace(/(\*\*|__|~~|`)/g,'')
    .replace(/\s+/g,' ').trim();
}

export function sections(catalog,previews,text) {
  const columns = Object.fromEntries(lanes.map(lane => [lane,new Map()]));
  if (!catalog) return Object.fromEntries(lanes.map(lane => [lane,[]]));
  const trees = new Map(catalog.worktrees.map(tree => [tree.id,tree]));
  const projects = new Map(catalog.projects.map(project => [project.id,project]));
  const unread = new Set(catalog.unread_terminals);
  function add(state,projectId,card) {
    const column = columns[state];
    if (!column) return;
    if (!column.has(projectId)) {
      const project = projects.get(projectId);
      column.set(projectId,{id:projectId,name:project?.name ?? text.activity_no_project,
        appearance:project?.appearance ?? defaultAppearance,cards:[]});
    }
    card.state = state;column.get(projectId).cards.push(card);
  }
  for (const session of [...catalog.sessions].reverse()) {
    if (session.archived || session.delegation) continue;
    const tree = trees.get(session.worktree), lane = catalog.session_lanes[session.id];
    if (!tree || !lane) continue;
    const project = projects.get(session.project), preview = previews.get(session.id);
    const status = sessionStatus(session,text);
    const content = preview?.turn === (session.activity.run?.turn ?? null) ? preview.text : null;
    add(group(lane),session.project,{kind:'session',id:session.id,lane,
      approval:lane === 'waiting' && session.activity.waiting === 'approval',
      title:session.activity.title.trim().replace(/[\r\n]/g,' ') || text.chat_new,status,tone:statusTone(session),
      time:session.activity.run?.finished_ms ?? session.activity.run?.started_ms ?? null,
      phase:['running','stopping'].includes(session.activity.run?.status)
        ? session.activity.waiting ? 'turn_waiting' : 'turn_tools_running' : null,
      content:content?.trim() ? ['failed','waiting'].includes(lane) ? `${status} · ${content}` : content : status,
      preview:content?.trim() ? content : null,duration:duration(session.activity.run),
      project:project?.name ?? '',path:session.project ? tree.path : '',appearance:project?.appearance ?? defaultAppearance});
  }
  for (const terminal of catalog.terminals) {
    const tree = trees.get(terminal.worktree), lane = catalog.terminal_lanes[terminal.id];
    if (!tree || !lane) continue;
    const state = ['completed','failed'].includes(lane) ? unread.has(terminal.id) ? 'completed' : 'idle' : group(lane);
    const project = projects.get(tree.project), status = text[lane === 'waiting' ? 'activity_input' : labels[lane]];
    add(state,tree.project,{kind:'terminal',id:terminal.id,lane,
      title:terminal.title?.trim() ? terminal.title : text.terminal,status,tone:lane === 'waiting' ? 'input' : lane,content:status,
      preview:null,duration:null,time:null,phase:lane === 'running' ? 'turn_tools_running' : null,
      project:project?.name ?? '',path:tree.project ? tree.path : '',appearance:project?.appearance ?? defaultAppearance});
  }
  const position = order(catalog);
  for (const column of Object.values(columns)) for (const section of column.values())
    section.cards.sort((left,right) => position(left) - position(right));
  return Object.fromEntries(lanes.map(lane => [lane,[...columns[lane].values()]
    .sort((left,right) => left.id === null ? -1 : right.id === null ? 1 : left.id.localeCompare(right.id))]));
}

export class State {
  constructor(read,readPreviews,changed,report = () => {}) {
    this.read = read;this.readPreviews = readPreviews;this.changed = changed;this.report = report;
    this.catalog = null;this.previews = new Map();this.connected = true;
    this.error = false;this.pending = null;this.again = false;this.generation = 0;this.visible = false;
  }
  async refresh() {
    if (this.pending) {this.again = true;return this.pending;}
    this.pending = this.load();
    try {await this.pending;} finally {
      this.pending = null;
      if (this.again) {this.again = false;await this.refresh();}
    }
  }
  async load() {
    try {
      const catalog = await this.read();
      if (this.catalog && catalog.node === this.catalog.node && BigInt(catalog.cursor) < BigInt(this.catalog.cursor)) return;
      const generation = ++this.generation;
      this.catalog = catalog;this.error = false;this.changed();
      this.previewTask = this.loadPreviews(catalog,generation);
    } catch (_) {this.error = true;this.report();this.changed();}
  }
  async loadPreviews(catalog,generation) {
    const ids = catalog.sessions.filter(session => !session.archived && !session.delegation).reverse().map(session => session.id);
    const batches=Array.from({length:Math.ceil(ids.length/8)},(_,index)=>ids.slice(index*8,index*8+8));
    await Promise.all(batches.map(async sessions=>{
      if (generation !== this.generation) return;
      let result;
      try {result = await this.readPreviews(sessions);}
      catch (_) {return; /* Summaries remain usable when optional previews fail. */}
      if (generation !== this.generation) return;
      for (const preview of result) this.previews.set(preview.session,preview);
      this.changed();
    }));
  }
  hosts(value) {
    const resumed = value.visible && !this.visible;
    // Terminal read state also feeds the shared order, so a change rereads it.
    const reordered = !!this.catalog && String(this.catalog.unread_terminals) !== String(value.unread_terminals);
    this.visible = value.visible;
    if (!this.visible) this.generation += 1;
    if (this.catalog) {
      this.catalog.hosts = value.hosts;this.catalog.unread_terminals = value.unread_terminals;
      this.changed();
    }
    if (resumed || reordered && this.visible) this.refresh();
  }
}

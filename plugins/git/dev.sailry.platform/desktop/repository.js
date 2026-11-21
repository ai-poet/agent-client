// Scoped reads and command policy use the public SDK; Node owns Git and receipts.
const defaults = () => ({signoff:false,skip_hooks:false,tracked:false,all:false,after:'none',push_remote:null});

export function failure(kind, code) {
  if (kind === 'index') return ({revision_conflict:'git_index_changed',busy:'git_index_busy',permission_denied:'git_index_denied',not_found:'git_index_missing',outcome_unknown:'git_index_unknown'})[code] ?? 'git_index_failed';
  if (code === 'revision_conflict') return ['switch','delete','merge'].includes(kind) ? 'git_branch_revision' : 'git_index_conflict';
  if (['commit','merge'].includes(kind) && code === 'not_configured') return 'git_identity_missing';
  if (['commit','merge'].includes(kind) && code === 'unavailable') return 'git_commit_unavailable';
  if (['switch','merge'].includes(kind) && code === 'busy') return 'git_branch_busy';
  if (code === 'conflict') return ({switch:'git_branch_conflict',merge:'git_merge_conflict',delete:'git_branch_delete_conflict',action:'git_action_conflict'})[kind] ?? 'git_commit_failed';
  if (kind === 'action' && code === 'permission_denied') return 'git_auth_failed';
  const prefix = ({switch:'git_branch',delete:'git_branch_delete',merge:'git_merge',commit:'git_commit',action:'git_action',prune:'worktree_prune'})[kind] ?? 'git_action';
  return `${prefix}_${code === 'outcome_unknown' ? 'unknown' : 'failed'}`;
}

export class Repository {
  constructor(api, notify = () => {}, report = () => {}, pause = async () => {}) {
    this.api = api; this.notify = notify; this.report = report;
    this.pause = pause; this.visible = true; this.dirty = false; this.lastRead = 0;
    this.connected = true; this.status = null; this.branches = null; this.loading = false;
    this.error = null; this.readCode = null; this.pending = null; this.followUp = null;
    this.message = ''; this.amend = false; this.amendDraft = null; this.options = defaults(); this.commitOpen = false;
    this.view = 'changes'; this.history = null; this.historyLoading = false;
    this.historyCursors = new Map();
    this.tabs = []; this.selected = null; this.documents = new Map();
    this.read = 0; this.historyRead = 0; this.documentRead = 0; this.closed = false;
    this.refreshing = null;
    this.references = null; this.loadedReferences = null; this.branchesDirty = true;
    this.navigation = {hierarchical:true,grouping:'tracked',sort:'path'};
  }

  writable() { return this.connected && !this.loading && !this.pending && this.status?.index_revision != null; }
  canInitialize() { return this.connected && !this.loading && !this.pending && this.status?.kind === 'directory'; }
  canCommit() { return this.writable() && this.message.trim() !== '' && !this.status.entries.some(entry => entry.conflicted)
    && (this.amend ? this.status.head !== null : this.status.entries.length > 0); }
  revisions() { return {expected_index:this.status.index_revision,expected_head:this.status.head,expected_branch:this.status.branch}; }

  refresh(review = false, quiet = false) {
    if (!quiet) this.branchesDirty = true;
    this.readCode = null;
    if (this.closed || !this.connected || !this.visible || this.pending?.busy) {
      this.dirty = !this.closed;
      return Promise.resolve(false);
    }
    const previous = this.refreshing;
    if (previous?.active && previous.sequence === this.read && this.loading) {
      previous.review ||= review;
      previous.quiet &&= quiet;
      return previous.promise;
    }
    const current = {sequence:++this.read,review:review || !!(previous?.active && previous.sequence === this.read - 1 && previous.review),quiet,followup:false,active:true,promise:null};
    this.dirty = false;
    this.refreshing = current;
    current.promise = this.reload(current);
    return current.promise;
  }

  invalidate() {
    const current = this.refreshing;
    if (current?.active && current.sequence === this.read) {
      current.invalidated = true;
      return current.promise;
    }
    return this.refresh(false,true);
  }

  latest(sequence) {
    const current = this.refreshing;
    return !this.closed && current?.sequence === this.read && current.sequence > sequence ? current.promise : false;
  }

  async reload(current) {
    const sequence = current.sequence;
    this.loading = true; this.notify();
    try {
      while (!this.closed && this.connected && this.visible) {
        const delay = 750 - (Date.now() - this.lastRead);
        if ((current.quiet || current.followup) && delay > 0) await this.pause(delay);
        if (this.closed || sequence !== this.read) return this.latest(sequence);
        if (!this.connected || !this.visible) { this.dirty = true; return false; }
        this.lastRead = Date.now(); current.invalidated = false;
        try {
          const status = await this.api.inspectGit();
          if (this.closed || sequence !== this.read) return this.latest(sequence);
          const report = !current.quiet || !this.status;
          const identityChanged = !this.status || this.status.kind !== status.kind || this.status.head !== status.head || this.status.branch !== status.branch;
          this.status = status;
          this.readCode = null;
          if (!this.connected || !this.visible) { this.dirty = true; return false; }
          let branches = this.branches, branchError = null;
          const references = this.references;
          try {
            if (status.kind === 'directory') branches = null;
            else if (!branches || this.branchesDirty || identityChanged || this.loadedReferences !== references) branches = await this.api.listGitBranches();
          }
          catch (_) { branchError = 'git_branches_failed'; }
          if (this.closed || sequence !== this.read) return this.latest(sequence);
          if (!this.connected || !this.visible) { this.dirty = true; return false; }
          if (!branchError) {this.branches = branches;this.loadedReferences = references;this.branchesDirty = false;}
          if (!branchError || report) this.error = branchError;
          if (branchError && report && !current.invalidated) this.report(branchError);
          if (this.pending?.reviewing || current.review) {
            if (this.pending) this.api.forgetRequest(this.pending.id);
            this.pending = null;
          }
          this.loading = false;
          if (!current.invalidated) {
            const document = this.documents.get(this.selected);
            await Promise.all([
              document ? this.open(document.request,false,current.quiet) : Promise.resolve(),
              this.view === 'history' ? this.loadHistory(null,current.quiet) : Promise.resolve(),
            ]);
          }
          if (this.closed || sequence !== this.read) return this.latest(sequence);
          if (!current.invalidated) return true;
        } catch (error) {
          if (sequence !== this.read || this.closed) return this.latest(sequence);
          this.readCode = this.api.faultCode(error.message ?? String(error));
          if (!current.invalidated && this.visible && (!current.quiet || !this.status)) {
            this.error = 'git_read_failed'; this.report(this.error);
          }
          if (!current.invalidated) return false;
        }
        current.followup = true;
        this.loading = true; this.notify();
      }
      this.dirty = !this.closed;
      return false;
    } finally {
      current.active = false;
      if (sequence === this.read) this.loading = false;
      this.notify();
    }
  }

  toggleAmend() {
    if (!this.status?.head) return;
    this.amend = !this.amend;
    if (this.amend) { this.amendDraft = this.message; this.message = this.branches?.sync.head_message ?? ''; }
    else { this.message = this.amendDraft ?? ''; this.amendDraft = null; }
    this.notify();
  }

  commitChoices(scope = 'auto') {
    if (scope !== 'auto' || this.amend || this.status?.entries.some(entry => entry.staged !== null)) return null;
    return this.status?.entries.some(entry => !entry.untracked) ? ['tracked','all'] : ['all'];
  }

  async commit(scope = 'auto') {
    if (!this.canCommit() || this.commitChoices(scope)) return false;
    const message = this.message;
    const options = {...this.options,tracked:scope === 'tracked',all:scope === 'all'};
    return this.change('commit',() => this.api.prepareGitCommit({message,amend:this.amend,options,...this.revisions()}),{message});
  }

  async index(paths, operation) {
    if (!paths.length || !this.writable()) return false;
    const {expected_index,expected_head} = this.revisions();
    return this.change('index',() => this.api.prepareGitIndex({paths:[...paths],operation,expected_index,expected_head}));
  }

  async action(action) {
    if (!this.writable()) return false;
    const kind = action.prune_worktree ? 'prune' : action.switch_tracking || action.switch_stashing || action.switch_remote ? 'switch' : 'action';
    return this.change(kind,() => this.api.prepareGitChange({kind:'action',action,...this.revisions()}),{action});
  }

  initialize() {
    if (!this.canInitialize()) return Promise.resolve(false);
    return this.change('initialize',() => this.api.prepareGitChange({kind:'action',action:'initialize',expected_index:'',expected_head:null,expected_branch:null}));
  }

  async branch(kind, draft) {
    if (!this.writable()) return false;
    const revisions = ['create','rename'].includes(kind) ? {} : this.revisions();
    if (kind === 'delete') delete revisions.expected_index;
    return this.change(kind,() => this.api.prepareGitChange({kind,...draft,...revisions}),{branch:draft});
  }

  async change(kind, prepare, details = {}) {
    if (!(kind === 'initialize' ? this.canInitialize() : this.writable())) return false;
    let id;
    try { id = prepare(); }
    catch (error) { this.error = failure(kind,this.api.faultCode(error.message ?? String(error))); this.report(this.error); this.notify(); return false; }
    ++this.read; ++this.documentRead; ++this.historyRead;
    this.loading = false; this.historyLoading = false;
    this.pending = {id,kind,...details,busy:false,reviewing:false};
    return this.complete();
  }

  async complete() {
    const pending = this.pending;
    if (!pending || pending.busy) return false;
    pending.busy = true; this.error = null; this.notify();
    try {
      const receipt = await this.api.completeRequest(pending.id);
      if (this.closed || this.pending !== pending) return false;
      if (receipt.Err) {
        const code = receipt.Err.code;
        this.error = failure(pending.kind,code); pending.error = this.error;
        pending.reviewing = (pending.kind === 'index' || pending.kind === 'prune') && code !== 'outcome_unknown'
          || pending.kind === 'action' && code === 'conflict';
        this.report(this.error);
        return false;
      }
      const result = receipt.Ok;
      if (!result || !['git_index','git_commit_created','git_action_completed','git_branch_created','git_branch_renamed','git_branch_deleted','git_branch_switched','git_merged'].includes(result.kind)) {
        this.error = failure(pending.kind,'outcome_unknown'); this.report(this.error); return false;
      }
      pending.reviewing = true;
      if (pending.kind === 'commit') {
        if (this.message === pending.message) this.message = '';
        this.followUp = result.data?.follow_up ?? null;
        if (this.followUp) this.report('git_commit_sync_failed');
        this.options = defaults(); this.amend = false; this.amendDraft = null; this.commitOpen = false;
      }
      if (pending.action?.push || pending.action?.sync) this.followUp = null;
      if (pending.kind !== 'index') this.history = null;
      return true;
    } catch (_) {
      if (!this.closed && this.pending === pending) {
        this.error = failure(pending.kind,'outcome_unknown'); this.report(this.error);
      }
      return false;
    } finally {
      pending.busy = false;
      if (!this.closed && this.pending === pending && pending.reviewing) await this.refresh();
      this.notify();
    }
  }

  async loadHistory(cursor = null, quiet = false, revision = null) {
    if (this.closed || !this.connected || !this.visible || this.pending?.busy) return;
    const sequence = ++this.historyRead;
    this.historyLoading = true; this.notify();
    try {
      const page = await this.api.readGitLog(100,cursor,{revision});
      if (this.closed || sequence !== this.historyRead) return;
      if (cursor === null && revision === null) this.historyCursors.clear();
      const current = Math.floor((page.offset ?? 0) / 100) + 1;
      this.historyCursors.set(current,cursor);
      if (page.next) this.historyCursors.set(current + 1,page.next);
      else this.historyCursors.delete(current + 1);
      this.history = page;
    } catch (_) { if (sequence === this.historyRead && !this.closed && this.visible && (!quiet || !this.history)) { this.error = 'git_read_failed'; this.report(this.error); } }
    finally { if (sequence === this.historyRead) this.historyLoading = false; this.notify(); }
  }

  historyPage(page) {
    if (!this.history?.head || !Number.isInteger(page) || page < 1 || this.pending) return;
    if (!this.historyCursors.has(page)) return;
    return this.loadHistory(this.historyCursors.get(page),false,this.history.head);
  }

  async open(request, select = true, quiet = false) {
    if (this.closed || !this.visible || this.pending?.busy) return;
    const sequence = ++this.documentRead;
    const id = request.kind === 'file' ? request.path : request.kind === 'changes' ? 'changes:'
      : request.kind === 'output' ? 'output:' : `${request.kind}:${request.commit}`;
    this.documentLoading = true; this.notify();
    try {
      let content;
      if (request.kind === 'file') content = {files:[await this.api.readGitDiff(request.path,request.scope ?? 'all')]};
      else if (request.kind === 'output') content = await this.api.readGitOutput();
      else if (request.kind === 'commit') content = await this.api.readGitCommit(request.commit);
      else if (request.kind === 'stash') content = {files:[await this.api.readGitStash(request.commit)]};
      else {
        const status = this.status ?? await this.api.inspectGit();
        const files = []; let size = 0, truncated = false;
        const diffs = await Promise.all(status.entries.map(entry => this.api.readGitDiff(entry.path,'all')));
        for (const diff of diffs) {
          files.push(diff);
          size += utf8Length(diff.text);
          if (size >= 1024 * 1024) { truncated = true; break; }
        }
        content = {files,truncated};
      }
      if (this.closed || sequence !== this.documentRead || (!select && !this.tabs.includes(id))) return;
      if (select) {
        if (!this.tabs.includes(id)) this.tabs.push(id);
        this.selected = id;
      }
      this.documents.set(id,{id,request,...content});
      this.error = content.truncated || content.files?.some(file => file.truncated) ? 'git_diff_partial'
        : content.binary || content.files?.some(file => file.binary) ? 'git_diff_binary' : null;
    } catch (_) {
      if (sequence === this.documentRead && !this.closed && this.visible && (!quiet || !this.documents.has(id))) { this.error = 'git_read_failed'; this.report(this.error); }
    } finally { if (sequence === this.documentRead) this.documentLoading = false; this.notify(); }
  }

  close(id) {
    const index = this.tabs.indexOf(id);
    if (index < 0) return;
    this.tabs.splice(index,1); this.documents.delete(id);
    if (this.selected === id) { ++this.documentRead; this.selected = this.tabs[Math.min(index,this.tabs.length - 1)] ?? null; }
    this.notify();
  }

  stop() { this.closed = true; ++this.read; ++this.historyRead; ++this.documentRead; }
}

// QuickJS does not expose TextEncoder; count the UTF-8 representation of scalars.
function utf8Length(text) {
  let size = 0;
  for (const character of text) { const code = character.codePointAt(0); size += code < 128 ? 1 : code < 2048 ? 2 : code < 65536 ? 3 : 4; }
  return size;
}

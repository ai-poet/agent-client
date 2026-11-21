// Package-owned forms and retries; core retains source scope, admission and session moves.
import {listing,registered,menu,errorKey,current} from './policy.js';

export class Controller {
  constructor(api,text,changed) {
    this.api = api; this.text = text; this.changed = changed;
    this.location = api.readLocation(); this.serial = 0; this.dialog = null; this.catalog = null;
    this.entries = null; this.catalogRead = null; this.operation = null; this.disposed = false;
  }
  notify() { if (!this.disposed) this.changed(); }
  owner(kind, extra = {}) {
    return {token:`worktrees-${++this.serial}`,kind,bound:this.location,pending:false,error:null,request:null,uncertain:false,...extra};
  }
  accept(location) {
    if (['node','project','worktree'].some(key => location[key] !== this.location[key])) {
      this.catalog = null; this.entries = null; this.catalogRead = null;
    }
    this.location = location;
    if (this.dialog && ['list','actions'].includes(this.dialog.kind) && !this.valid(this.dialog)) this.close();
    this.notify();
  }
  invalidate(owner) {
    if (this.disposed || owner && (owner.bound.node !== this.location.node || owner.bound.project !== this.location.project))
      return Promise.resolve(null);
    this.catalog = null; this.entries = null; this.catalogRead = null;
    this.notify();
    return this.refresh();
  }
  refresh(force = false) {
    if (!this.location.project || !this.location.connected || this.disposed) return Promise.resolve(null);
    if (this.catalogRead && (!force || this.catalogRead.pending)) return this.catalogRead.done;
    const read = {pending:true};
    this.catalogRead = read;
    const discovery = this.api.listWorktrees().then(value => ({value}),error => ({error}));
    read.registered = this.api.readWorktreeCatalog().catch(() => null);
    read.done = (async () => {
      try {
        const catalog = await read.registered;
        if (this.catalogRead !== read || this.disposed) return null;
        if (!catalog) throw Error('Worktree catalog is unavailable');
        this.catalog = catalog; this.notify();
        const found = await discovery;
        if (this.catalogRead !== read || this.disposed) return null;
        if (Object.hasOwn(found,'error')) throw found.error;
        this.entries = found.value.entries; this.notify();
        return found.value;
      } catch (_) {
        if (this.catalogRead === read && !this.disposed) {
          this.catalogRead = null;
        }
        return null;
      } finally { read.pending = false; }
    })();
    return read.done;
  }
  choices() { return menu(this.location,this.text); }
  selected() { return this.location.project ? `project:${this.location.project}` : 'unassigned'; }
  async select(id) {
    this.accept(this.api.readLocation());
    const owner = this.owner('location');
    if (!this.location.can_move || !this.choices().some(choice => choice.id === id)) {
      this.fail(owner,'worktree_context_changed'); return;
    }
    if (id === this.selected()) return;
    const selection = id === 'unassigned' ? {project:null} : id === 'add_project' ? {add_project:true}
      : id === 'projects' ? {projects:true} : {project:id.slice('project:'.length)};
    try { await this.api.selectLocation(this.location.cursor,selection); }
    catch (_) { this.fail(owner,'worktree_context_changed'); return; }
    // A project change replaces this view, so its location may already be closed.
    try { this.accept(this.api.readLocation()); } catch (_) {}
  }
  valid(owner) { return current(owner.bound,this.api.readLocation()); }
  show(owner) { if (this.dialog !== owner) this.close(); this.dialog = owner; this.notify(); return owner; }
  close() {
    const owner = this.dialog;
    this.dialog = null;
    if (owner && !owner.pending) this.forget(owner);
    this.notify();
  }
  forget(owner) {
    if (owner.request) { this.api.forgetRequest(owner.request); owner.request = null; }
  }
  settle(owner) {
    owner.pending = false;
    if (this.dialog !== owner) this.forget(owner);
    this.notify();
  }
  fail(owner,key) {
    owner.error = key;
    this.api.toast({id:owner.token,message:this.text(key),kind:'error'});
    this.notify();
  }
  async open(intent = 'manage', entry) {
    this.accept(this.api.readLocation());
    if (!this.location.can_move) return;
    if (!this.location.project) { await this.api.selectLocation(this.location.cursor,{projects:true}); return; }
    if (intent === 'create' && entry) return this.create({branch:entry.branch,head:entry.commit});
    if (intent === 'fork' && entry?.base) return this.createManaged(true,entry.base);
    if (intent === 'create' || intent === 'fork') return this.createManaged(intent === 'fork');
    if (this.dialog?.loading && this.dialog.kind === 'list' && this.valid(this.dialog)) return this.dialog.read;
    const owner = this.show(this.owner('list',{loading:true,choices:[]}));
    owner.read = this.load(owner,entry);
    return owner.read;
  }
  async load(owner,entry) {
    try {
      const discovery = this.refresh(true);
      const catalog = this.location.connected ? await this.catalogRead?.registered : null;
      if (this.dialog !== owner || !this.valid(owner) || this.disposed) return;
      if (!catalog) { this.fail(owner,'worktree_load_failed'); return; }
      owner.choices = registered(catalog,owner.bound,this.text); this.notify();
      const found = await discovery;
      if (this.dialog !== owner || !this.valid(owner) || this.disposed) return;
      if (!found) { this.fail(owner,'worktree_load_failed'); return; }
      const choices = listing(catalog,found,owner.bound,this.text);
      if (owner.kind === 'list') {
        owner.choices = choices;
        if (entry) return this.create(entry);
      } else if (owner.kind === 'actions') owner.previous = choices;
    } catch (_) { if (this.dialog === owner && this.valid(owner) && !this.disposed) this.fail(owner,'worktree_load_failed'); }
    finally { owner.loading = false; this.notify(); }
  }
  async choose(id, captured) {
    const owner = this.dialog;
    if (!owner || owner.pending) return;
    const choice = captured?.token === owner.token ? captured.choice : owner.choices?.find(choice => choice.id === id && !choice.disabled);
    if (!choice?.action) return;
    if (!this.valid(owner)) { this.fail(owner,'worktree_context_changed'); return; }
    const action = choice.action;
    if (action.kind === 'actions') {
      owner.previous = owner.choices;
      owner.choices = action.actions.map((action,index) => ({id:`action-${index}`,label:this.text(action.kind === 'open' ? 'workspace_open_worktree' : 'worktree_remove'),action}));
      owner.title = choice.label; owner.kind = 'actions'; owner.loading = false; this.notify();
    } else if (action.kind === 'managed') await this.createManaged(action.fork);
    else if (action.kind === 'create') this.create(action.entry);
    else if (action.kind === 'prune') await this.prune(owner,action.entry);
    else await this.register(owner,action);
  }
  back() {
    const owner = this.dialog;
    if (!owner || owner.pending || owner.kind !== 'actions') return;
    owner.choices = owner.previous; owner.previous = null; owner.kind = 'list'; this.notify();
  }
  create(entry) {
    const owner = this.owner('create',{values:{revision:entry.branch || entry.head || '',branch:'',path:''},fields:{}});
    this.show(owner); return owner;
  }
  /** A `base` ({from, commit}) starts the checkout at another branch without source changes. */
  async createManaged(fork,base = null) {
    const valid = base && typeof base.from === 'string' && /^[0-9a-f]{40}$/i.test(base.commit ?? '');
    const owner = this.show(this.owner('managed',{fork,base:valid ? {from:base.from,commit:base.commit} : null,
      values:{branch:`sailry/task-${this.api.newId().slice(0,8)}`},fields:{},include_changes:!valid,status:null,pending:true}));
    try { owner.status = await this.api.inspectGit(); }
    catch (_) { if (this.dialog === owner) this.fail(owner,'worktree_load_failed'); }
    finally { owner.pending = false; this.notify(); }
    return owner;
  }
  async register(owner,action) {
    if (owner.pending) return;
    owner.pending = true; this.operation = {owner,action}; this.notify();
    try {
      let id = action.worktree;
      if (!id) {
        const signature = `register:${action.entry.path}`;
        if (owner.request && owner.signature !== signature) {
          if (owner.uncertain) { this.fail(owner,'worktree_outcome_unknown'); return; }
          this.api.forgetRequest(owner.request); owner.request = null;
        }
        if (!owner.request) {owner.signature = signature; owner.request = this.api.prepareWorktreeChange({kind:'register',project:owner.bound.project,path:action.entry.path});}
        const result = await this.api.completeRequest(owner.request);
        if (result.Err) {
          owner.uncertain = result.Err.code === 'outcome_unknown'; this.fail(owner,errorKey(result.Err.code));
          this.api.toast({id:owner.token,message:this.text(errorKey(result.Err.code)),kind:'error',action:{id:'retry',label:this.text('worktree_retry')}}); return;
        }
        id = result.Ok.data.id;
        owner.uncertain = false;
        this.invalidate(owner);
      }
      if (this.dialog !== owner) return;
      if (!this.valid(owner)) { this.fail(owner,'worktree_context_changed'); return; }
      if (action.kind === 'open') {
        await this.api.selectLocation(owner.bound.cursor,{worktree:id,fork:false,...(owner.request ? {request:owner.request} : {})});
        if (owner.request) {this.api.forgetRequest(owner.request); owner.request = null;}
        if (this.dialog === owner) this.close();
      } else {
        if (await this.api.worktreeHasDrafts(id,owner.request ?? undefined)) { this.fail(owner,'worktree_remove_unsaved'); return; }
        if (owner.request) {this.api.forgetRequest(owner.request); owner.request = null;}
        const removal = this.owner('remove',{entry:action.entry,worktree:id});
        removal.bound = owner.bound;
        this.show(removal);
      }
    } catch (_) {
      owner.uncertain = !!owner.request;
      this.fail(owner,owner.request ? 'worktree_outcome_unknown' : 'worktree_context_changed');
      if (owner.request) this.api.toast({id:owner.token,message:this.text('worktree_outcome_unknown'),kind:'error',action:{id:'retry',label:this.text('worktree_retry')}});
    } finally { this.settle(owner); }
  }
  async submit(values = this.dialog?.values) {
    const owner = this.dialog;
    if (!owner || owner.pending) return;
    if (!owner.request && !this.valid(owner)) { this.fail(owner,'worktree_context_changed'); return; }
    if (owner.kind === 'create') await this.submitCreate(owner,values);
    else if (owner.kind === 'managed') await this.submitManaged(owner,values);
    else if (owner.kind === 'remove') await this.submitRemoval(owner);
  }
  async submitCreate(owner,values) {
    const signature = JSON.stringify(values);
    if (owner.uncertain && owner.signature !== signature) return;
    owner.pending = true; owner.error = null; owner.values = {...values}; this.notify();
    try {
      if (owner.request && owner.signature !== signature) { this.api.forgetRequest(owner.request); owner.request = null; }
      if (!owner.request) {
        let resolved;
        try { resolved = await this.api.resolveGitRevision(values.revision); }
        catch (_) { this.fail(owner,'worktree_revision_failed'); return; }
        if (this.dialog !== owner) return;
        if (!this.valid(owner)) { this.fail(owner,'worktree_context_changed'); return; }
        owner.request = this.api.prepareWorktreeChange({kind:'create',project:owner.bound.project,path:values.path,branch:values.branch,commit:resolved.commit});
        owner.signature = signature;
      }
      await this.execute(owner);
    } finally { this.settle(owner); }
  }
  async submitManaged(owner,values) {
    const branch = values.branch.trim();
    if (!owner.status) { this.fail(owner,'worktree_load_failed'); return; }
    if (!owner.base && (!owner.status.head || !owner.status.index_revision)) { this.fail(owner,'worktree_commit_first'); return; }
    if (!branch) { this.fail(owner,'git_branch_name_required'); return; }
    owner.pending = true; owner.error = null; this.notify();
    try {
      if (!owner.request) owner.request = this.api.prepareWorktreeChange(owner.base
        ? {kind:'managed',project:owner.bound.project,branch,expected_head:'',expected_index:'',include_changes:false,base:owner.base.commit}
        : {kind:'managed',project:owner.bound.project,branch,
          expected_head:owner.status.head,expected_index:owner.status.index_revision,include_changes:owner.include_changes});
      await this.execute(owner);
    } finally { this.settle(owner); }
  }
  async submitRemoval(owner) {
    if (!owner.request && !this.valid(owner)) { this.fail(owner,'worktree_context_changed'); return; }
    if (await this.api.worktreeHasDrafts(owner.worktree)) { this.fail(owner,'worktree_remove_unsaved'); return; }
    if (this.dialog !== owner || owner.pending) return;
    owner.pending = true; owner.error = null; this.notify();
    try {
      if (!owner.request) owner.request = this.api.prepareWorktreeChange({kind:'remove',worktree:owner.worktree,expected_head:owner.entry.head,expected_branch:owner.entry.branch});
      await this.execute(owner);
    } finally { this.settle(owner); }
  }
  async execute(owner) {
    try {
      const result = await this.api.completeRequest(owner.request);
      if (result.Err) { owner.uncertain = result.Err.code === 'outcome_unknown'; this.fail(owner,errorKey(result.Err.code,owner.kind === 'remove')); return; }
      this.invalidate(owner);
      if (owner.kind === 'remove') await this.api.releaseWorktree(owner.worktree,owner.request);
      owner.uncertain = false;
      if (this.dialog !== owner) {this.api.forgetRequest(owner.request);owner.request = null;return;}
      if (owner.kind !== 'remove') {
        if (!this.valid(owner)) { this.fail(owner,owner.kind === 'managed' ? 'location_created_changed' : 'worktree_context_changed'); return; }
        await this.api.selectLocation(owner.bound.cursor,{worktree:result.Ok.data.id,fork:owner.fork ?? false,request:owner.request});
      }
      this.api.forgetRequest(owner.request); owner.request = null;
      if (this.dialog === owner) this.close();
    } catch (_) { owner.uncertain = !!owner.request; this.fail(owner,owner.request ? 'worktree_outcome_unknown' : 'worktree_context_changed'); }
  }
  async prune(owner,entry) {
    owner.pending = true; this.notify();
    try {
      const signature = `prune:${entry.path}`;
      if (owner.request && owner.signature !== signature) {
        if (owner.uncertain) {this.fail(owner,'worktree_outcome_unknown'); return;}
        this.api.forgetRequest(owner.request); owner.request = null;
      }
      if (!owner.request) {
        const status = await this.api.inspectGit();
        if (this.dialog !== owner || !this.valid(owner)) return;
        owner.signature = signature; owner.request = this.api.prepareGitChange({kind:'action',action:{prune_worktree:{path:entry.path}},
          expected_head:status.head,expected_branch:status.branch,expected_index:status.index_revision});
      }
      const result = await this.api.completeRequest(owner.request);
      if (result.Err) { owner.uncertain = result.Err.code === 'outcome_unknown'; this.fail(owner,'worktree_prune_failed'); return; }
      this.invalidate(owner);
      this.api.forgetRequest(owner.request); owner.request = null;
      this.api.toast({id:`${owner.token}-complete`,message:this.text('worktree_pruned'),kind:'info'});
      if (this.dialog === owner) await this.open();
    } catch (_) { owner.uncertain = !!owner.request; this.fail(owner,'worktree_prune_failed'); }
    finally { this.settle(owner); }
  }
  retry() {
    const pending = this.operation;
    if (pending && this.dialog === pending.owner) return this.register(pending.owner,pending.action);
  }
  dispose() { this.disposed = true; }
}

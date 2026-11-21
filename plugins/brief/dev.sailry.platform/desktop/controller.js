export class Controller {
  constructor(api,resource,changed,report) {
    this.api = api;this.resource = resource;this.changed = changed;this.report = report;
    this.artifact = null;this.items = [];this.before = null;this.revision = resource?.revision ?? null;
    this.follow = resource?.revision === undefined;this.connected = true;this.loading = false;
    this.exporting = false;this.closed = false;this.again = false;this.failed = false;this.generation = 0;
  }
  async refresh() {
    if (this.closed || !this.connected || !this.resource) return;
    if (this.loading) {this.again = true;return;}
    await this.load(this.follow ? undefined : this.revision,this.follow);
  }
  async load(target,follow) {
    const generation = ++this.generation;
    this.loading = true;this.changed();
    try {
      const [content,page] = await Promise.all([
        this.api.readArtifact(this.resource.id,target),this.api.listArtifactRevisions(this.resource.id),
      ]);
      if (this.closed || generation !== this.generation) return;
      this.artifact = content.artifact;this.revision = content.artifact.revision;this.follow = follow;
      this.merge(page);this.failed = false;
    } catch (_) {
      if (!this.closed && generation === this.generation) {
        this.revision = this.artifact?.revision ?? this.revision;
        this.failed = true;this.report('brief_load_failed');
      }
    } finally {
      if (!this.closed && generation === this.generation) {
        this.loading = false;this.changed();
        const again = this.again;this.again = false;
        if (again) await this.refresh();
      }
    }
  }
  async select(value) {
    const revision = Number(value);
    if (this.closed || !this.connected || !Number.isInteger(revision) || revision <= 0
      || (revision === this.revision && !this.follow) || !this.items.some(item => item.revision === revision)) return;
    this.again = false;
    await this.load(revision,false);
  }
  async freeze(revision) {
    if (this.closed || !Number.isInteger(revision) || revision <= 0) return;
    const interrupted = this.loading;
    this.generation++;this.loading = false;this.again = false;
    this.follow = false;this.revision = revision;this.changed();
    if (interrupted || this.artifact?.revision !== revision) await this.load(revision,false);
  }
  async more() {
    if (this.closed || !this.connected || this.before === null) return;
    try {
      const page = await this.api.listArtifactRevisions(this.resource.id,this.before);
      if (!this.closed) {this.merge(page);this.changed();}
    } catch (_) {if (!this.closed) this.report('brief_load_failed');}
  }
  merge(page) {
    this.items = [...new Map([...this.items,...page.items].map(item => [item.revision,item])).values()]
      .sort((left,right) => right.revision-left.revision);
    this.before = page.next_before;
  }
  versions() {
    const items = this.items.map(item => ({id:String(item.revision),label:`v${item.revision}`}));
    if (this.artifact && !this.items.some(item => item.revision === this.artifact.revision))
      items.unshift({id:String(this.artifact.revision),label:`v${this.artifact.revision}`});
    return items;
  }
  async export(format) {
    if (this.closed || this.exporting || !this.connected || !this.artifact || !['html','png'].includes(format)) return;
    if (this.artifact.kind === 'preview' && format !== 'html') {this.report('brief_export_unsupported');return;}
    this.exporting = true;this.changed();
    try {await this.api.exportArtifact({artifact:this.artifact.id,revision:this.artifact.revision,format});}
    catch (_) {if (!this.closed) this.report('brief_export_failed');}
    finally {if (!this.closed) {this.exporting = false;this.changed();}}
  }
  stop() {this.closed = true;this.generation++;}
}

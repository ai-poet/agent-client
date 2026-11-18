import {browseSshDirectory} from 'sailry/connections';
import {createText,setText,releaseText} from 'sailry/forms';
export const join=(directory,name)=>`${directory.replace(/\/$/,'')}/${name}`;
export const parent=path=>path==='/'?'/':path.includes('/')?path.slice(0,path.lastIndexOf('/'))||'/':'.';
export const name=path=>path.split('/').at(-1);
const marker=(path,kind)=>`ssh-files-placeholder:${JSON.stringify({path,kind})}`;
export class Files {
  constructor(profile,report=()=>{},text={}) {this.profile=profile;this.report=report;this.path='.';this.input=createText('.',{label:text.ssh_remote_path,placeholder:text.ssh_remote_path});this.editing=false;this.entries=[];this.next=null;this.pending=false;this.error=null;this.selected=[];this.current=null;this.anchor=null;this.clipboard=null;this.revision=0;this.generation=0;this.closed=false;this.directories=new Map();this.expanded=new Set();}
  close(){this.closed=true;releaseText(this.input);}
  segments(){const absolute=this.path.startsWith('/'),parts=this.path.split('/').filter(Boolean),segments=absolute?[{label:'/',path:'/'}]:[];let path=absolute?'/':'';
    for(const label of parts){path=path?join(path,label):label;segments.push({label,path});}return segments;}
  placeholder(id){return id?.startsWith('ssh-files-placeholder:')?JSON.parse(id.slice('ssh-files-placeholder:'.length)):null;}
  async load(path,cx,append=false) {
    if(this.closed||this.pending||!path.trim())return;this.pending=true;this.error=null;const generation=append?this.generation:++this.generation;cx.notify();let loaded=false;
    try {const value=await browseSshDirectory({profile:this.profile.id,expected_revision:this.profile.revision,path,after:append?this.next:null});
      if(this.closed||generation!==this.generation)return;if(value.kind==='host_key_required'){this.error=value.changed?'ssh_key_changed':'ssh_key_unknown';this.report({},this.error);return;}
      if(value.kind!=='directory')throw new Error('invalid directory response');
      if(this.path!==value.path){this.selected=[];this.current=null;this.anchor=null;this.expanded.clear();}
      if(!append)this.directories.clear();this.path=value.path;setText(this.input,this.path);this.editing=false;this.entries=append?[...this.entries,...value.entries]:value.entries;this.next=value.next;this.revision++;loaded=true;
    } catch(error){if(!this.closed){this.error='ssh_files_failed';this.report(error,this.error);}}finally{this.pending=false;cx.notify();}
    if(loaded&&!append)await this.loadExpanded(this.path,cx);
  }
  async loadBranch(path,cx,append=false){if(path===this.path)return this.load(path,cx,append);if(this.closed||this.pending)return;
    let directory=this.directories.get(path);if(!directory){directory={entries:[],next:null,pending:false,error:null,loaded:false};this.directories.set(path,directory);}if(directory.pending)return;
    const generation=this.generation;directory.pending=true;directory.error=null;cx.notify();
    try{const value=await browseSshDirectory({profile:this.profile.id,expected_revision:this.profile.revision,path,after:append?directory.next:null});
      if(this.closed||generation!==this.generation)return;if(value.kind==='host_key_required'){directory.error=value.changed?'ssh_key_changed':'ssh_key_unknown';this.report({},directory.error);return;}
      if(value.kind!=='directory')throw new Error('invalid directory response');directory.entries=append?[...directory.entries,...value.entries]:value.entries;directory.next=value.next;directory.loaded=true;this.revision++;
    }catch(error){if(!this.closed&&generation===this.generation){directory.error='ssh_files_failed';this.report(error,directory.error);}}finally{directory.pending=false;cx.notify();}
  }
  async loadExpanded(path,cx){const entries=path===this.path?this.entries:this.directories.get(path)?.entries??[];
    await Promise.all(entries.filter(entry=>entry.kind==='directory'&&this.expanded.has(join(path,entry.name))).map(async entry=>{const child=join(path,entry.name);await this.loadBranch(child,cx);if(!this.closed&&!this.pending)await this.loadExpanded(child,cx);}));}
  async expand(path,cx){if(this.entry(path)?.kind!=='directory')return;this.expanded.add(path);if(!this.directories.get(path)?.loaded)await this.loadBranch(path,cx);cx.notify();}
  entry(path){const directory=parent(path),entries=directory===this.path?this.entries:this.directories.get(directory)?.entries??[];return entries.find(entry=>join(directory,entry.name)===path);}
  visible(){const paths=[],visit=(directory,entries)=>{for(const entry of entries){const path=join(directory,entry.name);paths.push(path);if(this.expanded.has(path))visit(path,this.directories.get(path)?.entries??[]);}};visit(this.path,this.entries);return paths;}
  select(path,event) {
    if(event.shift&&this.anchor){const paths=this.visible(),start=paths.indexOf(this.anchor),end=paths.indexOf(path);if(start>=0&&end>=0)this.selected=[...new Set([...(event.additive?this.selected:[]),...paths.slice(Math.min(start,end),Math.max(start,end)+1)])];}
    else if(event.additive){this.selected=this.selected.includes(path)?this.selected.filter(value=>value!==path):[...this.selected,path];this.anchor=path;}
    else {this.selected=[path];this.anchor=path;}this.current=path;
  }
  items(menu,text={}){const rows=(path,directory)=>{const entries=directory.entries,items=entries.map(entry=>{const id=join(path,entry.name),item={id,label:entry.name,icon:entry.kind==='directory'?'folder':'file',menu:menu(id)};
      if(entry.kind==='directory'){item.folder=true;item.expanded=this.expanded.has(id);item.children=rows(id,this.directories.get(id)??{entries:[],pending:true});}return item;});
      // The listing's own load shows in the panel's header; a branch shows
      // its load only while it has no rows yet.
      if(directory.pending){if(directory!==this&&!entries.length)items.push({id:marker(path,'loading'),label:text.ssh_files_loading??'',icon:'loader-circle',disabled:true});}
      else if(directory.error)items.push({id:marker(path,'retry'),label:text[directory.error]??text.refresh,icon:'rotate-cw'});
      if(directory.next&&!directory.pending)items.push({id:marker(path,'more'),label:text.files_load_more,icon:'chevron-down'});return items;};
    return rows(this.path,this);}
}

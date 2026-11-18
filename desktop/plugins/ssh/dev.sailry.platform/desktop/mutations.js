import {forgetRequest} from 'sailry/sdk';
import {Request} from './requests.js';

function normalize(path) {
  const parts=[];
  for(const part of path.split('/')) {
    if(!part||part==='.')continue;
    if(part==='..'&&parts.length&&parts.at(-1)!=='..')parts.pop();
    else if(part!=='..'||!path.startsWith('/'))parts.push(part);
  }
  return `${path.startsWith('/')?'/':''}${parts.join('/')}`||'.';
}

function access(command) {
  const {path,action}=command.data;
  return action.operation==='copy'
    ?{read:[normalize(path)],write:[normalize(action.destination)]}
    :{read:[],write:[path,...(action.operation==='move'?[action.destination]:[])].map(normalize)};
}

const overlaps=(left,right)=>left===right||left==='/'||right==='/'
  ||left==='.'&&!right.startsWith('/')||right==='.'&&!left.startsWith('/')
  ||left.startsWith(`${right}/`)||right.startsWith(`${left}/`);
const conflicts=(left,right)=>left.write.some(path=>[...right.read,...right.write].some(other=>overlaps(path,other)))
  ||right.write.some(path=>left.read.some(other=>overlaps(path,other)));
const changed=request=>request.output?.kind==='ssh_outcome'&&request.output.data?.kind==='files_changed';

// Independent paths share no completion gate. Related paths retain their input order.
export class Mutations {
  constructor(commands,current) {
    this.current=current;this.requests=[];this.active=false;this.started=false;
    try {for(const command of commands)this.requests.push(new Request(command));}
    catch(error) {for(const request of this.requests)forgetRequest(request.id);throw error;}
    const paths=commands.map(access);
    this.dependencies=paths.map((path,index)=>paths.slice(0,index).flatMap((prior,other)=>conflicts(prior,path)?[other]:[]));
  }
  async start(cx) {
    if(this.started)return;this.started=true;this.active=true;cx.notify();
    const jobs=[];
    for(const [index,request]of this.requests.entries())jobs.push((async()=>{
      const dependencies=this.dependencies[index];
      if(dependencies.length)await Promise.all(dependencies.map(other=>jobs[other]));
      if(!this.current()||dependencies.some(other=>!changed(this.requests[other]))) {
        request.accept({Err:{code:'cancelled',message:'File operation was not admitted'}});return;
      }
      await request.start(cx);
    })());
    try {await Promise.allSettled(jobs);}
    finally {this.active=false;cx.notify();}
  }
  async check(cx) {
    if(this.running)return;this.active=true;cx.notify();
    try {await Promise.allSettled(this.requests.filter(request=>request.unknown).map(request=>request.check(cx)));}
    finally {this.active=false;cx.notify();}
  }
  async cancel() {await Promise.allSettled(this.requests.filter(request=>!request.done).map(request=>request.cancel()));}
  get running() {return this.active||this.requests.some(request=>request.running);}
  get unknown() {return this.requests.some(request=>request.unknown);}
  get uncertainty() {return this.requests.find(request=>request.unknown)?.uncertainty;}
  get done() {return this.requests.every(request=>request.done);}
  get error() {return this.done?this.requests.find(request=>request.error)?.error??null:null;}
  get output() {
    if(!this.done||this.error)return null;
    return this.requests.find(request=>!changed(request))?.output??{kind:'ssh_outcome',data:{kind:'files_changed'}};
  }
  release() {for(const request of this.requests)request.release();}
}

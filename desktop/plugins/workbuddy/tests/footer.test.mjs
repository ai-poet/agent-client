import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import * as credits from '../dev.sailry.platform/desktop/credits.js';
import {messages} from '../dev.sailry.platform/desktop/locales.js';

// These tests exercise package scheduling and SDK arguments, not native rendering.
const source=(await readFile(new URL('../dev.sailry.platform/desktop/footer.js',import.meta.url),'utf8'))
  .replace(/^import .*;\n/gm,'').replace(/^export default /gm,'').replace(/^export /gm,'');
const providerSource=(await readFile(new URL('../dev.sailry.platform/desktop/providers.js',import.meta.url),'utf8'))
  .replace(/^import .*;\n/gm,'').replace(/^export default /gm,'');
const visibilitySource=(await readFile(new URL('../dev.sailry.platform/desktop/visibility.js',import.meta.url),'utf8'))
  .replace(/^import .*;\n/gm,'').replace(/^export /gm,'');
const plain=value=>JSON.parse(JSON.stringify(value));
const deferred=()=>{
  let resolve,reject;
  const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});
  return {promise,resolve,reject};
};
const state=(cursor='1',overrides={})=>({cursor,connected:true,provider:{id:'channel',revision:'9007199254740993',model:'selected',
  preset:{source:{kind:'package',data:{name:'workbuddy'}},id:'china',route:null}},...overrides});
const response=(remaining=12)=>({status:200,headers:{},body:JSON.stringify({code:0,data:{Response:{Data:{Accounts:[
  {PackageName:'Actual package',CapacityRemain:remaining,CapacitySize:20},
]}}}})});
const settle=async()=>{for(let index=0;index<8;index++)await Promise.resolve();};

async function fixture(initial=state(),providers=null) {
  const requests=[],published=[],toasts=[],watches=[],sleeps=[],failures=[],registrations=[],visibility=[];
  let clock=1000;
  class Clock extends Date {static now(){return clock;}}
  const Controller=vm.runInNewContext(`${visibilitySource}\n${providerSource}\n${source}\nFooter`,{
    Date:Clock,
    nextChange:cursor=>{if(!cursor)return Promise.resolve({cursor:'1',visible:true,connected:true});const wait=deferred();visibility.push(wait);return wait.promise;},
    View:class {},div:()=>({}),context:()=>JSON.stringify({locale:'en',surface:providers ? 'provider' : 'composer'}),
    ...credits,messages,
    publishContributions:states=>published.push(plain(states)),
    readModelContext:()=>plain(initial),
    readProviderContexts:()=>plain(providers),
    nextProviderContexts:cursor=>{const wait=deferred();watches.push({cursor,...wait});return wait.promise;},
    nextModelContext:cursor=>{const wait=deferred();watches.push({cursor,...wait});return wait.promise;},
    requestProviderHttp:(...args)=>{const wait=deferred();requests.push({args:plain(args),...wait});return wait.promise;},
    prepareRequest:command=>{registrations.push(plain(command));return `registration-${registrations.length}`;},
    completeRequest:async()=>({Ok:{kind:'plugin_result',data:{registered:true}}}),forgetRequest(){},
    faultCode:message=>{try{return JSON.parse(message).code;}catch{return 'failed';}},
    toast:value=>toasts.push(plain(value)),
  });
  const cx={notify(){},sleep:ms=>{const wait=deferred();sleeps.push({ms,...wait,resolve:()=>{clock+=ms;wait.resolve();}});return wait.promise;},
    spawn:callback=>{const promise=callback(cx);promise.catch(error=>failures.push(error));return promise;}};
  const view=new Controller();view.init({},cx);await settle();
  return {view,cx,requests,published,toasts,watches,sleeps,failures,registrations,
    async update(value){watches.shift().resolve(plain(value));await settle();},
    advance:ms=>{clock+=ms;},
    async visible(value){visibility.shift().resolve({cursor:String(clock),visible:value,connected:true});await settle();},
    latest:()=>published.at(-1)[0]};
}

test('queries the selected channel and refreshes without overlapping requests',async()=>{
  const setup=await fixture();
  assert.equal(setup.requests.length,1);
  const [id,revision,request,input]=setup.requests[0].args;
  assert.deepEqual([id,revision,request],['channel','9007199254740993','credits']);
  assert.deepEqual(Object.keys(input).sort(),['begin','end']);
  assert.match(input.begin,/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/);
  assert.equal(setup.latest().visible,false);
  assert.equal(setup.latest().value,'—');
  setup.sleeps.shift().resolve();await settle();
  assert.equal(setup.requests.length,1);
  setup.requests[0].resolve(response());await settle();
  assert.equal(setup.latest().value,'12');
  assert.ok(setup.latest().details.some(row=>row.progress===60));
  assert.equal(setup.sleeps[0].ms,60000);
  setup.sleeps.shift().resolve();await settle();
  assert.equal(setup.requests.length,2);
  setup.requests[1].resolve(response(10));await settle();
  assert.equal(setup.latest().value,'10');
  assert.deepEqual(setup.toasts,[]);
  assert.deepEqual(setup.failures,[]);
});

test('registers China scheduling once for each channel revision',async()=>{
  const setup=await fixture();
  assert.equal(setup.registrations.length,1);
  assert.equal(setup.registrations[0].data.handler,'register');
  assert.equal(setup.registrations[0].data.input.provider.preset.id,'china');
  const changed=state('2');changed.provider.model='another';
  await setup.update(changed);assert.equal(setup.registrations.length,1);
  const revised=state('3');revised.provider.revision='9007199254740994';
  await setup.update(revised);assert.equal(setup.registrations.length,2);
  const international=state('4');international.provider.preset.id='workbuddy-global';
  await setup.update(international);assert.equal(setup.registrations.length,2);
});

test('hides unrelated selections and discards replies from older contexts',async()=>{
  const setup=await fixture();
  await setup.update(state('2',{provider:null}));
  assert.equal(setup.latest().visible,false);
  setup.requests[0].resolve(response(999));await settle();
  assert.equal(setup.latest().visible,false);
  assert.equal(setup.latest().value,'—');
  assert.equal(setup.requests.length,1);
  await setup.update(state('3'));
  assert.equal(setup.requests.length,2);
  const other=state('4');other.provider.model='another';other.provider.revision='9007199254740994';
  await setup.update(other);
  assert.equal(setup.latest().value,'—');
  assert.equal(setup.requests.length,3,'the new context starts before the old request settles');
  setup.requests[1].resolve(response(888));await settle();
  assert.equal(setup.requests.length,3);
  assert.equal(setup.requests[2].args[1],'9007199254740994');
  assert.equal(setup.latest().value,'—');
  setup.requests[2].resolve(response(7));await settle();
  assert.equal(setup.latest().value,'7');
  assert.deepEqual(setup.failures,[]);
});

test('new context completes before a stale request',async()=>{
  const setup=await fixture(),changed=state('2');changed.provider.id='new';
  await setup.update(changed);assert.equal(setup.requests.length,2);
  setup.requests[1].resolve(response(8));await settle();assert.equal(setup.latest().value,'8');
  setup.requests[0].resolve(response(999));await settle();assert.equal(setup.latest().value,'8');
  assert.equal(setup.requests.length,2);assert.deepEqual(setup.failures,[]);
});

test('clears retained values on disconnection and a new context generation',async()=>{
  const setup=await fixture();setup.requests[0].resolve(response());await settle();
  await setup.update(state('2',{connected:false,provider:null}));
  assert.equal(setup.latest().visible,false);
  assert.equal(setup.latest().value,'—');
  await setup.update(state('3'));
  assert.equal(setup.latest().visible,false);
  assert.equal(setup.latest().value,'—');
  setup.requests[1].resolve(response(8));await settle();
  await setup.update(state('4'));
  assert.equal(setup.latest().value,'—');
  assert.equal(setup.requests.length,3);
  assert.deepEqual(setup.failures,[]);
});

test('does not request another package or a disconnected channel',async()=>{
  for(const initial of [state('1',{provider:null}),state('1',{connected:false})]) {
    const setup=await fixture(initial);
    assert.deepEqual(setup.requests,[]);
    assert.equal(setup.latest().visible,false);
    assert.deepEqual(setup.failures,[]);
  }
});

test('silently hides failures and recovers after successful refresh',async()=>{
  const setup=await fixture();
  const failure=new Error(JSON.stringify({code:'permission_denied',message:'Synthetic private diagnostic'}));
  setup.requests[0].reject(failure);await settle();
  assert.deepEqual(setup.toasts,[]);
  setup.advance(60000);setup.view.refresh(setup.cx);setup.requests[1].reject(failure);await settle();
  assert.deepEqual(setup.toasts,[]);
  setup.advance(60000);setup.view.refresh(setup.cx);setup.requests[2].resolve(response());await settle();
  assert.equal(setup.latest().value,'12');
  setup.advance(60000);setup.view.refresh(setup.cx);setup.requests[3].reject(failure);await settle();
  assert.deepEqual(setup.toasts,[]);
  assert.equal(setup.latest().value,'—');
  assert.equal(setup.latest().visible,false);
  assert.deepEqual(setup.failures,[]);
});

test('silently hides rejected credit queries',async()=>{
  const setup=await fixture();
  setup.requests[0].resolve({status:429,headers:{},body:JSON.stringify({code:14018,message:'Actual account is unavailable'})});
  await settle();
  assert.deepEqual(setup.toasts,[]);
  assert.equal(setup.latest().value,'—');
  assert.equal(setup.latest().visible,false);
  assert.deepEqual(setup.failures,[]);
});


test('silently hides invalid credit responses',async()=>{
  const setup=await fixture();
  setup.requests[0].resolve({status:503,headers:{},body:'upstream transport unavailable'});await settle();
  assert.deepEqual(setup.toasts,[]);
  assert.equal(setup.latest().value,'—');
  assert.equal(setup.latest().visible,false);
  assert.deepEqual(setup.failures,[]);
});

test('does not toast failures for a replaced selection',async()=>{
  const setup=await fixture();await setup.update(state('2',{provider:null}));
  setup.requests[0].reject(new Error('Late SDK diagnostic'));await settle();
  assert.deepEqual(setup.toasts,[]);
  assert.equal(setup.latest().visible,false);
  assert.deepEqual(setup.failures,[]);
});

test('keeps successful zero distinct from unavailable credits',async()=>{
  for(const preset of ['china','workbuddy-global','codebuddy-global']) {
    const initial=state();initial.provider.preset.id=preset;
    const setup=await fixture(initial);
    const reply=Accounts=>({status:200,body:JSON.stringify({code:0,data:{Response:{Data:{Accounts}}}})});
    setup.requests[0].resolve(reply(null));await settle();
    assert.equal(setup.latest().visible,true);assert.equal(setup.latest().value,'0');
    setup.advance(60000);setup.view.refresh(setup.cx);setup.requests[1].resolve(reply([]));await settle();
    assert.equal(setup.latest().visible,false);
    setup.advance(60000);setup.view.refresh(setup.cx);setup.requests[2].resolve(response(0));await settle();
    assert.equal(setup.latest().visible,true);assert.equal(setup.latest().value,'0');
    setup.advance(60000);setup.view.refresh(setup.cx);setup.requests[3].reject(new Error('Private SDK diagnostic'));await settle();
    assert.equal(setup.latest().visible,false);assert.deepEqual(setup.latest().details,[]);
    assert.deepEqual(setup.toasts,[]);assert.deepEqual(setup.failures,[]);
  }
});

const provider=(id,revision='1',credential='account-a',credentialRevision='1')=>({...state().provider,id,revision,
  scope:{provider:id,revision:`${revision}:${credentialRevision}`,credential:{node:'execution-node',id:credential}},
});
const batch=(providers,cursor='1')=>({cursor,connected:true,providers});
const current=(setup,id)=>setup.published.flat().findLast(entry=>entry.scope?.provider===id);

test('shares one provider page loop without registering jobs',async()=>{
  const setup=await fixture(state(),batch([provider('china'),provider('global')]));
  assert.equal(setup.requests.length,2,'all providers start before either reply settles');
  assert.equal(setup.requests[1].args[0],'global');
  setup.requests[0].resolve(response(12));await settle();
  assert.equal(setup.requests[1].args[0],'global');
  setup.requests[1].resolve(response(0));await settle();
  assert.equal(current(setup,'china').value,'12');
  assert.equal(current(setup,'global').value,'0');
  assert.deepEqual(current(setup,'china').details,[]);
  assert.equal(setup.sleeps.length,1);assert.equal(setup.watches.length,1);
  assert.deepEqual(setup.registrations,[]);assert.deepEqual(setup.toasts,[]);
  assert.deepEqual(setup.failures,[]);
});

test('isolates late balances after account replacement',async()=>{
  const setup=await fixture(state(),batch([provider('same')]));
  const replacement=provider('same','2','account-b');
  await setup.update(batch([replacement],'2'));
  assert.equal(setup.requests.length,2,'the replacement does not wait for the old account');
  setup.requests[0].resolve(response(999));await settle();
  assert.equal(current(setup,'same').visible,false);
  assert.deepEqual(current(setup,'same').scope,replacement.scope);
  assert.equal(setup.requests.length,2);
  setup.requests[1].resolve(response(8));await settle();
  assert.equal(current(setup,'same').value,'8');
  await setup.update(batch([replacement],'3'));
  assert.equal(setup.requests.length,2);
  assert.deepEqual(setup.registrations,[]);assert.deepEqual(setup.failures,[]);
});

test('new provider rows bypass unrelated pending requests',async()=>{
  const first=provider('slow'),second=provider('new');
  const setup=await fixture(state(),batch([first]));
  await setup.update(batch([first,second],'2'));
  assert.equal(setup.requests.length,2);assert.equal(setup.requests[1].args[0],'new');
  setup.requests[1].resolve(response(8));await settle();
  assert.equal(current(setup,'new').value,'8');assert.equal(current(setup,'slow').visible,false);
  setup.requests[0].resolve(response(12));await settle();assert.equal(current(setup,'slow').value,'12');
  assert.deepEqual(setup.failures,[]);
});

test('discards late balances after same-credential renewal',async()=>{
  const original=provider('same');
  const setup=await fixture(state(),batch([original]));
  const renewed=provider('same','1','account-a','2');
  assert.deepEqual(renewed.scope.credential,original.scope.credential);
  assert.equal(renewed.revision,original.revision);
  await setup.update(batch([renewed],'2'));
  setup.requests[0].resolve(response(999));await settle();
  assert.equal(current(setup,'same').visible,false);
  assert.equal(current(setup,'same').scope.revision,'1:2');
  assert.equal(setup.requests[1].args[1],'1');
  setup.requests[1].resolve(response(8));await settle();
  assert.equal(current(setup,'same').value,'8');
  assert.deepEqual(setup.failures,[]);
});

test('pauses hidden refreshes and preserves fresh results',async()=>{
  const setup=await fixture();
  setup.requests[0].resolve(response());await settle();
  await setup.visible(false);
  setup.sleeps.shift().resolve();await settle();
  assert.equal(setup.sleeps.length,0);
  assert.equal(setup.requests.length,1);
  await setup.visible(true);
  assert.equal(setup.requests.length,2);
  setup.requests[1].resolve(response());await settle();
  await setup.visible(false);
  setup.advance(60000);
  setup.sleeps.shift().resolve();await settle();
  const count=setup.requests.length;
  await setup.visible(true);
  assert.equal(setup.requests.length,count+1);
  assert.deepEqual(setup.failures,[]);
});

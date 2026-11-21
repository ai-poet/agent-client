import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';
import {day,status,claimed} from '../dev.sailry.platform/host/protocol.js';
import {ResponseError} from '../dev.sailry.platform/errors.js';
import {messages} from '../dev.sailry.platform/desktop/locales.js';

const root = new URL('../',import.meta.url);
const source = (await readFile(new URL('dev.sailry.platform/host/checkin.js',root),'utf8'))
  .replace(/^import .*;\n/gm,'').replace(/^export /gm,'');
const schedules = (await readFile(new URL('dev.sailry.platform/host/schedules.js',root),'utf8'))
  .replace(/^import .*;\n/gm,'').replace(/^export /gm,'');
const plain = value=>JSON.parse(JSON.stringify(value));
const provider = (id='00000000-0000-4000-8000-000000000001',preset='china')=>({id,revision:'2',
  preset:{id:preset,source:{kind:'package',data:{name:'workbuddy'}}}});
const response = (data,status=200)=>({status,headers:{},body:JSON.stringify(data)});
const current = (active=true,checked=false)=>response({code:0,data:{active,today_checked_in:checked}});
const channel = p=>p?.preset?.id==='china' && p.preset.source?.kind==='package' && p.preset.source.data?.name==='workbuddy';
const fault = (code,message=code)=>Object.assign(new Error(message),{code});
const deferred = ()=>{let resolve;const promise=new Promise(yes=>{resolve=yes;});return {promise,resolve};};

function fixture(options={}) {
  const entries=options.entries??new Map(),requests=[],claims=[],notifications=[],forgotten=[],drafts=new Map();
  let now=Date.parse('2026-10-20T00:00:00Z'),writes=0;
  const read=async key=>plain(entries.get(key)??{present:false,revision:'0',value:null});
  const write=async(key,value,revision)=>{
    writes++;
    if (options.writeFailure?.(writes)) throw fault('unavailable');
    const previous=entries.get(key)??{revision:'0'};
    if (previous.revision!==revision) throw fault('revision_conflict');
    const entry={present:true,revision:String(BigInt(revision)+1n),value:plain(value)};
    entries.set(key,entry);return plain(entry);
  };
  const environment={Map,day:()=>day(now),status,claimed,ResponseError,channel,read,write,messages,
    readSettings:async()=>({values:options.settings ?? {checkin_enabled:true}}),
    requestProviderHttp:async(...args)=>{requests.push(plain(args));return options.status?options.status():current();},
    prepareProviderHttp:(...args)=>{const id=`request-${drafts.size}`;drafts.set(id,args);return id;},
    forgetRequest:id=>forgotten.push(id),
    complete:async id=>{claims.push(plain(drafts.get(id)));if(options.claim) return options.claim();return response({code:0,data:{credit:100}});},
    notify:async(...args)=>{notifications.push(plain(args));if(options.notifyFailure)throw fault('unavailable');},
    report:async(key,value,revision,...notice)=>{
      if(options.notifyFailure)throw fault('unavailable');
      const saved=await write(key,value,revision);notifications.push(plain(notice));return saved;
    },
  };
  const controller=()=>vm.runInNewContext(`${source}\n({checkin})`,{...environment});
  return {...controller(),controller,requests,claims,notifications,entries,forgotten,
    advance:ms=>{now+=ms;},value:()=>entries.get(`checkin/day/${provider().id}`)?.value};
}

test('uses the China calendar without a second timer',()=>{
  assert.equal(day(Date.parse('2026-10-19T15:59:59Z')),'2026-10-19');
  assert.equal(day(Date.parse('2026-10-19T16:00:00Z')),'2026-10-20');
});

test('decodes only declared status and claim outcomes',()=>{
  assert.deepEqual(status(current()),{active:true,checked:false});
  assert.deepEqual(claimed(response({code:10001},400)),{status:'already',credit:null});
  assert.deepEqual(claimed(response({code:1001},400)),{status:'already',credit:null});
  assert.deepEqual(claimed(response({code:'1001'})),{status:'already',credit:null});
  assert.deepEqual(claimed(response({code:1003},400)),{status:'closed',credit:null});
  assert.throws(()=>claimed(response({code:1002},400)),ResponseError);
  assert.deepEqual(claimed(response({code:0,data:{credit:'12.25'}})),{status:'claimed',credit:12.25});
  assert.deepEqual(claimed(response({code:0,data:{credit:0}})),{status:'claimed',credit:0});
  assert.deepEqual(claimed(response({code:0,data:{}})),{status:'claimed',credit:null});
  assert.throws(()=>status(response({code:0,data:{active:true}})),/status/);
  assert.throws(()=>status(response({code:10085,msg:'请求不合法，如有疑问请联系客服'},403)),
    error=>error instanceof ResponseError && error.message==='请求不合法，如有疑问请联系客服');
});

test('skips international and unrelated provider selections',async()=>{
  const setup=fixture();
  for(const preset of ['workbuddy-global','codebuddy-global']) assert.equal((await setup.checkin({provider:provider(undefined,preset)})).status,'unsupported');
  const other=provider();other.preset.source.data.name='other';
  assert.equal((await setup.checkin({provider:other})).status,'unsupported');
  assert.equal(setup.requests.length,0);assert.equal(setup.claims.length,0);assert.equal(setup.entries.size,0);
});

for(const [name,active,checked,result] of [['closed',false,false,'closed'],['already claimed',true,true,'already']]) {
  test(`keeps ${name} activities from mutating`,async()=>{
    const setup=fixture({status:()=>current(active,checked)});
    assert.equal((await setup.checkin({provider:provider()})).status,result);
    assert.equal((await setup.checkin({provider:provider()})).skipped,true);
    assert.equal(setup.requests.length,1);assert.equal(setup.claims.length,0);assert.deepEqual(setup.notifications,[]);
  });
}

test('claims once and preserves server-reported credit amounts',async()=>{
  const setup=fixture();const result=await setup.checkin({provider:provider(),locale:'zh-CN'});
  assert.deepEqual(plain(result),{status:'claimed',credit:100});
  assert.deepEqual(setup.requests,[[provider().id,'2','checkin_status',{}]]);
  assert.deepEqual(setup.claims,[[provider().id,'2','checkin_claim',{}]]);
  assert.equal(setup.value().status,'claimed');
  assert.deepEqual(setup.notifications,[['账户积分','每日积分已领取 100','success']]);
  await setup.checkin({provider:provider()});assert.equal(setup.claims.length,1);
});

test('accepts an upstream duplicate without another claim',async()=>{
  const setup=fixture({claim:()=>response({code:10001,msg:'Already checked in'},400)});
  assert.equal((await setup.checkin({provider:provider()})).status,'already');
  assert.equal((await setup.checkin({provider:provider()})).skipped,true);
  assert.equal(setup.claims.length,1);assert.deepEqual(setup.notifications,[]);
});

test('retains original business failures without retrying mutations',async()=>{
  const setup=fixture({claim:()=>response({code:10085,msg:'请求不合法，如有疑问请联系客服'},403)});
  const result=await setup.checkin({provider:provider()});
  assert.equal(result.status,'failed');assert.equal(result.error.message,'请求不合法，如有疑问请联系客服');
  await setup.checkin({provider:provider()});assert.equal(setup.claims.length,1);assert.equal(setup.notifications.length,1);
});

test('retains uncertain claims across callback restarts',async()=>{
  const setup=fixture({claim:()=>{throw fault('outcome_unknown');}});
  assert.equal((await setup.checkin({provider:provider()})).status,'unknown');
  assert.equal((await setup.controller().checkin({provider:provider()})).skipped,true);
  assert.equal(setup.claims.length,1);assert.equal(setup.requests.length,2);
});

test('reconciles uncertain claims without resubmitting them',async()=>{
  let checked=false;
  const setup=fixture({status:()=>current(true,checked),claim:()=>{throw fault('outcome_unknown');}});
  await setup.checkin({provider:provider()});checked=true;
  assert.equal((await setup.controller().checkin({provider:provider()})).reconciled,true);
  assert.equal(setup.value().status,'already');assert.equal(setup.claims.length,1);
});

test('allows a later account revision after a pre-claim failure',async()=>{
  let ready=false;
  const setup=fixture({status:()=>{if(!ready)throw fault('not_configured');return current();}});
  assert.equal((await setup.checkin({provider:provider()})).status,'failed');
  assert.equal(setup.value().submitted,false);
  assert.equal((await setup.checkin({provider:provider()})).status,'failed');
  assert.equal(setup.notifications.length,1);ready=true;
  assert.equal((await setup.checkin({provider:{...provider(),revision:'3'}})).status,'claimed');
  assert.equal(setup.claims.length,1);
});

test('checks again on a new China date',async()=>{
  const setup=fixture({status:()=>current(false,false)});
  await setup.checkin({provider:provider()});setup.advance(86400000);
  await setup.checkin({provider:provider()});assert.equal(setup.requests.length,2);assert.equal(setup.claims.length,0);
});

test('coalesces concurrent callbacks for one provider',async()=>{
  const wait=deferred(),setup=fixture({status:()=>wait.promise});
  const first=setup.checkin({provider:provider()}),second=setup.checkin({provider:provider()});
  await Promise.resolve();wait.resolve(current());
  await Promise.all([first,second]);assert.equal(setup.requests.length,1);assert.equal(setup.claims.length,1);
});

test('fences concurrent callback instances before admission',async()=>{
  const wait=deferred(),setup=fixture({status:()=>wait.promise});
  const first=setup.checkin({provider:provider()}),second=setup.controller().checkin({provider:provider()});
  await Promise.resolve();wait.resolve(current());
  const results=await Promise.all([first,second]);
  assert.equal(setup.claims.length,1);assert.ok(results.some(result=>result.status==='conflict'));
});

test('does not submit when the daily fence fails',async()=>{
  const setup=fixture({writeFailure:()=>true});
  assert.equal((await setup.checkin({provider:provider()})).status,'failed');
  assert.equal(setup.claims.length,0);assert.equal(setup.forgotten.length,1);
});

test('keeps confirmed claims after observation or notification failures',async()=>{
  const setup=fixture({writeFailure:number=>number===2,notifyFailure:true});
  assert.equal((await setup.checkin({provider:provider()})).status,'claimed');
  assert.equal(setup.value().status,'pending');
  await setup.controller().checkin({provider:provider()});assert.equal(setup.claims.length,1);
});

test('registers an atomic native callback and daily schedule',async()=>{
  const calls=[],scheduleId='00000000-0000-4000-8000-000000000010';
  const register=vm.runInNewContext(`${schedules}\nregister`,{channel,
    readSettings:async()=>({values:{checkin_enabled:true}}),read:async()=>({present:false,revision:'0'}),newId:()=>scheduleId,context:()=>({package:{name:'workbuddy'}}),
    prepareRequest:command=>{calls.push(plain(command));return command;},
    prepareTransaction:operations=>{calls.push(plain(operations));return operations;},
    complete:async command=>Array.isArray(command)?[]:{data:[]},
  });
  assert.equal((await register({provider:provider(),locale:'en'})).registered,true);
  const operations=calls.at(-1);
  assert.deepEqual(operations.map(item=>item.kind),['write','dispatch','dispatch','dispatch','dispatch']);
  assert.equal(operations[3].data.data.source.topic,'settings.changed');
  assert.equal(operations[3].data.data.callback.command.data.handler,'reconcile');
  const schedule=operations[1].data.data,handler=operations[2].data.data;
  assert.deepEqual(schedule.timing,{kind:'weekly',data:{timezone:'Asia/Shanghai',weekdays:[1,2,3,4,5,6,7],hour:9,minute:5}});
  assert.equal(handler.callback.completion,'command');assert.equal(handler.callback.command.kind,'call_plugin');
  assert.equal(handler.callback.command.data.handler,'checkin');assert.equal(handler.queue,scheduleId);
  assert.equal(operations[4].data.kind,'publish');assert.equal(operations[4].data.data.topic,scheduleId);
});

test('keeps existing registration without adding another schedule',async()=>{
  const register=vm.runInNewContext(`${schedules}\nregister`,{channel,
    readSettings:async()=>({values:{checkin_time:'09:05'}}),context:()=>({package:{settings_revision:0}}),
    read:async()=>({present:true,revision:'1',value:{id:'existing',provider:provider(),time:'09:05',settings_revision:'0'}}),
    prepareRequest:command=>{assert.equal(command.data.action.kind,'list_handlers');return command;},
    complete:async()=>({data:[{name:'checkin.settings',revision:1}]}),
  });
  assert.equal((await register({provider:provider()})).registered,true);
  assert.equal((await register({provider:provider(undefined,'workbuddy-global')})).registered,false);
});


test('updates the existing schedule after settings change',async()=>{
  const calls=[];
  const register=vm.runInNewContext(`${schedules}\nregister`,{channel,
    readSettings:async()=>({values:{checkin_enabled:true,checkin_time:'23:59'}}),context:()=>({package:{name:'workbuddy',settings_revision:4}}),
    read:async()=>({present:true,revision:'3',value:{id:'existing',provider:provider(),time:'09:05',settings_revision:'3'}}),
    newId:()=>{throw new Error('Schedule identity must be preserved');},
    prepareRequest:command=>command,prepareTransaction:operations=>{calls.push(plain(operations));return operations;},
    complete:async command=>Array.isArray(command)?[]:{data:command.data.action.kind==='list_schedules'
      ?[{id:'existing',revision:5}]:[{name:'existing',revision:7},{name:'existing.settings',revision:1}]},
  });
  assert.equal((await register({provider:provider()})).registered,true);
  const [record,schedule,handler,listener]=calls[0];
  assert.equal(listener.data.data.name,'existing.settings');
  assert.equal(listener.data.data.source.topic,'settings.changed');
  assert.equal(record.data.value.time,'23:59');assert.equal(record.data.value.settings_revision,'4');
  assert.equal(schedule.data.data.id,'existing');assert.equal(schedule.data.data.revision,'5');
  assert.deepEqual(schedule.data.data.timing.data,{timezone:'Asia/Shanghai',weekdays:[1,2,3,4,5,6,7],hour:23,minute:59});
  assert.equal(handler.data.data.name,'existing');assert.equal(handler.data.data.revision,'7');
  assert.equal(calls[0].some(item=>item.data?.kind==='publish'),false);
  const setup=fixture();await setup.checkin({provider:provider()});
  assert.equal((await setup.checkin(schedule.data.data.payload)).skipped,true);
  assert.equal(setup.requests.length,1);assert.equal(setup.claims.length,1);
});

test('rejects malformed times before touching registration',async()=>{
  for(const time of ['24:00','09:60','9:05','09:5',' 9:05','09:05\n',null,905]) {
    const register=vm.runInNewContext(`${schedules}\nregister`,{channel,
      readSettings:async()=>({values:{checkin_time:time}}),
      read:()=>{throw new Error('Invalid settings must not alter registration');},
    });
    await assert.rejects(register({provider:provider()}),/Invalid daily check-in time/);
  }
});

test('declares a validated common time field',async()=>{
  const schema=JSON.parse(await readFile(new URL('dev.sailry.platform/settings.json',root),'utf8'));
  const field=schema.properties.checkin_time,pattern=new RegExp(field.pattern);
  assert.equal(field.default,'09:05');assert.equal(field['x-sailry-time'],true);
  assert.deepEqual(schema.required,['checkin_enabled','checkin_time']);
  assert.deepEqual(schema.properties.checkin_enabled,{type:'boolean',title:'Automatic check-in',default:false});
  for(const name of ['free_only_china','free_only_international','free_only_codebuddy']) {
    assert.equal(schema.properties[name].type,'boolean');assert.equal(schema.properties[name].default,true);
    for(const table of Object.values(schema['x-sailry-locales']))assert.equal(typeof table[name],'string');
  }
  for(const time of ['00:00','09:05','23:59'])assert.equal(pattern.test(time),true);
  for(const time of ['24:00','09:60','9:05','09:5'])assert.equal(pattern.test(time),false);
  assert.equal(field.maxLength,5);
  assert.equal(Object.keys(schema['x-sailry-locales']).length,10);
});


test('reconciles saved schedules without a selected channel',async()=>{
  const calls=[];
  const records={
    'checkin/schedule/first':{present:true,revision:'2',value:{id:'first',provider:provider(),locale:'en',time:'09:05',settings_revision:'1'}},
    'checkin/schedule/removed':{present:true,revision:'1',value:{id:'removed',provider:provider(),time:'09:05',settings_revision:'1'}},
    'checkin/schedule/second':{present:true,revision:'3',value:{id:'second',provider:provider('second'),locale:'zh-CN',time:'09:05',settings_revision:'1'}},
  };
  const reconcile=vm.runInNewContext(`${schedules}\nreconcile`,{channel,
    context:()=>({package:{name:'workbuddy',settings_revision:2}}),
    readSettings:async()=>({values:{checkin_enabled:true,checkin_time:'07:30'}}),read:async key=>records[key],
    prepareRequest:command=>command,prepareTransaction:operations=>{calls.push(plain(operations));return operations;},
    complete:async command=>Array.isArray(command)?[]:{data:[{id:'first',revision:4,enabled:true},{id:'second',revision:6,enabled:false}]},
  });
  for(const key of ['first','second']) assert.deepEqual(plain(await reconcile({key:`checkin/schedule/${key}`})),{updated:1});
  assert.deepEqual(plain(await reconcile({key:'checkin/schedule/removed'})),{updated:0});
  assert.deepEqual(calls.map(operations=>operations[1].data.data.id),['first','second']);
  assert.deepEqual(calls.map(operations=>operations[1].data.data.enabled),[true,false]);
  assert.ok(calls.every(operations=>operations.length===2 && operations[0].kind==='write'
    && operations[1].data.kind==='save_schedule' && operations[1].data.data.timing.data.hour===7
    && operations[1].data.data.timing.data.minute===30));
  assert.equal(calls.some(operations=>operations.some(item=>item.data?.kind==='publish')),false);
});


test('persists one notification for each daily result',async()=>{
  const success=fixture({claim:()=>response({code:0,data:{credit:12.25}})});
  await success.checkin({provider:provider()});await success.controller().checkin({provider:provider()});
  assert.deepEqual(success.notifications,[['Account credits','Daily credits received 12.25','success']]);
  const failure=fixture({status:()=>response({code:10085,msg:'Original upstream failure'},403)});
  await failure.checkin({provider:provider()});await failure.controller().checkin({provider:provider()});
  assert.deepEqual(failure.notifications,[['Account credits','Original upstream failure','error']]);assert.equal(failure.claims.length,0);
  const unknown=fixture({claim:()=>{throw fault('outcome_unknown');}});
  await unknown.checkin({provider:provider()});await unknown.controller().checkin({provider:provider()});
  assert.deepEqual(unknown.notifications,[['Account credits','Daily credit claim outcome unknown','warning']]);
  assert.equal(unknown.claims.length,1);
});


test('rechecks safe status after a transient failure',async()=>{
  let ready=false;
  const setup=fixture({status:()=>{if(!ready)throw fault('unavailable');return current();}});
  await setup.checkin({provider:provider()});await setup.controller().checkin({provider:provider()});
  assert.equal(setup.notifications.length,1);assert.equal(setup.claims.length,0);
  ready=true;assert.equal((await setup.controller().checkin({provider:provider()})).status,'claimed');
  assert.equal(setup.claims.length,1);assert.equal(setup.notifications.length,2);
  assert.equal(setup.notifications[1][2],'success');
});


test('ignores older captured settings after a newer save',async()=>{
  for(const [captured,stored] of [['4','5'],['9007199254740992','9007199254740993']]) {
    let transactions=0;
    const api=vm.runInNewContext(`${schedules}\n({register,reconcile})`,{channel,
      context:()=>({package:{name:'workbuddy',settings_revision:captured}}),
      readSettings:async()=>({values:{checkin_time:'09:05'}}),
      read:async()=>({present:true,revision:'8',value:{id:'existing',provider:provider(),time:'11:30',settings_revision:stored}}),
      prepareRequest:()=>{throw new Error('Older settings must not query dispatch');},
      prepareTransaction:()=>{transactions++;throw new Error('Older settings must not overwrite timing');},
    });
    assert.deepEqual(plain(await api.register({provider:provider()})),{registered:true,skipped:true});
    assert.deepEqual(plain(await api.reconcile({key:`checkin/schedule/${provider().id}`})),{updated:0});
    assert.equal(transactions,0);
  }
});

test('disables automatic claims without blocking manual checks',async()=>{
  const settings={checkin_enabled:false},setup=fixture({settings});
  assert.deepEqual(plain(await setup.checkin({provider:provider(),automatic:true})),{status:'disabled',skipped:true});
  assert.deepEqual([setup.requests.length,setup.claims.length,setup.notifications.length],[0,0,0]);
  assert.equal((await setup.checkin({provider:provider()})).status,'claimed');
  settings.checkin_enabled=true;
  assert.equal((await setup.checkin({provider:provider(),automatic:true})).skipped,true);
  assert.deepEqual([setup.claims.length,setup.notifications.length],[1,1]);
});

test('registers disabled schedules without publishing claims',async()=>{
  let operations;
  const register=vm.runInNewContext(`${schedules}\nregister`,{channel,
    readSettings:async()=>({values:{checkin_enabled:false,checkin_time:'10:45'}}),
    context:()=>({package:{name:'workbuddy',settings_revision:1}}),newId:()=> 'disabled',
    read:async()=>({present:false,revision:'0'}),prepareRequest:command=>command,
    prepareTransaction:value=>{operations=plain(value);return value;},
    complete:async request=>Array.isArray(request)?{}:{data:[]},
  });
  assert.equal((await register({provider:provider()})).registered,true);
  assert.equal(operations[1].data.data.enabled,false);
  assert.equal(operations[1].data.data.timing.data.hour,10);
  assert.equal(operations[1].data.data.timing.data.minute,45);
  assert.equal(operations[2].data.data.name,'disabled.settings');
  assert.ok(operations.every(item=>!['publish'].includes(item.data?.kind)));
  assert.ok(operations.every(item=>item.data?.data?.callback?.command?.data?.handler!=='checkin'));
});

test('toggles the same schedule and cancels queued callbacks',async()=>{
  const settings={checkin_enabled:false,checkin_time:'07:30'};
  let revision=2,record={id:'existing',provider:provider(),locale:'en',time:'09:05',settings_revision:'1'},saved;
  const schedule={id:'existing',revision:5,enabled:true};
  let handlers=[{name:'existing',revision:7}];
  const reconcile=vm.runInNewContext(`${schedules}\nreconcile`,{channel,
    context:()=>({package:{name:'workbuddy',settings_revision:revision}}),
    readSettings:async()=>({values:settings}),read:async()=>({present:true,revision:'3',value:record}),
    prepareRequest:command=>command,prepareTransaction:operations=>operations,
    complete:async request=>{
      if(!Array.isArray(request)) return {data:request.data.action.kind==='list_schedules'?[schedule]:handlers};
      saved=plain(request);record=saved[0].data.value;Object.assign(schedule,saved[1].data.data);return {};
    },
  });
  assert.equal((await reconcile({key:`checkin/schedule/${provider().id}`})).updated,1);
  assert.equal(saved[1].data.data.id,'existing');assert.equal(saved[1].data.data.enabled,false);
  assert.deepEqual(saved[2].data,{kind:'remove_handler',data:{name:'existing',expected_revision:'7',cancel_pending:true}});
  handlers=[];revision++;settings.checkin_enabled=true;
  assert.equal((await reconcile({key:`checkin/schedule/${provider().id}`})).updated,1);
  assert.equal(saved[1].data.data.id,'existing');assert.equal(saved[1].data.data.enabled,true);
  assert.equal(saved[1].data.data.timing.data.hour,7);assert.equal(saved[1].data.data.timing.data.minute,30);
  assert.equal(saved[2].data.data.name,'existing');assert.equal(saved[2].data.data.revision,'0');
  assert.equal(saved[2].data.data.callback.command.data.handler,'checkin');
  assert.ok(saved.every(item=>item.data?.kind!=='publish'));
});

test('preserves manual pauses through master toggles',async()=>{
  for(const initiallyEnabled of [false,true]) {
    const settings={checkin_enabled:true,checkin_time:'07:30'};
    let revision=2,record={id:'existing',provider:provider(),locale:'en',time:'09:05',settings_revision:'1'};
    const schedule={id:'existing',revision:5,enabled:initiallyEnabled};
    let handlers=[{name:'existing',revision:7}],saved;
    const api=vm.runInNewContext(`${schedules}\n({register,reconcile})`,{channel,
      context:()=>({package:{name:'workbuddy',settings_revision:revision}}),
      readSettings:async()=>({values:settings}),read:async()=>({present:true,revision:'3',value:record}),
      prepareRequest:command=>command,prepareTransaction:operations=>operations,
      newId:()=>{throw new Error('Existing schedule must keep its identity');},
      complete:async request=>{
        if(!Array.isArray(request)) return {data:request.data.action.kind==='list_schedules'?[schedule]:handlers};
        saved=plain(request);record=saved[0].data.value;Object.assign(schedule,saved[1].data.data);
        const callback=saved.find(item=>['save_handler','remove_handler'].includes(item.data?.kind));
        if(callback) handlers=callback.data.kind==='remove_handler'?[]:[{...callback.data.data,revision:8}];
        return {};
      },
    });
    const reconcile=()=>api.reconcile({key:`checkin/schedule/${provider().id}`});
    // The newly injected default and a time edit must not enable a legacy pause.
    assert.equal((await reconcile()).updated,1);
    assert.equal(schedule.enabled,initiallyEnabled);assert.equal(record.paused,undefined);
    assert.equal(saved.length,2);assert.equal(saved[1].data.data.id,'existing');
    revision++;settings.checkin_enabled=false;
    assert.equal((await reconcile()).updated,1);
    assert.equal(schedule.enabled,false);
    assert.equal(record.paused,initiallyEnabled?true:undefined);
    assert.equal(saved[2].data.kind,'remove_handler');assert.equal(saved[2].data.data.cancel_pending,true);
    revision++;settings.checkin_time='06:15';
    assert.equal((await reconcile()).updated,1);
    assert.equal(schedule.enabled,false);assert.equal(record.paused,initiallyEnabled?true:undefined);
    revision++;settings.checkin_enabled=true;
    assert.equal((await reconcile()).updated,1);
    assert.equal(schedule.enabled,initiallyEnabled);assert.equal(record.paused,undefined);
    assert.equal(schedule.timing.data.hour,6);assert.equal(schedule.timing.data.minute,15);
    assert.equal(saved[2].data.kind,'save_handler');
    assert.ok(saved.every(item=>item.data?.kind!=='publish'));
    // Explicit registration also preserves a manual pause after another time edit.
    revision++;settings.checkin_time='08:45';
    assert.equal((await api.register({provider:provider()})).registered,true);
    assert.equal(schedule.enabled,initiallyEnabled);
    assert.equal(saved.some(item=>item.data?.kind==='publish'),false);
  }
});

test('keeps automatic check-in off until it is chosen',async()=>{
  for(const settings of [{},{checkin_time:'09:05'},{checkin_enabled:false}]) {
    const setup=fixture({settings});
    assert.deepEqual(plain(await setup.checkin({provider:provider(),automatic:true})),{status:'disabled',skipped:true});
    assert.deepEqual([setup.requests.length,setup.claims.length,setup.notifications.length],[0,0,0]);
  }
  const calls=[];
  const register=vm.runInNewContext(`${schedules}\nregister`,{channel,
    readSettings:async()=>({values:{}}),read:async()=>({present:false,revision:'0'}),newId:()=>'new',context:()=>({package:{name:'workbuddy'}}),
    prepareRequest:command=>command,prepareTransaction:operations=>{calls.push(plain(operations));return operations;},
    complete:async command=>Array.isArray(command)?[]:{data:[]},
  });
  assert.equal((await register({provider:provider(),locale:'en'})).registered,true);
  assert.equal(calls[0][0].data.value.enabled,false);
  assert.equal(calls[0][1].data.data.enabled,false);
  assert.equal(calls[0].some(item=>item.data?.kind==='publish'),false);
});

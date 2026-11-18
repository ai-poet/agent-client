import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
const plain=value=>JSON.parse(JSON.stringify(value));
async function fixture() {
  let state;
  const context=vm.createContext({});
  const sdk=new vm.SyntheticModule(['stageTurnState','readTurnState'],function(){this.setExport('stageTurnState',value=>state=plain(value));this.setExport('readTurnState',()=>plain(state));},{context});
  const modules=new Map([['sailry/sdk',sdk]]);
  async function load(name) {
    if(modules.has(name))return modules.get(name);
    const source=await readFile(new URL(`../dev.sailry.platform/host/${name}`,import.meta.url),'utf8');
    const module=new vm.SourceTextModule(source,{context,identifier:name});modules.set(name,module);await module.link(load);return module;
  }
  const main=await load('./main.js');await main.evaluate();return main.namespace;
}
const tools=['ssh_run','ssh_transfer'];
const connections={bound:{kind:'ssh',id:'a'},databases:[],ssh:[{id:'a',name:'Work host'}]};
const created='11111111-1111-4111-8111-111111111111';
const other='22222222-2222-4222-8222-222222222222';
const management=['ssh_connections','ssh_save_connection'];

test('binds the saved target and package workspace guidance',async()=>{
  const api=await fixture();
  const result=api.initialize({tools,resources:true,connections});
  assert.equal(result.parameters.ssh_run.properties.connection.enum,undefined);
  assert.match(created,new RegExp(result.parameters.ssh_run.properties.connection.pattern));
  assert.equal(result.parameters.ssh_run.properties.connection.description,'[["a","Work host"]]');
  assert.equal(result.parameters.ssh_run.properties.connection.default,'a');
  assert.match(result.instruction,/primary SSH connection is a/);
  assert.match(result.instruction,/does not restrict other SSH operations/);
  const manifest=JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  assert.ok(manifest.extensions['dev.sailry.platform'].desktop.conversations[0].context.includes('SSH workspace assistant'));
  assert.match(api.initialize({tools,resources:false,connections}).instruction,/primary SSH connection is a/);
  assert.equal(api.initialize({tools,resources:true,connections:{...connections,bound:null}}).instruction,'');
});

test('primary defaults retain every captured SSH target',async()=>{
  const api=await fixture();
  const profiles=[{id:created,name:'Primary host'},{id:other,name:'Other private host'}];
  const initialized=api.initialize({tools:[...tools,...management],connections:{bound:{kind:'ssh',id:created},ssh:profiles,databases:[]}});
  for(const name of tools) {
    assert.equal(initialized.parameters[name].properties.connection.default,created);
    assert.equal(initialized.parameters[name].properties.connection.description,JSON.stringify(profiles.map(profile=>[profile.id,profile.name])));
  }
  assert.equal(initialized.parameters.ssh_save_connection.properties.connection.default,undefined);
  assert.equal(api.run({connection:other,command:'uname -a'}).connection,other);
  assert.equal(api.transfer({connection:other,path:'local.txt',remote_path:'/tmp/remote.txt',direction:'upload'}).connection,other);
  assert.deepEqual(plain(api.save({connection:other,expected_revision:1,name:'Renamed'})),{connection:other,expected_revision:1,name:'Renamed'});
  assert.equal(api.save({name:'Third',host:'third.example',port:22,username:'user',authentication:'agent'}).isError,undefined);
  const manifest=JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  const context=manifest.extensions['dev.sailry.platform'].desktop.conversations[0].context;
  assert.match(context,/default target and conversation association/);
  assert.match(context,/inspect, use, create and edit/);
  assert.doesNotMatch(context,/Work only on the saved SSH connection|Never bypass host-key verification, access another connection/);
  const preview=manifest.extensions['dev.sailry.platform'].tools.find(tool=>tool.name==='ssh_save_connection').display.output.preview;
  for(const part of preview.parts) assert.deepEqual(part.when,{all:[{value:{source:'result',path:'/kind'},equals:'ssh_profile'}]});
});

test('keeps command text, enforces target and timeout, and never accepts caller credentials',async()=>{
  const api=await fixture();
  const initialized=api.initialize({tools,resources:true,connections});
  const manifest=JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  assert.equal(initialized.parameters.ssh_run.properties.timeout_ms.maximum,600000);
  assert.equal(manifest.extensions['dev.sailry.platform'].tools.find(tool=>tool.name==='ssh_run')
    .handler.parameters.properties.timeout_ms.maximum,600000);
  const command='printf "资料 🙂\\n"';
  assert.deepEqual(plain(api.run({connection:'a',command})),{connection:'a',command,timeout_ms:120000});
  assert.equal(api.run({connection:'a',command,timeout_ms:600000}).timeout_ms,600000);
  for(const args of [{connection:'b',command},{connection:'a',command,timeout_ms:0},
    {connection:'a',command,timeout_ms:600001},{connection:'a',command,password:'hidden'}]) assert.equal(api.run(args).isError,true);
});

test('prepares only explicit upload or download while preserving both path domains',async()=>{
  const api=await fixture();api.initialize({tools,resources:true,connections});
  const input={connection:'a',path:'local/资料.txt',remote_path:'/srv/资料.txt',direction:'upload'};
  assert.deepEqual(plain(api.transfer(input)),{...input,timeout_ms:120000});
  assert.equal(api.transfer({...input,direction:'other'}).isError,true);
  assert.equal(api.transfer({...input,worktree:'caller-selected'}).isError,true);
  assert.deepEqual(plain(api.initialize({tools,resources:true,connections:{...connections,ssh:[]}}).tools),tools);
});

test('distinguishes unavailable connections from invalid inputs',async()=>{
  const api=await fixture();api.initialize({tools,connections});
  for(const call of [()=>api.run({connection:other,command:'uname -a'}),
    ()=>api.transfer({connection:other,path:'file.txt',remote_path:'/tmp/file.txt',direction:'upload'})]) {
    assert.equal(call().error.code,'permission_denied');
    assert.match(call().error.message,/ssh_connections/);
  }
  assert.equal(api.run({connection:other,command:null}).error.code,'invalid_request');
  assert.equal(api.transfer({connection:other,path:'file.txt',remote_path:'/tmp/file.txt',direction:'other'}).error.code,'invalid_request');
});

test('retains saved names without changing operation outcomes',async()=>{
  const api=await fixture();api.initialize({tools,connections});
  const output={kind:'ssh_outcome',data:{kind:'command',stdout:'ready'}};
  assert.deepEqual(plain(api.completed({arguments:{connection:'a'},output})),{...output,connection_name:'Work host'});
  assert.equal(api.completed({arguments:{connection:other},output}),output);
  const saved={kind:'ssh_profile',data:{id:created,name:'Build host',revision:1}};
  api.saved({arguments:{},output:saved});
  assert.deepEqual(plain(api.completed({arguments:{connection:created},output})),{...output,connection_name:'Build host'});
  const failure={isError:true,error:{code:'unavailable',message:'Unavailable'}};
  const named=plain(api.completed({arguments:{connection:created},output:failure}));
  assert.equal(named.isError,true);
  assert.deepEqual(named.error,failure.error);
  assert.equal(named.connection_name,'Build host');
  api.saved({arguments:{connection:created},output:{kind:'ssh_profile',data:{id:created,name:'Renamed host'}}});
  assert.equal(api.completed({arguments:{connection:created},output}).connection_name,'Renamed host');
  for(const name of ['',null,7]) {
    api.initialize({tools,connections:{ssh:[{id:created,name}]}});
    assert.equal(api.completed({arguments:{connection:created},output}),output);
  }
  const manifest=JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  for(const name of tools) {
    const tool=manifest.extensions['dev.sailry.platform'].tools.find(tool=>tool.name===name);
    assert.equal(tool.handler.result,'completed');
    assert.equal(tool.display.input.summary.format,'connection');
    assert.equal(tool.display.input.summary.result_path,'/connection_name');
  }
});

test('saves accept typed credentials without exposing read or trust inputs',async()=>{
  const api=await fixture();
  const all=[...tools,...management];
  const initialized=api.initialize({tools:all,connections:{ssh:[],databases:[]}});
  assert.deepEqual(plain(initialized.tools),all);
  assert.deepEqual(plain(api.connections({})),{});
  for(const args of [null,[],{host:'example.com'},{password:'hidden'}]) assert.equal(api.connections(args).isError,true);
  const schema=initialized.parameters.ssh_save_connection;
  assert.equal(schema.additionalProperties,false);
  assert.equal(schema.properties.expected_revision.maximum,undefined);
  assert.deepEqual(plain(schema.properties.authentication.enum),['password','private_key','key_path','agent']);
  assert.deepEqual(plain(schema.properties.credential.oneOf).map(value=>value.properties.kind.const),['password','private_key','key_path','agent']);
  for(const value of schema.properties.credential.oneOf) assert.equal(value.additionalProperties,false);
  for(const key of ['password','key','passphrase','host_key','sharing']) assert.equal(schema.properties[key],undefined);
  assert.deepEqual(plain(schema.allOf[0].then.required),['expected_revision']);
  const manifest=JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  const extension=manifest.extensions['dev.sailry.platform'];
  for(const name of management) {
    const tool=extension.tools.find(tool=>tool.name===name);
    assert.equal(tool.presentation,name==='ssh_connections'?'summary':'details');
    assert.equal(tool.handler.operation,name==='ssh_connections'?'ssh.connections':'ssh.save');
    assert.equal(tool.handler.result,name==='ssh_save_connection'?'saved':undefined);
    assert.deepEqual(tool.handler.parameters,plain(initialized.parameters[name]));
    assert.ok(extension.desktop.conversations[0].tools.some(tool=>tool.name===name));
  }
  assert.match(extension.instructions,/user must confirm the exact first host fingerprint/);
});

test('save preserves omission and explicit native credential replacements',async()=>{
  const api=await fixture();api.initialize({tools:management,connections:{ssh:[{id:created,name:'Work host'}],databases:[]}});
  const agent={name:'Work host',host:'ssh.example',port:22,username:'user',authentication:'agent'};
  assert.deepEqual(plain(api.save(agent)),agent);
  assert.deepEqual(plain(api.save({...agent,connection:null,expected_revision:null})),agent);
  const key={...agent,authentication:'key_path',key_path:'/keys/id_ed25519'};
  assert.deepEqual(plain(api.save(key)),key);
  for(const credential of [{kind:'password',password:'supplied-password'},
    {kind:'private_key',key:'supplied-key',passphrase:'supplied-passphrase'},
    {kind:'private_key',key:'replacement-key'}, {kind:'key_path',path:'/keys/id_ed25519',passphrase:null},
    {kind:'agent'}]) {
    const input={...agent,authentication:credential.kind,credential};
    assert.deepEqual(plain(api.save(input)),input);
    assert.deepEqual(plain(api.save({connection:created,expected_revision:3,credential})),
      {connection:created,expected_revision:3,credential});
  }
  const update={connection:created,expected_revision:3,name:'Renamed'};
  assert.deepEqual(plain(api.save(update)),update);
  assert.deepEqual(plain(api.save({...update,authentication:'key_path'})),{...update,authentication:'key_path'});
  for(const args of [null,[],{}, {...agent,port:0}, {...agent,port:65536}, {...agent,port:1.5},
    {...agent,username:null}, {...agent,authentication:'password'}, {...agent,key_path:'/keys/key'},
    {...key,key_path:null}, {...key,key_path:''}, {connection:created}, {expected_revision:1},
    {...agent,credential:null}, {...agent,credential:{kind:'password',password:'supplied'}},
    {...key,credential:{kind:'key_path',path:'/keys/key'}},
    {...agent,authentication:'password',credential:{kind:'password',password:7}},
    {...agent,authentication:'private_key',credential:{kind:'private_key',key:'supplied',passphrase:7}},
    {...agent,credential:{kind:'agent',password:'hidden'}},
    ...['constructor','__proto__','toString'].map(kind=>({...agent,credential:{kind}})),
    {connection:created,expected_revision:null}, {connection:other,expected_revision:1},
    {connection:created,expected_revision:0}, {...update,name:null},
    {connection:created,expected_revision:Number.MAX_SAFE_INTEGER+1},
    ...['password','key','passphrase','host_key','sharing'].map(key=>({...agent,[key]:'hidden'}))]) {
    assert.equal(api.save(args).error.code,'invalid_request');
  }
});

test('only successful save results enable same-turn SSH calls',async()=>{
  const api=await fixture();api.initialize({tools:[...tools,...management],connections:{ssh:[],databases:[]}});
  const args={name:'Work host',host:'ssh.example',port:22,username:'user',authentication:'agent'};
  const run={connection:created,command:'uname -a'};
  assert.equal(api.run(run).isError,true);
  for(const output of [{isError:true,error:{code:'denied',message:'Denied'}},
    {kind:'ssh_profiles',data:[{id:created}]}, {kind:'ssh_profile',data:{id:'invalid'}},
    {kind:'ssh_profile',data:{id:created},error:{code:'outcome_unknown'}}]) {
    assert.equal(api.saved({arguments:args,output}),output);
    assert.equal(api.run(run).isError,true);
  }
  const output={kind:'ssh_profile',data:{id:created,revision:1,authentication:{kind:'agent'}}};
  assert.equal(api.saved({arguments:args,output}),output);
  assert.equal(api.saved({arguments:args,output}),output);
  assert.deepEqual(plain(api.run(run)),{...run,timeout_ms:120000});
  api.saved({arguments:{connection:created,expected_revision:1},output:{kind:'ssh_profile',data:{id:other}}});
  assert.equal(api.run({...run,connection:other}).isError,true);
  assert.deepEqual(plain(api.transfer({connection:created,path:'local.txt',remote_path:'/tmp/remote.txt',direction:'upload'})),
    {connection:created,path:'local.txt',remote_path:'/tmp/remote.txt',direction:'upload',timeout_ms:120000});
});

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
  const main=await load('./main.js');await main.evaluate();
  return main.namespace;
}
const profiles=[{id:'a',name:'Reporting',engine:'mysql',database:'schema-name',read_only:true},{id:'b',name:'Analysis',engine:'postgres',database:'analytics',read_only:false}];
const tools=['database_catalog','database_query','database_execute'];
const created='11111111-1111-4111-8111-111111111111';
const other='22222222-2222-4222-8222-222222222222';
const management=['database_connections','database_save_connection'];

test('keeps exact captured choices and engine policy in the declared schemas',async()=>{
  const api=await fixture();
  const initialized=api.initialize({tools,resources:true,connections:{bound:{kind:'database',id:'a'},databases:profiles,ssh:[]}});
  assert.deepEqual(plain(initialized.tools),tools);
  assert.equal(initialized.instruction,'');
  const manifest=JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  assert.ok(manifest.extensions['dev.sailry.platform'].desktop.conversations[0].context.includes('database workspace assistant'));
  for(const name of tools) {
    const tool=manifest.extensions['dev.sailry.platform'].tools.find(tool=>tool.name===name);
    assert.equal(tool.presentation,name==='database_catalog'?'summary':name==='database_query'?'content':'details');
    const schema=initialized.parameters[name];
    assert.equal(schema.properties.connection.enum,undefined);
    assert.match(created,new RegExp(schema.properties.connection.pattern));
    assert.deepEqual(JSON.parse(schema.properties.connection.description).map(([id])=>id),['a','b']);
    assert.ok(schema.properties.connection.description.includes('backtick-quoted identifiers'));
    assert.ok(schema.properties.connection.description.includes('qualify tables with schema, not database'));
    assert.equal(schema.additionalProperties,false);
  }
  assert.deepEqual(plain(initialized.parameters.database_query.required),['connection','sql']);
});

test('retains SQL exactly and only prepares the admitted targets and operation defaults',async()=>{
  const api=await fixture();api.initialize({tools,resources:true,connections:{bound:null,databases:profiles,ssh:[]}});
  const sql='SELECT "value" FROM "资料" WHERE id = 9007199254740993';
  assert.deepEqual(plain(api.query({connection:'b',database:'analytics',sql})),{connection:'b',database:'analytics',sql,row_limit:1000,timeout_ms:30000});
  assert.deepEqual(plain(api.catalog({connection:'a'})),{connection:'a'});
  for(const args of [{connection:'other',sql},{connection:'a',sql,read_only:false},{connection:'a',sql:7},{connection:'a',database:{},sql}]) assert.equal(api.execute(args).isError,true);
});

test('does not add workspace instructions to imported tools or project use',async()=>{
  const api=await fixture();
  for(const [resources,bound] of [[false,{kind:'database',id:'a'}],[true,null]]) {
    const result=api.initialize({tools:['database_query'],resources,connections:{bound,databases:profiles,ssh:[]}});
    assert.equal(result.instruction,'');assert.deepEqual(Object.keys(result.parameters),['database_query']);
  }
  const empty=api.initialize({tools,resources:true,connections:{bound:null,databases:[],ssh:[]}});
  assert.deepEqual(plain(empty.tools),tools);
});

test('save captures its successful ID while other outputs remain raw',async()=>{
  const manifest=JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  for(const tool of manifest.extensions['dev.sailry.platform'].tools) {
    assert.equal(tool.handler.result,tool.name==='database_save_connection' ? 'saved' : undefined);
  }
});

test('connection tools remain available without initial profiles',async()=>{
  const api=await fixture();
  const all=[...tools,...management];
  const initialized=api.initialize({tools:all,connections:{databases:[],ssh:[]}});
  assert.deepEqual(plain(initialized.tools),all);
  assert.deepEqual(plain(initialized.parameters.database_connections),{
    type:'object',properties:{},required:[],additionalProperties:false,
  });
  assert.deepEqual(plain(api.connections({})),{});
  for(const args of [null,[],{connection:created},{password:'hidden'}]) assert.equal(api.connections(args).isError,true);
  const schema=initialized.parameters.database_save_connection;
  assert.equal(schema.additionalProperties,false);
  assert.equal(schema.properties.expected_revision.maximum,undefined);
  assert.deepEqual(plain(schema.properties.engine.enum),['sqlite','mysql','postgres']);
  assert.equal(schema.properties.password,undefined);
  assert.equal(schema.properties.sharing,undefined);
  assert.deepEqual(plain(schema.allOf[0].then.required),['expected_revision']);
  assert.deepEqual(plain(schema.allOf[0].else.required),['name','engine']);
  const manifest=JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  const extension=manifest.extensions['dev.sailry.platform'];
  for(const name of management) {
    const tool=extension.tools.find(tool=>tool.name===name);
    assert.equal(tool.presentation,name==='database_connections'?'summary':'details');
    assert.equal(tool.handler.operation,name==='database_connections'?'databases.connections':'databases.save');
    assert.deepEqual(tool.handler.parameters,plain(initialized.parameters[name]));
    assert.ok(extension.desktop.conversations[0].tools.some(tool=>tool.name===name));
  }
  assert.match(extension.instructions,/SQLite needs no credentials/);
});

test('save validates create fields and preserves omitted update fields',async()=>{
  const api=await fixture();api.initialize({tools:management,connections:{databases:[{...profiles[0],id:created}],ssh:[]}});
  const sqlite={name:'Local data',engine:'sqlite',path:'/tmp/资料.sqlite'};
  assert.deepEqual(plain(api.save(sqlite)),{...sqlite,read_only:true});
  for(const path of ['sales.db','./sales.db','data/sales.db']) {
    assert.deepEqual(plain(api.save({...sqlite,path})),{...sqlite,path,read_only:true});
  }
  const schema=api.initialize({tools:management,connections:{databases:[],ssh:[]}}).parameters.database_save_connection;
  assert.match(schema.properties.path.description,/relative paths use the captured worktree/);
  assert.match(schema.properties.path.description,/Saving does not create the file/);
  api.initialize({tools:management,connections:{databases:[{...profiles[0],id:created}],ssh:[]}});
  assert.deepEqual(plain(api.save({...sqlite,connection:null,expected_revision:null})),{...sqlite,read_only:true});
  const network={name:'Analytics',engine:'postgres',host:'db.example',port:5432,username:'analyst',database:'analytics',read_only:false};
  assert.deepEqual(plain(api.save(network)),network);
  const update={connection:created,expected_revision:3,name:'Renamed'};
  assert.deepEqual(plain(api.save(update)),update);
  assert.deepEqual(update,{connection:created,expected_revision:3,name:'Renamed'});
  for(const args of [null,[],{}, {name:'Missing engine'}, {...sqlite,path:null}, {...sqlite,engine:'unknown'},
    {...sqlite,read_only:null}, {...sqlite,password:'hidden'}, {...sqlite,sharing:[]},
    {...network,port:0}, {...network,port:65536}, {...network,port:1.5}, {...network,username:null},
    {connection:created}, {expected_revision:1}, {connection:created,expected_revision:null},
    {connection:other,expected_revision:1}, {connection:created,expected_revision:0},
    {connection:created,expected_revision:Number.MAX_SAFE_INTEGER+1},
    {connection:created,expected_revision:1,name:null}]) assert.equal(api.save(args).error.code,'invalid_request');
  const unavailable=api.save({connection:other,expected_revision:1,read_only:false});
  assert.match(unavailable.error.message,/Connection is not available in this session/);
  assert.match(unavailable.error.message,/create a connection without connection and expected_revision/);
  assert.match(api.initialize({tools:management,connections:{databases:[],ssh:[]}})
    .parameters.database_save_connection.properties.connection.description,/omit with expected_revision to create/);
});

test('only successful save results enable same-turn queries',async()=>{
  const api=await fixture();api.initialize({tools:[...tools,...management],connections:{databases:[],ssh:[]}});
  const args={name:'Local data',engine:'sqlite',path:'/tmp/data.sqlite'};
  const query={connection:created,sql:'SELECT 42'};
  assert.equal(api.query(query).isError,true);
  for(const output of [{isError:true,error:{code:'denied',message:'Denied'}},
    {kind:'database_profiles',data:[{id:created}]}, {kind:'database_profile',data:{id:'invalid'}},
    {kind:'database_profile',data:{id:created},error:{code:'outcome_unknown'}}]) {
    assert.equal(api.saved({arguments:args,output}),output);
    assert.equal(api.query(query).isError,true);
  }
  const output={kind:'database_profile',data:{id:created,revision:1,engine:'sqlite',credentials_required:false}};
  assert.equal(api.saved({arguments:args,output}),output);
  assert.equal(api.saved({arguments:args,output}),output);
  assert.deepEqual(plain(api.query(query)),{...query,row_limit:1000,timeout_ms:30000});
  const mismatched={kind:'database_profile',data:{id:other,revision:2}};
  api.saved({arguments:{connection:created,expected_revision:1},output:mismatched});
  assert.equal(api.query({...query,connection:other}).isError,true);
  assert.deepEqual(plain(api.save({connection:created,expected_revision:1,name:'Updated'})),
    {connection:created,expected_revision:1,name:'Updated'});
});

test('describes safe writable SQLite initialization',async()=>{
  const api=await fixture();
  const initialized=api.initialize({tools:[...tools,...management],connections:{databases:[],ssh:[]}});
  const manifest=JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  const extension=manifest.extensions['dev.sailry.platform'];
  const save=extension.tools.find(tool=>tool.name==='database_save_connection');
  const execute=extension.tools.find(tool=>tool.name==='database_execute');
  assert.match(save.description,/Saving records configuration only and does not create a SQLite file/);
  assert.match(save.description,/default to read_only=true/);
  assert.match(save.description,/If database_connections returns no profiles, create without connection or expected_revision/);
  assert.match(execute.description,/authorized schema write initializes it and creates missing parent directories/);
  assert.match(execute.description,/Explicit writable connection checks can also initialize/);
  assert.match(execute.description,/Reads, catalog, read-only connection checks, and PRAGMAs do not initialize/);
  assert.match(extension.instructions,/Do not write an empty file as a workaround/);
  assert.match(extension.instructions,/silently change a read-only connection/);
  assert.deepEqual(plain(save.handler.parameters),plain(initialized.parameters.database_save_connection));
  const args={name:'New SQLite',engine:'sqlite',path:'output/test.db',read_only:false};
  assert.deepEqual(plain(api.save(args)),args);
  const readOnly={name:args.name,engine:args.engine,path:args.path};
  assert.deepEqual(plain(api.save(readOnly)),{...readOnly,read_only:true});
  const output={kind:'database_profile',data:{id:created,revision:1,engine:'sqlite',read_only:false,credentials_required:false}};
  api.saved({arguments:args,output});
  assert.deepEqual(plain(api.execute({connection:created,sql:'CREATE TABLE items(value INTEGER)'})),{
    connection:created,sql:'CREATE TABLE items(value INTEGER)',row_limit:1000,timeout_ms:30000,
  });
  assert.equal(save.handler.operation,'databases.save');
  assert.equal(execute.handler.operation,'databases.execute');
});

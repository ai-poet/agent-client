import test from 'node:test';
import assert from 'node:assert/strict';
import {checkConnectors} from '../connectors.mjs';
const declaration = () => ({id:'example',display:{label:'Example'},accounts:'single',origins:['https://api.example.com'],
  auth:[{kind:'secret',fields:['token']}],fields:[{name:'token',title:'Token',secret:true}],inject:[{origin:'https://api.example.com',header:'Authorization',prefix:'Bearer '}]});
const check = item => checkConnectors({connectors:[item]}, '.');

test('accepts the current public declaration', () => { assert.doesNotThrow(() => check(declaration())); });
test('rejects nonpublic or noncanonical origins', () => {
  for (const origin of ['http://api.example.com','https://127.0.0.1','https://[::1]','https://host.local','https://api.example.com/path','https://api.example.com/','https://user@api.example.com']) {
    const item = declaration(); item.origins=[origin]; item.inject[0].origin=origin;
    assert.throws(() => check(item));
  }
});
test('confines injection targets and headers', () => {
  for (const change of [item=>item.inject[0].origin='https://other.example.com',item=>item.inject[0].header='Cookie',item=>item.inject[0].header='Proxy-Authorization',item=>item.inject.push({...item.inject[0]}),item=>item.inject[0].prefix='Bearer\n']) {
    const item=declaration(); change(item); assert.throws(()=>check(item));
  }
});
test('keeps secret inputs outside package defaults', () => {
  for (const change of [item=>item.fields[0].default='private',item=>item.fields.push({...item.fields[0]}),item=>item.accounts='multiple',item=>item.auth[0].fields=['unknown']]) {
    const item=declaration();change(item);assert.throws(()=>check(item));
  }
  const item=declaration();item.auth=[{kind:'oauth2',flow:'pkce',authorize:'https://api.example.com/authorize',token:'https://api.example.com/token',client:{bundled:{client_id:'public',client_secret:'private'}}}];
  assert.throws(()=>check(item));
});
test('bounds silent CLI declarations', () => {
  const cli={kind:'local_cli',command:'example',env:{NO_PROMPT:'1'},status:{argv:['status','--json'],account:'/account'},token:{argv:['token','--user','{account}'],parse:'line'},timeout_ms:15000,max_output:65536};
  const item=declaration();item.auth=[cli];assert.doesNotThrow(()=>check(item));
  for (const change of [auth=>auth.command='../example',auth=>auth.env.PATH='/tmp',auth=>auth.status.argv.push('{account}'),auth=>auth.token.argv.push('{input}'),auth=>auth.timeout_ms=30001,auth=>auth.max_output=65537]) {
    const value=structuredClone(item);change(value.auth[0]);assert.throws(()=>check(value));
  }
});
test('bounds account status conditions',()=>{
 const item=declaration();
 item.auth=[{kind:'local_cli',command:'example',status:{argv:['status'],account:'/account',condition:{pointer:'/active',values:[true]}},token:{argv:['token'],parse:'line'},timeout_ms:1000,max_output:1024}];
 assert.doesNotThrow(()=>check(item));
 for(const condition of [{pointer:'active',values:[true]},{pointer:'/active',values:[]},{pointer:'/active',values:[{}]},{pointer:'/active',values:Array(9).fill(true)}]){
  const changed=structuredClone(item);changed.auth[0].status.condition=condition;assert.throws(()=>check(changed));
 }
});

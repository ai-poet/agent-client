import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {authorize} from '../dev.sailry.platform/auth/flow.js';
import {runtimeTest,translate,translationInput} from '../../scripts/test-host.mjs';
const now=1800000000000;
const context={previous:null,fields:{client_id:'9d1c250a-e61b-44d9-88ed-5944d1962f5e'},scopes:'org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload'.split(' '),host_id:'install',redirect:'http://localhost:54545/callback',proof:{state:'private-state',verifier:'private-verifier',challenge:'public-challenge'}};
const step=(event,state=null,overrides={})=>authorize({event,state,context:{...context,...overrides},now_ms:now});
test('declares the existing callback and token contracts',async()=>{
 const manifest=JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
 assert.equal(manifest.version,'0.1.5');
 const declaration=manifest.extensions['dev.sailry.platform'].model_auth[0];
 assert.match(declaration.warning.text,/Anthropic's terms/);
 assert.deepEqual(Object.keys(declaration.warning.locales).sort(),['de','es','fr','ja','ko','pt-BR','ru','zh-CN','zh-TW']);
 for(const text of [declaration.warning.text,...Object.values(declaration.warning.locales)]) assert.ok(text.trim().length>0&&Buffer.byteLength(text)<=1024&&!/[\u0000-\u001f\u007f]/.test(text));
 assert.equal(declaration.authentication,'claude_oauth');
 assert.deepEqual(declaration.flow.callback,{path:'/callback',redirect_uri:context.redirect,manual:true,allow_localhost:true,port:54545});
 assert.equal('scope' in declaration.flow.requests.refresh.body.value,false);
 assert.equal(declaration.flow.requests.token.url,'https://api.anthropic.com/v1/oauth/token');
});
test('keeps PKCE proof private and supports pasted codes',()=>{
 const prompt=step({kind:'start'});
 const url=new URL(prompt.url);
 assert.equal(url.searchParams.get('code_challenge'),context.proof.challenge);
 assert.equal(url.searchParams.get('state'),context.proof.state);
 assert.equal(url.searchParams.get('redirect_uri'),context.redirect);
 assert.equal(url.searchParams.get('client_id'),'9d1c250a-e61b-44d9-88ed-5944d1962f5e');
 assert.equal(step({kind:'presented'},prompt.state).allow_code,true);
 const exchange=step({kind:'callback',fields:{code:'private-code'}},{phase:'returned'});
 assert.equal(exchange.name,'token');
 assert.equal(exchange.fields.verifier,context.proof.verifier);
});
test('keeps rotation and expiry semantics',()=>{
 const pending=step({kind:'response',status:200,body:{access_token:'access',refresh_token:'refresh',expires_in:3600}},{phase:'tokens'});
 const result=step({kind:'response',status:503,body:null},pending.state);
 assert.deepEqual(result,{kind:'finish',grant:{kind:'claude_oauth',access:'access',refresh:'refresh',expires_at_ms:now+3600000,renewal_pending:false,installation_id:'install',account_id:null,email:null,org_id:null,org_name:null}});
 assert.equal(step({kind:'refresh'},null,{previous:result.grant}).name,'refresh');
 for(const missing of ['access_token','refresh_token','expires_in']){
   const body={access_token:'access',refresh_token:'refresh',expires_in:3600};delete body[missing];
   assert.equal(step({kind:'response',status:200,body},{phase:'tokens'}).kind,'reject');
 }
});
test('keeps actual rejections outside token decoding',()=>{
 for(const status of [400,401,403])assert.equal(step({kind:'response',status,body:null},{phase:'tokens'}).code,'permission_denied');
 assert.equal(step({kind:'response',status:500,body:{access_token:'access'}},{phase:'tokens'}).kind,'reject');
});
runtimeTest('uses production private translation',async()=>{
 const manifest=JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
 const declaration=manifest.extensions['dev.sailry.platform'].model_auth[0];
 let state=null;
 const call=async event=>{
   const input={event,state,context,now_ms:now};
   const report=translate(await translationInput(new URL('../',import.meta.url),declaration.flow.script,'authorize',input));
   assert.deepEqual(report.calls,[]);assert.deepEqual(report.result.Ok,authorize(input));
   state=report.result.Ok.state;return report.result.Ok;
 };
 assert.equal((await call({kind:'start'})).kind,'prompt');
 assert.equal((await call({kind:'presented'})).kind,'callback');
 assert.equal((await call({kind:'callback',fields:{code:'private-code'}})).name,'token');
 assert.equal((await call({kind:'response',status:200,body:{access_token:'access',refresh_token:'refresh',expires_in:3600}})).name,'identity');
 const result=await call({kind:'response',status:503,body:null});
 assert.equal(result.grant.kind,'claude_oauth');
});

test('recovers identity without reassigning organizations',()=>{
 const tokens={access_token:'access',refresh_token:'refresh',expires_in:3600};
 const pending=step({kind:'response',status:200,body:tokens},{phase:'tokens'});
 assert.equal(pending.name,'identity');assert.equal(pending.optional,true);
 const completed=step({kind:'response',status:200,body:{oauth_account:{account_uuid:'account',account_email:'mail',organization_uuid:'org',organization_name:'Team'}}},pending.state);
 assert.equal(completed.grant.account_id,'account');assert.equal(completed.grant.org_id,'org');
 const refreshed=step({kind:'response',status:200,body:{...tokens,organization:{uuid:'other'}}},{phase:'tokens',refreshing:true},{previous:completed.grant});
 assert.equal(refreshed.kind,'finish');assert.equal(refreshed.grant.org_id,'org');
 const fallback=step({kind:'failure',code:'unavailable'},pending.state);
 assert.equal(fallback.kind,'finish');assert.equal(fallback.grant.access,'access');
});


test('resolves fresh login identity independently',()=>{
 const previous={account_id:'old-account',email:'old@example.test',org_id:'old-org',org_name:'Old',installation_id:'install'};
 const pending=step({kind:'response',status:200,body:{access_token:'access',refresh_token:'refresh',expires_in:3600}},{phase:'tokens',refreshing:false},{previous});
 assert.equal(pending.name,'identity');
 assert.equal(pending.state.grant.account_id,null);
 assert.equal(pending.state.grant.email,null);
 const result=step({kind:'response',status:200,body:{oauth_account:{account_uuid:'new-account',account_email:'new@example.test',organization_uuid:'new-org'}}},pending.state,{previous});
 assert.equal(result.grant.account_id,'new-account');
 assert.equal(result.grant.email,'new@example.test');
 assert.equal(result.grant.org_id,'new-org');
 assert.equal(result.grant.installation_id,'install');
});


test('uses effective version for bootstrap requests',()=>{
 const tokens={access_token:'access',refresh_token:'refresh',expires_in:3600};
 for(const version of ['2.1.280','2.1.990']){
   const pending=step({kind:'response',status:200,body:tokens},{phase:'tokens'},{fields:{...context.fields,user_agent:`claude-cli/${version} (external, cli)`}});
   assert.equal(pending.fields.bootstrap_agent,`claude-code/${version}`);
 }
});

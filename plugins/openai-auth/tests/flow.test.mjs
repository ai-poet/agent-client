import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {codex,siwc} from '../dev.sailry.platform/auth/flow.js';
import {runtimeTest,translate,translationInput} from '../../scripts/test-host.mjs';

const now=1800000000000;
const context={previous:null,host_id:'urn:uuid:fixture',registration:null,redirect:'http://127.0.0.1:12345/auth/callback',fields:{client_id:'app_EMoamEEZ73f0CkXaXp7hrann'},scopes:'openid profile email offline_access resource.invoke chatgpt.tokens.use.direct'.split(' '),proof:{state:'private-state',nonce:'private-nonce',verifier:'private-verifier',challenge:'public-challenge'}};
const jwt=claims=>`e30.${Buffer.from(JSON.stringify(claims)).toString('base64url')}.c2lnbmF0dXJl`;
const token={access_token:'fixture-access',refresh_token:'fixture-refresh',id_token:jwt({sub:'fixture-subject',email:'fixture@example.test',chatgpt_account_id:'fixture-account'}),expires_in:3600,token_type:'Bearer'};
const identity={sub:'fixture-subject',email:'fixture@example.test',chatgpt_account_id:'fixture-account'};
const step=(handler,event,state,overrides={})=>handler({event,state,context:{...context,redirect:handler===codex?'http://localhost:1455/auth/callback':context.redirect,...overrides},now_ms:now});
function codexTokens(body=token,overrides={}) {
  return step(codex,{kind:'response',status:200,body},{phase:'tokens',client_id:'app_EMoamEEZ73f0CkXaXp7hrann'},overrides);
}

test('keeps the two account protocols distinct',async()=>{
  const manifest=JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  assert.equal(manifest.version,'0.1.6');
  const declarations=manifest.extensions['dev.sailry.platform'].model_auth;
  assert.deepEqual(declarations.map(item=>[item.id,item.authentication]),[['siwc','chat_gpt'],['codex','chat_gpt_oauth']]);
  assert.equal(declarations[0].flow.callback.path,'/auth/callback');
  assert.equal(declarations[1].flow.callback.port,1455);
  assert.equal(declarations[1].flow.callback.redirect_uri,'http://localhost:1455/auth/callback');
  assert.deepEqual(Object.keys(declarations[1].flow.requests),['token','refresh']);
  assert.equal(declarations[1].flow.requests.refresh.body.fields.scope,'openid profile email');
  for(const declaration of declarations)assert.equal('token' in declaration,false);
});
test('preserves the existing credential representation',()=>{
  assert.deepEqual(codexTokens(),{kind:'finish',grant:{kind:'chat_gpt_oauth',access:'fixture-access',refresh:'fixture-refresh',expires_at_ms:now+3600000,subject:identity.sub,email:identity.email,id_token:token.id_token,renewal_pending:false,account_id:identity.chatgpt_account_id}});
  assert.equal(codexTokens({...token,expires_in:undefined}).kind,'reject');
  assert.equal(codexTokens({...token,id_token:undefined}).kind,'reject');
  assert.equal(codexTokens({...token,refresh_token:undefined}).kind,'reject');
});
test('retains omitted rotating fields on refresh',()=>{
  const previous=codexTokens().grant;
  const result=step(codex,{kind:'response',status:200,body:{access_token:'new-access',expires_in:3600}},{phase:'tokens',client_id:'app_EMoamEEZ73f0CkXaXp7hrann',refreshing:true},{previous});
  assert.equal(result.kind,'finish');
  assert.equal(result.grant.refresh,previous.refresh);
  assert.equal(result.grant.id_token,previous.id_token);
  assert.equal(result.grant.account_id,previous.account_id);
  assert.equal(step(codex,{kind:'response',status:200,body:{...token,id_token:jwt({...identity,chatgpt_account_id:'other'})}},{phase:'tokens',refreshing:true},{previous}).kind,'reject');
});
test('keeps SIWC registration and nonce bound',()=>{
  const prompted=step(siwc,{kind:'start'},null);
  const url=new URL(prompted.url);
  assert.equal(url.searchParams.get('client_id'),'dynamic_agent_client');
  assert.equal(url.searchParams.get('ext_agent_host_id'),context.host_id);
  assert.equal(url.searchParams.get('nonce'),context.proof.nonce);
  assert.equal(url.searchParams.get('resource'),'https://api.openai.com/v1');
  assert.equal(url.searchParams.get('code_challenge'),context.proof.challenge);
  assert.equal(prompted.url.includes(context.proof.verifier),false);
  const callback=step(siwc,{kind:'presented'},prompted.state);
  assert.equal(callback.proof,'state');
  const registered=step(siwc,{kind:'callback',fields:{code:'private-code',client_id:'oaiapp_fixture'}},callback.state);
  assert.equal(registered.kind,'registered');
  const exchange=step(siwc,{kind:'registered'},registered.state);
  assert.equal(exchange.name,'token');
  const verify=step(siwc,{kind:'response',status:200,body:{...token,scope:'profile openid profile'}},exchange.state);
  assert.equal(verify.nonce,context.proof.nonce);
  const finished=step(siwc,{kind:'identity',claims:identity},verify.state);
  assert.equal(finished.grant.client_id,'oaiapp_fixture');
  assert.deepEqual(finished.grant.scopes,['openid','profile']);
});
test('rejects changed registration and account',()=>{
  const previous={kind:'chat_gpt',client_id:'oaiapp_existing',subject:'existing-subject',refresh:'refresh',id_token:'id',scopes:['openid']};
  const prompted=step(siwc,{kind:'start'},null,{previous});
  assert.equal(new URL(prompted.url).searchParams.get('client_id'),previous.client_id);
  assert.equal(step(siwc,{kind:'callback',fields:{code:'private-code',client_id:'oaiapp_changed'}},{phase:'returned',client_id:previous.client_id},{previous}).kind,'reject');
  const verify=step(siwc,{kind:'response',status:200,body:token},{phase:'tokens',client_id:previous.client_id,refreshing:true},{previous});
  assert.equal(step(siwc,{kind:'identity',claims:identity},verify.state,{previous}).kind,'reject');
});
test('maps only valid Codex metadata shapes',()=>{
  assert.equal(codexTokens({...token,id_token:jwt({sub:identity.sub,'https://api.openai.com/auth':{chatgpt_account_id:'nested-account'}})}).grant.account_id,'nested-account');
  for(const claims of [{sub:identity.sub,chatgpt_account_id:42},{sub:identity.sub,'https://api.openai.com/auth':'wrong'},{sub:identity.sub,'https://api.openai.com/auth':{chatgpt_account_id:false}},{...identity,sub:undefined},{...identity,chatgpt_account_id:undefined}])assert.equal(codexTokens({...token,id_token:jwt(claims)}).kind,'reject');
});
test('keeps Codex metadata separate from OIDC',()=>{
  const claims={...identity,email:'测试@example.test'};
  const body={...token,id_token:jwt(claims)};
  const finished=codexTokens(body);
  assert.equal(finished.kind,'finish');
  assert.equal(finished.grant.email,claims.email);
  const refreshing=step(codex,{kind:'response',status:200,body},{phase:'tokens',refreshing:true},{previous:finished.grant});
  assert.equal(refreshing.kind,'finish');
  assert.equal(refreshing.grant.id_token,body.id_token);
  for(const refreshing of [false,true])assert.equal(step(siwc,{kind:'response',status:200,body},{phase:'tokens',client_id:'oaiapp_fixture',refreshing},{previous:refreshing?{...finished.grant,client_id:'oaiapp_fixture'}:null}).kind,'verify');
});
test('rejects malformed Codex metadata',()=>{
  for(const id_token of ['missing.parts','a..b','a.A.b','a._w.b','a.W10.b','a.e.b','a.eyJ9.b',jwt(null),jwt('text'),'a.'+'A'.repeat(16384)+'.b'])assert.equal(codexTokens({...token,id_token}).kind,'reject');
});
test('requires the existing token fields and types',()=>{
  for(const fields of [{expires_in:0},{expires_in:'3600'},{expires_in:1.5},{token_type:'Basic'},{access_token:''},{refresh_token:''},{id_token:''}])assert.equal(codexTokens({...token,...fields}).kind,'reject');
  const previous={kind:'chat_gpt',client_id:'oaiapp_existing',subject:identity.sub,refresh:'refresh',id_token:'id',scopes:['openid']};
  const verify=step(siwc,{kind:'response',status:200,body:{...token,scope:false}},{phase:'tokens',client_id:previous.client_id},{previous});
  assert.equal(step(siwc,{kind:'identity',claims:identity},verify.state,{previous}).kind,'reject');
});
test('retains SIWC registration scopes and refresh hints',()=>{
  const previous={kind:'chat_gpt',client_id:'oaiapp_existing',subject:identity.sub,email:identity.email,refresh:'old-refresh',id_token:'old-id',scopes:['openid','resource.invoke'],account_id:'resource-account'};
  const exchange=step(siwc,{kind:'refresh'},null,{previous});
  assert.equal(exchange.fields.client_id,previous.client_id);
  const grant=step(siwc,{kind:'response',status:200,body:{access_token:'new-access',refresh_token:'new-refresh',token_type:'Bearer',expires_in:3600,earliest_refresh_at:1790036100}},exchange.state,{previous}).grant;
  assert.equal(grant.client_id,previous.client_id);assert.equal(grant.id_token,previous.id_token);assert.deepEqual(grant.scopes,previous.scopes);
  assert.equal(grant.account_id,previous.account_id);assert.equal(grant.earliest_refresh_at,1790036100000);
  for (const earliest of [1790036100,1790036100000]) {
    const result=step(siwc,{kind:'response',status:200,body:{access_token:'new-access',refresh_token:'new-refresh',token_type:'Bearer',expires_in:3600,earliest_refresh_at:earliest}},exchange.state,{previous});
    assert.equal(result.grant.earliest_refresh_at,1790036100000);
  }
  for (const earliest of [-1,1.5,'1790036100']) {
    const result=step(siwc,{kind:'response',status:200,body:{access_token:'new-access',refresh_token:'new-refresh',token_type:'Bearer',expires_in:3600,earliest_refresh_at:earliest}},exchange.state,{previous});
    assert.equal(result.kind,'reject');
  }
  assert.equal(step(siwc,{kind:'response',status:200,body:{access_token:'new-access',token_type:'Bearer',expires_in:3600}},exchange.state,{previous}).kind,'reject');
});
for(const handler of ['codex','siwc'])runtimeTest(`${handler} uses production private translation`,async()=>{
  const manifest=JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  const declaration=manifest.extensions['dev.sailry.platform'].model_auth.find(item=>item.flow.handler===handler);
  let state=null;
  const call=async event=>{
    const input={event,state,context:{...context,redirect:handler==='codex'?'http://localhost:1455/auth/callback':context.redirect},now_ms:now};
    const report=translate(await translationInput(new URL('../',import.meta.url),declaration.flow.script,handler,input));
    assert.deepEqual(report.calls,[]);
    assert.deepEqual(report.result.Ok,(handler==='codex'?codex:siwc)(input));
    const output=report.result.Ok;state=output.state;
    return output;
  };
  assert.equal((await call({kind:'start'})).kind,'prompt');
  if(handler==='codex'){
    await call({kind:'presented'});
    await call({kind:'callback',fields:{code:'private-code'}});
  }else{
    await call({kind:'presented'});
    assert.equal((await call({kind:'callback',fields:{code:'private-code',client_id:'oaiapp_issued'}})).kind,'registered');
    await call({kind:'registered'});
  }
  const exchanged=await call({kind:'response',status:200,body:{...token,scope:'openid profile'}});
  assert.equal(exchanged.kind,handler==='codex'?'finish':'verify');
  const finished=handler==='codex'?exchanged:await call({kind:'identity',claims:identity});
  assert.equal(finished.kind,'finish');
  assert.equal(finished.grant.kind,declaration.authentication);
});

runtimeTest('Codex metadata keeps production credential boundaries',async()=>{
  const manifest=JSON.parse(await readFile(new URL('../plugin.json',import.meta.url),'utf8'));
  const declaration=manifest.extensions['dev.sailry.platform'].model_auth.find(item=>item.id==='codex');
  const run=async (body,previous=null)=>{
    const input={event:{kind:'response',status:200,body},state:{phase:'tokens',refreshing:previous!==null},context:{...context,redirect:'http://localhost:1455/auth/callback',previous},now_ms:now};
    const report=translate(await translationInput(new URL('../',import.meta.url),declaration.flow.script,'codex',input));
    assert.deepEqual(report.calls,[]);
    return report.result.Ok;
  };
  const result=await run(token);
  assert.equal(result.kind,'finish');
  assert.equal(result.grant.id_token,token.id_token);
  for(const email of [undefined,'   ']){
    const optional=await run({...token,id_token:jwt({...identity,email,name:'Display name'})});
    assert.equal(optional.kind,'finish');assert.equal(optional.grant.email,null);
  }
  assert.equal((await run({...token,access_token:'rotated-access'},result.grant)).kind,'finish');
  for(const body of [{...token,id_token:'broken'}, {...token,id_token:jwt({...identity,sub:undefined})}, {...token,id_token:jwt({...identity,chatgpt_account_id:'other'})}, {...token,expires_in:0}])assert.equal((await run(body,result.grant)).kind,'reject');
});

test('replaces accounts only through interactive sign in',()=>{
  const previous=codexTokens().grant;
  const claims={...identity,sub:'new-subject',chatgpt_account_id:'new-account',name:'New account',email:'new@example.test'};
  const body={...token,access_token:'new-access',refresh_token:'new-refresh',id_token:jwt(claims)};
  const result=codexTokens(body,{previous});
  assert.equal(result.kind,'finish');assert.equal(result.grant.subject,claims.sub);assert.equal(result.grant.account_id,claims.chatgpt_account_id);
  assert.equal(result.grant.refresh,'new-refresh');assert.equal(result.grant.email,'new@example.test');
  for(const field of ['refresh_token','id_token'])assert.equal(codexTokens({...body,[field]:undefined},{previous}).kind,'reject');
  const verified=step(siwc,{kind:'identity',claims},{phase:'tokens',tokens:{...body,scope:'openid'},client_id:'oaiapp_existing'},{previous});
  assert.equal(verified.kind,'finish');assert.equal(verified.grant.subject,claims.sub);
  assert.equal(step(siwc,{kind:'identity',claims},{phase:'tokens',tokens:{...body,scope:'openid'},client_id:'oaiapp_existing',refreshing:true},{previous}).kind,'reject');
});
test('publishes only optional account emails',()=>{
  for(const [fields,email] of [[{name:'Account name',preferred_username:'Username'},identity.email],[{email:undefined,name:'Account name'},null],[{email:'   ',name:'Account name'},null],[{email:'  spaced@example.test  '},'spaced@example.test']]){
    const result=codexTokens({...token,id_token:jwt({...identity,...fields})});
    assert.equal(result.grant.email,email);
    assert.equal('account_label' in result.grant,false);
  }
});


test('binds browser login to fixed callback and PKCE',()=>{
  const redirect='http://localhost:1455/auth/callback',overrides={redirect,scopes:['openid','profile','email','offline_access']};
  const prompt=step(codex,{kind:'start'},null,overrides);
  assert.equal(prompt.kind,'prompt');assert.equal(prompt.user_code,null);
  const url=new URL(prompt.url);
  assert.equal(url.origin+url.pathname,'https://auth.openai.com/oauth/authorize');
  assert.equal(url.searchParams.get('redirect_uri'),redirect);
  assert.equal(url.searchParams.get('state'),context.proof.state);
  assert.equal(url.searchParams.get('code_challenge'),context.proof.challenge);
  assert.equal(url.searchParams.get('code_challenge_method'),'S256');
  assert.equal(url.searchParams.get('originator'),'codex_cli_rs');
  assert(!prompt.url.includes(context.proof.verifier));
  const waiting=step(codex,{kind:'presented'},prompt.state,overrides);
  assert.equal(waiting.kind,'callback');assert.equal(waiting.proof,'state');assert.equal(waiting.allow_code,false);
  const exchange=step(codex,{kind:'callback',fields:{code:'private-code'}},waiting.state,overrides);
  assert.equal(exchange.name,'token');assert.equal(exchange.fields.redirect_uri,redirect);
  assert.equal(exchange.fields.verifier,context.proof.verifier);
  const result=step(codex,{kind:'response',status:200,body:token},exchange.state,overrides);
  assert.deepEqual(result,codexTokens());
  assert.equal(step(codex,{kind:'start'},null,{redirect:context.redirect}).kind,'reject');
});

test('refreshes browser credentials without a listener',()=>{
  const previous=codexTokens().grant,overrides={previous,redirect:null};
  const refresh=step(codex,{kind:'refresh'},null,overrides);
  assert.equal(refresh.name,'refresh');
  assert.equal(refresh.fields.refresh,previous.refresh);
  const finished=step(codex,{kind:'response',status:200,body:{access_token:'rotated',expires_in:3600}},refresh.state,overrides);
  assert.equal(finished.kind,'finish');assert.equal(finished.grant.refresh,previous.refresh);
  assert.equal(finished.grant.id_token,previous.id_token);
});

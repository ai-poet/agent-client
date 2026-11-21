// OAuth contract aligned with oh-my-pi c29f7c56f5530d805bb2887ade964aa55970d8e1.
// Copyright (c) 2025 Mario Zechner; 2025-2026 Can Bölük; 2026 Stencil Labs, Inc.
// MIT license: LICENSE.pi.
const invalid=()=>({kind:'reject',code:'invalid_request'});
const text=value=>typeof value==='string'&&value.length>0&&value.length<=16384;
const request=(name,fields,state)=>({kind:'request',name,fields,state});
const query=(base,fields)=>`${base}?${Object.entries(fields).map(([key,value])=>`${encodeURIComponent(key)}=${encodeURIComponent(value)}`).join('&')}`;
const finish=grant=>({kind:'finish',grant});
const bootstrapAgent=context=>`claude-code/${/^claude-cli\/([^\s]+)/.exec(context.fields?.user_agent??'')?.[1]??'2.1.280'}`;
const identity=(body,previous,refreshing)=>({
  account_id:text(body?.account?.uuid)?body.account.uuid:previous?.account_id??null,
  email:text(body?.account?.email_address)?body.account.email_address:previous?.email??null,
  org_id:refreshing?previous?.org_id??null:text(body?.organization?.uuid)?body.organization.uuid:null,
  org_name:refreshing?previous?.org_name??null:text(body?.organization?.name)?body.organization.name:null,
});
export function authorize({event,context,state,now_ms:now}) {
  const CLIENT=context.fields?.client_id;
  if (!text(CLIENT)) return invalid();
  if(event.kind==='refresh')return request('refresh',{refresh:context.previous?.refresh},{phase:'tokens',refreshing:true});
  if(event.kind==='start'){
    if(!text(context.redirect))return invalid();
    const fields={code:'true',client_id:CLIENT,response_type:'code',redirect_uri:context.redirect,scope:context.scopes.join(' '),code_challenge_method:'S256',code_challenge:context.proof.challenge,state:context.proof.state};
    return {kind:'prompt',url:query('https://claude.ai/oauth/authorize',fields),user_code:null,expires_at_ms:now+900000,state:{phase:'callback'}};
  }
  if(state?.phase==='identity'){
    const grant=state.grant;
    const account=event.kind==='response'&&event.status>=200&&event.status<300?event.body?.oauth_account:null;
    if(account){
      if(!grant.account_id&&text(account.account_uuid))grant.account_id=account.account_uuid;
      if(!grant.email&&text(account.account_email))grant.email=account.account_email;
      if(!state.refreshing){
        if(!grant.org_id&&text(account.organization_uuid))grant.org_id=account.organization_uuid;
        if(!grant.org_name&&text(account.organization_name))grant.org_name=account.organization_name;
      }
    }
    return finish(grant);
  }
  if(event.kind==='response'&&(event.status<200||event.status>=300))return {kind:'reject',code:[400,401,403].includes(event.status)?'permission_denied':'invalid_request'};
  switch(state?.phase){
    case 'callback':return {kind:'callback',proof:'state',allow_code:true,state:{phase:'returned'}};
    case 'returned':return request('token',{code:event.fields.code,state:context.proof.state,redirect_uri:context.redirect,verifier:context.proof.verifier},{phase:'tokens',refreshing:false});
    case 'tokens':{
      const body=event.body;
      if(!text(body?.access_token)||!text(body?.refresh_token)||!Number.isSafeInteger(body?.expires_in)||body.expires_in<0||!Number.isSafeInteger(now+body.expires_in*1000))return invalid();
      const refreshing=state.refreshing===true;
      const grant={kind:'claude_oauth',access:body.access_token,refresh:body.refresh_token,expires_at_ms:now+body.expires_in*1000,renewal_pending:false,installation_id:context.previous?.installation_id??context.host_id,...identity(body,refreshing?context.previous:null,refreshing)};
      if(grant.account_id&&grant.email&&(refreshing||grant.org_id))return finish(grant);
      return {...request('identity',{access:grant.access,bootstrap_agent:bootstrapAgent(context)},{phase:'identity',grant,refreshing}),optional:true};
    }
    default:return invalid();
  }
}

// Existing Sailry SIWC and Codex protocols, executed by the shared private auth host.
const RESOURCE = 'https://api.openai.com/v1';
const invalid = () => ({kind:'reject',code:'invalid_request'});
const denied = () => ({kind:'reject',code:'permission_denied'});
const text = value => typeof value === 'string' && value.length > 0 && value.length <= 16384 && !/[\x00-\x1f\x7f-\x9f]/.test(value);
const client = value => typeof value === 'string' && value.length <= 256 && /^[A-Za-z0-9_-]+$/.test(value) && value !== 'dynamic_agent_client';
const request = (name,fields,state) => ({kind:'request',name,fields,state});
const expires = (now,seconds) => Number.isSafeInteger(seconds) && seconds > 0 && Number.isSafeInteger(now+seconds*1000) ? now+seconds*1000 : null;
const query = (base,fields) => `${base}?${Object.entries(fields).map(([key,value])=>`${encodeURIComponent(key)}=${encodeURIComponent(value)}`).join('&')}`;
function error(event) {
  if (event.kind !== 'response' || event.status >= 200 && event.status < 300) return null;
  return event.status === 410 ? {kind:'reject',code:'expired'} : [400,401,403].includes(event.status) ? denied() : invalid();
}
function refreshHint(value) {
  if (value == null) return null;
  if (!Number.isSafeInteger(value) || value < 0) return undefined;
  const milliseconds = value < 100000000000 ? value * 1000 : value;
  return Number.isSafeInteger(milliseconds) ? milliseconds : undefined;
}
function finish(kind,tokens,claims,context,now,selectedClient,refreshing=false) {
  const previous=refreshing ? context.previous : null;
  const expiry=expires(now,tokens.expires_in);
  if (!text(tokens.access_token) || expiry === null || !text(claims.sub)) return invalid();
  if (tokens.token_type != null && (typeof tokens.token_type !== 'string' || tokens.token_type.toLowerCase() !== 'bearer')) return invalid();
  const refresh=tokens.refresh_token ?? previous?.refresh;
  const identity=tokens.id_token ?? previous?.id_token;
  if (!text(refresh) || !text(identity)) return invalid();
  if (claims.chatgpt_account_id != null && typeof claims.chatgpt_account_id !== 'string' || claims['https://api.openai.com/auth'] != null && (typeof claims['https://api.openai.com/auth'] !== 'object' || Array.isArray(claims['https://api.openai.com/auth'])) || claims['https://api.openai.com/auth']?.chatgpt_account_id != null && typeof claims['https://api.openai.com/auth'].chatgpt_account_id !== 'string') return invalid();
  const account=claims.chatgpt_account_id ?? claims['https://api.openai.com/auth']?.chatgpt_account_id ?? null;
  const email=text(claims.email) && claims.email.length <= 1024 && claims.email.trim() ? claims.email.trim() : null;
  const grant={kind,access:tokens.access_token,refresh,expires_at_ms:expiry,subject:claims.sub,email,id_token:identity,renewal_pending:false};
  if (kind === 'chat_gpt_oauth') {
    if (!text(account)) return invalid();
    grant.account_id=account;
    if (previous && (previous.subject !== grant.subject || previous.account_id !== account)) return denied();
  } else {
    if (typeof tokens.token_type !== 'string' || tokens.token_type.toLowerCase() !== 'bearer' || !text(tokens.refresh_token) || !client(selectedClient)) return invalid();
    if (tokens.scope != null && typeof tokens.scope !== 'string') return invalid();
    const scopes=typeof tokens.scope === 'string' ? [...new Set(tokens.scope.split(/[\t\n\f\r ]+/).filter(Boolean))].sort() : previous?.scopes;
    if (!Array.isArray(scopes)) return invalid();
    const earliest=refreshHint(tokens.earliest_refresh_at);
    if (earliest === undefined) return invalid();
    Object.assign(grant,{client_id:selectedClient,scopes,earliest_refresh_at:earliest});
    if (account !== null || previous?.account_id != null) grant.account_id=account ?? previous.account_id;
    if (previous && previous.subject !== grant.subject) return denied();
  }
  return {kind:'finish',grant};
}
function verified(kind,event,state,context,now) {
  if (event.kind === 'identity') return finish(kind,state.tokens,event.claims,context,now,state.client_id,state.refreshing===true);
  const tokens=event.body;
  if (!tokens || typeof tokens !== 'object' || Array.isArray(tokens)) return invalid();
  if (text(tokens.id_token)) return {kind:'verify',token:tokens.id_token,audience:state.client_id,nonce:state.nonce ?? null,state:{...state,tokens}};
  if (!context.previous || !state.refreshing) return invalid();
  return finish(kind,tokens,{sub:context.previous.subject,email:context.previous.email,chatgpt_account_id:context.previous.account_id},context,now,state.client_id,state.refreshing===true);
}
// Codex tokens come from the declared TLS/PKCE exchange. Claims are metadata,
// not authority: they never select an issuer, client, endpoint or permission.
function metadata(token) {
  if (!text(token)) return null;
  const parts=token.split('.');
  if (parts.length !== 3 || parts.some(part=>!part || !/^[A-Za-z0-9_-]+$/.test(part))) return null;
  const payload=parts[1];
  if (payload.length % 4 === 1) return null;
  const alphabet='ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';
  let bits=0,buffer=0,encoded='';
  for (const character of payload) {
    buffer=(buffer<<6)|alphabet.indexOf(character);
    bits+=6;
    if (bits>=8) {
      bits-=8;
      encoded+=`%${((buffer>>bits)&255).toString(16).padStart(2,'0')}`;
      buffer&=(1<<bits)-1;
    }
  }
  if (buffer !== 0) return null;
  try {
    const claims=JSON.parse(decodeURIComponent(encoded));
    return claims && typeof claims === 'object' && !Array.isArray(claims) ? claims : null;
  } catch { return null; }
}
function codexTokens(event,state,context,now) {
  if (event.kind !== 'response') return invalid();
  const tokens=event.body;
  if (!tokens || typeof tokens !== 'object' || Array.isArray(tokens)) return invalid();
  if (tokens.id_token != null) {
    const claims=metadata(tokens.id_token);
    return claims ? finish('chat_gpt_oauth',tokens,claims,context,now,state.client_id,state.refreshing===true) : invalid();
  }
  if (!context.previous || !state.refreshing) return invalid();
  return finish('chat_gpt_oauth',tokens,{sub:context.previous.subject,email:context.previous.email,chatgpt_account_id:context.previous.account_id},context,now,state.client_id,state.refreshing===true);
}
export function siwc({event,context,state,now_ms:now}) {
  if (event.kind === 'refresh') {
    const selected=context.previous?.client_id;
    return client(selected) ? request('refresh',{client_id:selected,refresh:context.previous.refresh,resource:RESOURCE},{phase:'tokens',client_id:selected,refreshing:true}) : invalid();
  }
  if (event.kind === 'start') {
    const selected=context.previous?.client_id ?? context.registration ?? null;
    if (selected !== null && !client(selected) || !text(context.host_id) || !text(context.redirect)) return invalid();
    const fields={client_id:selected ?? 'dynamic_agent_client',ext_agent_host_id:context.host_id,redirect_uri:context.redirect,response_type:'code',scope:context.scopes.join(' '),resource:RESOURCE,state:context.proof.state,nonce:context.proof.nonce,code_challenge_method:'S256',code_challenge:context.proof.challenge};
    if (selected === null) fields.agent_name_hint='Sailry';
    return {kind:'prompt',url:query('https://auth.openai.com/api/accounts/authorize',fields),user_code:null,expires_at_ms:now+900000,state:{phase:'callback',client_id:selected}};
  }
  const failed=error(event); if (failed) return failed;
  switch (state?.phase) {
    case 'callback': return {kind:'callback',proof:'state',allow_code:false,state:{...state,phase:'returned'}};
    case 'returned': {
      const selected=event.fields.client_id ?? state.client_id;
      if (!client(selected) || state.client_id !== null && selected !== state.client_id) return invalid();
      return {kind:'registered',client_id:selected,state:{phase:'registered',client_id:selected,code:event.fields.code}};
    }
    case 'registered': return request('token',{client_id:state.client_id,redirect_uri:context.redirect,resource:RESOURCE,code:state.code,verifier:context.proof.verifier},{phase:'tokens',client_id:state.client_id,nonce:context.proof.nonce});
    case 'tokens': return verified('chat_gpt',event,state,context,now);
    default: return invalid();
  }
}

// Reference: yetone/magpie e8d42c02, internal/provider/signin.go.
// The public Codex client requires this exact loopback URI.
export function codex({event,context,state,now_ms:now}) {
  const CLIENT=context.fields?.client_id;
  if (!client(CLIENT)) return invalid();
  if (event.kind === 'refresh') return request('refresh',{client_id:CLIENT,refresh:context.previous?.refresh},{phase:'tokens',client_id:CLIENT,refreshing:true});
  if (!state?.refreshing && context.redirect !== 'http://localhost:1455/auth/callback') return invalid();
  if (event.kind === 'start') return {
    kind:'prompt',url:query('https://auth.openai.com/oauth/authorize',{
      response_type:'code',client_id:CLIENT,redirect_uri:context.redirect,
      scope:context.scopes.join(' '),code_challenge:context.proof.challenge,
      code_challenge_method:'S256',state:context.proof.state,
      id_token_add_organizations:'true',codex_cli_simplified_flow:'true',originator:'codex_cli_rs'
    }),user_code:null,expires_at_ms:now+900000,state:{phase:'callback',client_id:CLIENT}
  };
  const failed=error(event); if (failed) return failed;
  switch (state?.phase) {
    case 'callback': return {kind:'callback',proof:'state',allow_code:false,state:{...state,phase:'returned'}};
    case 'returned':
      if (event.kind !== 'callback' || !text(event.fields?.code)) return invalid();
      return request('token',{client_id:CLIENT,redirect_uri:context.redirect,code:event.fields.code,verifier:context.proof.verifier},{phase:'tokens',client_id:CLIENT});
    case 'tokens': return codexTokens(event,state,context,now);
    default: return invalid();
  }
}

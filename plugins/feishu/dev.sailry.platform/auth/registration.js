// Official Feishu registration protocol: oapi-sdk-python cc359bb7, scene/registration.
// Pure steps run privately in the Node; no SDK, process or network APIs are available.
function text(value) {
  return typeof value === 'string' && value.length > 0 && value.length <= 4096 && !/[\x00-\x1f\x7f]/.test(value);
}
function failure(code = 'outcome_unknown') { return {kind:'reject',code}; }
function request(name, state, fields = {}) { return {kind:'request',name,state,fields}; }

function encoded(value) {
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';
  let output = '';
  for (let index = 0; index < value.length; index += 3) {
    const group = (value[index] << 16) | ((value[index + 1] ?? 0) << 8) | (value[index + 2] ?? 0);
    output += alphabet[(group >>> 18) & 63] + alphabet[(group >>> 12) & 63];
    if (index + 1 < value.length) output += alphabet[(group >>> 6) & 63];
    if (index + 2 < value.length) output += alphabet[group & 63];
  }
  return output;
}

// A single uncompressed DEFLATE block keeps this bounded JSON encoder dependency-free.
function addons(scopes) {
  const source = encodeURIComponent(JSON.stringify({preset:false,scopes:{tenant:scopes}}));
  const bytes = [];
  for (let index = 0; index < source.length; index++) {
    if (source[index] === '%') { bytes.push(parseInt(source.slice(index + 1,index + 3),16)); index += 2; }
    else bytes.push(source.charCodeAt(index));
  }
  if (bytes.length > 65535) throw new Error('Registration options exceed the byte limit');
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit++) crc = (crc >>> 1) ^ ((crc & 1) ? 0xedb88320 : 0);
  }
  crc = (crc ^ 0xffffffff) >>> 0;
  const length = bytes.length, inverse = (~length) & 65535;
  const output = [31,139,8,0,0,0,0,0,0,3,1,length & 255,length >>> 8,inverse & 255,inverse >>> 8,...bytes];
  for (const value of [crc,length]) for (let shift = 0; shift < 32; shift += 8) output.push((value >>> shift) & 255);
  return encoded(output);
}

function verification(value, scopes) {
  if (!text(value)) return null;
  const match = /^(https:\/\/(?:accounts\.feishu\.cn|accounts\.larksuite\.com))(\/[^?#]*)?(?:\?([^#]*))?$/.exec(value);
  if (!match) return null;
  const replaced = new Set(['from','tp','source','addons','createOnly','clientID','name','desc','avatar']);
  const pairs = (match[3] ?? '').split('&').filter(Boolean).filter(pair => !replaced.has(decodeURIComponent(pair.split('=')[0].replace(/\+/g,' '))));
  for (const [key,data] of Object.entries({from:'sdk',tp:'sdk',source:'sailry',name:'Sailry',addons:addons(scopes),createOnly:'true'})) pairs.push(`${key}=${encodeURIComponent(data)}`);
  const url = `${match[1]}${match[2] ?? '/'}?${pairs.join('&')}`;
  return url.length <= 8192 ? url : null;
}

export function authorize({context,event,state,now_ms}) {
  if (event.kind === 'start') return request('init',{phase:'init'});
  if (!state || (state.expires && now_ms >= state.expires)) return failure('expired');
  if (state.phase === 'init') {
    if (event.status !== 200 || !event.body?.supported_auth_methods?.includes('client_secret')) return failure();
    return request('begin',{phase:'begin'});
  }
  if (state.phase === 'begin') {
    const body = event.body ?? {}, interval = body.interval ?? 5, expires = body.expires_in ?? 600;
    const url = verification(body.verification_uri_complete,context.scopes);
    if (event.status !== 200 || !text(body.device_code) || !url || !Number.isInteger(interval) || interval < 1 || interval > 60 || !Number.isInteger(expires) || expires < 1 || expires > 900) return failure();
    return {kind:'prompt',url,user_code:null,expires_at_ms:now_ms + expires * 1000,
      state:{phase:'presented',code:body.device_code,interval:interval * 1000,expires:now_ms + expires * 1000}};
  }
  if (state.phase === 'presented' || state.phase === 'waiting') return request('poll',{...state,phase:'poll'},{device_code:state.code});
  if (state.phase !== 'poll' || ![200,400].includes(event.status)) return failure();
  const body = event.body ?? {};
  if (body.user_info?.tenant_brand && body.user_info.tenant_brand !== 'feishu') return failure('not_configured');
  if (text(body.client_id) && body.client_id.startsWith('cli_') && body.client_id.length <= 256 && text(body.client_secret)) return {kind:'finish',grant:{client_id:body.client_id,client_secret:body.client_secret}};
  if (body.error === 'access_denied') return failure('permission_denied');
  if (body.error === 'expired_token') return failure('expired');
  if (!['authorization_pending','slow_down'].includes(body.error)) return failure();
  const interval = body.error === 'slow_down' ? Math.min(120000,state.interval + 5000) : state.interval;
  return {kind:'wait',delay_ms:interval,state:{...state,phase:'waiting',interval}};
}

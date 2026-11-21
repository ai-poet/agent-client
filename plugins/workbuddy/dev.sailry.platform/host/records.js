import {getValue,setValue,completeRequest,forgetRequest,prepareNotification,prepareTransaction} from 'sailry/sdk';

export function channel(provider) {
  return provider?.preset?.id === 'china'
    && provider.preset.source?.kind === 'package'
    && provider.preset.source.data?.name === 'workbuddy';
}

export function fault(code,message) {return Object.assign(new Error(message),{code});}

export async function complete(id) {
  const result = await completeRequest(id);
  forgetRequest(id);
  if (result.Err) throw fault(result.Err.code,result.Err.message);
  return result.Ok.data;
}

export async function read(key) {return getValue(key);}
export async function write(key,value,revision) {
  return complete(setValue(key,value,revision));
}

export async function notify(title,message,kind) {
  return complete(prepareNotification({title,message,kind,session:null}));
}

export async function report(key,value,revision,title,message,kind) {
  const outputs = await complete(prepareTransaction([
    {kind:'write',data:{key,value,expected_revision:revision}},
    {kind:'notify',data:{title,message,kind,session:null}},
  ]));
  return outputs[0].data;
}

import {callId, isValueConflict} from 'sailry/sdk';
import {fail, result} from './policy.js';
import {committed, identity} from './records.js';
import {prepare as scopeRequest} from './scope.js';
import {prepare as findingRequest} from './findings.js';
import {prepare as completionRequest, display} from './complete.js';
import {prepare as delegationRequest} from './delegation.js';
import {read as content} from './content.js';
export {initialize} from './delegation.js';

function flow(prepare, present = value => value) {
  return async ({arguments:input,step,state,outcome}) => {
    try {
      if (!Number.isInteger(step) || step < 0 || step > 8) fail('Invalid review continuation');
      if (step > 0 && (!isValueConflict(outcome) || step === 8)) {
        return outcome?.isError ? {result:outcome} : {
          result:present(committed(outcome,state)),output:step-1,
        };
      }
      const intent = step === 0 ? callId() : state?.intent;
      if (typeof intent !== 'string') fail('Review tool identity is unavailable','internal');
      const pending = await prepare(input,await identity(),intent);
      return {...pending,state:{...pending.state,intent}};
    } catch (error) { return {result:result(error)}; }
  };
}
export async function scope(input) {
  try { return await scopeRequest(input,await identity()); }
  catch (error) { return result(error); }
}
export const finding = flow(findingRequest);
export const complete = flow(completionRequest,display);

export async function delegate(input) {
  try { return await delegationRequest(input); }
  catch (error) { return result(error); }
}

export async function read(input) {
  try { return await content(input); }
  catch (error) { return result(error); }
}

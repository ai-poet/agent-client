// Tool descriptions, schemas and approval groups are adapted from Sailry c4bb60a2:
// plugins/builtin/browser/agent.rs. Native browser execution remains in the core.
import {captions} from './locales.js';

function prepare(action, args) {
  if (!args || typeof args !== 'object' || Array.isArray(args)) {
    return {isError:true,error:{code:'invalid_request',message:'browser arguments must be an object'}};
  }
  if (action === 'open' || action === 'navigate') {
    if (args.background !== undefined && typeof args.background !== 'boolean') {
      return {isError:true,error:{code:'invalid_request',message:'browser background must be a boolean'}};
    }
    return {action,arguments:{...args,background:args.background ?? false}};
  }
  if (action === 'wait') {
    const {condition, timeout_ms} = args;
    const query = condition?.kind === 'text' ? condition.text
      : ['visible', 'hidden'].includes(condition?.kind) ? condition.selector : null;
    if (!Number.isSafeInteger(timeout_ms) || timeout_ms < 1 || timeout_ms > 0xffffffff ||
        typeof query !== 'string' || !query.length) {
      return {isError:true,error:{code:'invalid_request',message:'invalid browser wait condition or timeout'}};
    }
  }
  return {action, arguments:args};
}

export function tabs(args) { return prepare('tabs', args); }
export function read(args) { return prepare('read', args); }
export function navigate(args) { return prepare('navigate', args); }
export function click(args) { return prepare('click', args); }
export function input(args) { return prepare('input', args); }
export function scroll(args) { return prepare('scroll', args); }
export function back(args) { return prepare('back', args); }
export function forward(args) { return prepare('forward', args); }
export function refresh(args) { return prepare('refresh', args); }
export function open(args) { return prepare('open', args); }
export function close(args) { return prepare('close', args); }
export function focus(args) { return prepare('focus', args); }
export function select(args) { return prepare('select', args); }
export function hover(args) { return prepare('hover', args); }
export function key(args) { return prepare('key', args); }
export function frame(args) { return prepare('frame', args); }
export function wait(args) { return prepare('wait', args); }
export function screenshot(args) { return prepare('screenshot', args); }

export function result({output}) {
  const message = captions[output.error?.message];
  if (message) return {sailry_content:{version:1,blocks:[{kind:'notice',message}]}};
  if (output.isError) return output;
  return output.data;
}

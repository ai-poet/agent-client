import {readComputerPermissions, requestComputerPermission} from 'sailry/sdk';

function status(value) {
  return {
    grants: [
      {kind: 'accessibility', authorized: value.accessibility ?? null},
      {kind: 'screen_recording', authorized: value.screen_capture ?? null},
    ],
    can_request: value.local && value.platform === 'macos',
  };
}

export async function check() {
  return status(await readComputerPermissions());
}

export async function request({kind}) {
  const before = await check();
  const grant = before.grants.find(grant => grant.kind === kind);
  if (!grant) throw new Error('Unknown permission');
  if (!before.can_request || grant.authorized === true) return before;
  // One request prompts for every missing grant and returns both current grants.
  const value = await requestComputerPermission(kind === 'screen_recording' ? 'screen_capture' : 'accessibility');
  return value === null ? before : status(value);
}

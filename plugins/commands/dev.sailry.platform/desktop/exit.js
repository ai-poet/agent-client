/** Any process exit ends the terminal item; only failures are reported. */
export function action(status) {
  if (status?.kind === 'failed') return {close:true, toast:true};
  if (status?.kind === 'exited') return {close:true, toast:status.data?.code !== 0};
  return {close:false, toast:false};
}

export function message(status, text) {
  return status.kind === 'exited' ? text.exited.replace('{code}', String(status.data?.code ?? '—')) : text.ended;
}

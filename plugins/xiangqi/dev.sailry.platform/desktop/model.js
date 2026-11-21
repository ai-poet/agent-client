export const retryable = ["failed", "timedOut", "invalidResponse", "unconfirmed"];
// Game prompts and offered-move validation are package-owned.
export function command(player, turn) {
  if (player.kind !== 'provider' || typeof player.model !== 'string') throw new Error('configure');
  if (typeof turn.instructions !== 'string' || !Array.isArray(turn.choices)) throw new Error('invalidResponse');
  return {kind:'generate_plugin_text',data:{model:player.model,effort:player.effort,
    prompt:turn.instructions+' Return ONLY {"move":id} using one offered integer id. No explanation.\n'
      +JSON.stringify({state:turn.state,choices:turn.choices})}};
}
export function select(result, player, moves) {
  if (result.Err?.message === 'plugin model request timed out') throw new Error('timedOut');
  if (result.Err) throw new Error(result.Err.code === 'not_configured' ? 'configure'
    : result.Err.code === 'outcome_unknown' ? 'unconfirmed' : 'failed');
  if (player.kind !== 'provider' || result.Ok?.kind !== 'plugin_text') throw new Error('invalidResponse');
  let reply;
  try { reply = JSON.parse(result.Ok.data.text.trim().replace(/^```(?:json)?\s*/, '').replace(/\s*```$/, '')); }
  catch (_) { throw new Error('invalidResponse'); }
  if (!Number.isSafeInteger(reply?.move) || reply.move < 0 || reply.move >= moves.length) throw new Error('invalidResponse');
  return {move:moves[reply.move],tokens:result.Ok.data.tokens??0};
}

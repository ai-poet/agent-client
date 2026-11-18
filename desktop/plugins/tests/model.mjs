// Package-owned model parsing with request-recovery fixtures.
export const retryable = ["failed", "timedOut", "invalidResponse", "unconfirmed"];

export {command,select} from "../gomoku/dev.sailry.platform/desktop/model.js";

export function failure(fault) {
  if (fault?.code === "not_configured") return "configure";
  if (fault?.message === "plugin model request timed out") return "timedOut";
  return "failed";
}

export async function completion(id, execute, outcome, sleep) {
  const result = JSON.parse(await execute(id));
  if (!result.Err) return result;
  while (true) {
    const receipt = JSON.parse(await outcome(id));
    if (receipt.Ok?.kind === "completed") return receipt.Ok.data;
    if (receipt.Ok?.kind !== "admitted") break;
    await sleep(1000);
  }
  throw new Error("unconfirmed");
}

export function errorCode(message) {
  const code = message?.replace(/^`sailry\/sdk\.[A-Za-z]+`: /, "");
  return ["configure", "failed", "invalidResponse", "unconfirmed", "timedOut"].includes(code) ? code : "failed";
}

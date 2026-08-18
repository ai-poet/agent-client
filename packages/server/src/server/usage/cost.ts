import type { CostSemantics } from "./types.js";

/**
 * Providers that report a running session total rather than a per-turn cost. Summing
 * their raw readings across turns would multiply-count every earlier turn.
 */
const CUMULATIVE_COST_PROVIDERS = new Set(["claude", "opencode", "pi"]);

export function resolveCostSemantics(provider: string): CostSemantics {
  return CUMULATIVE_COST_PROVIDERS.has(provider) ? "cumulative" : "delta";
}

/**
 * Converts a provider's raw cost reading into the amount attributable to this turn.
 *
 * For cumulative providers this is the increase since the previous reading. A decrease
 * means the session counter reset (a new session on the same agent), in which case the
 * new reading is itself the turn's cost.
 */
export function resolveTurnCostUsd(options: {
  semantics: CostSemantics;
  rawTotalCostUsd: number | undefined;
  previousRawTotalCostUsd: number | undefined;
}): number | undefined {
  const { semantics, rawTotalCostUsd, previousRawTotalCostUsd } = options;
  if (rawTotalCostUsd === undefined || !Number.isFinite(rawTotalCostUsd)) {
    return undefined;
  }
  if (semantics === "delta") {
    return Math.max(0, rawTotalCostUsd);
  }
  if (previousRawTotalCostUsd === undefined || !Number.isFinite(previousRawTotalCostUsd)) {
    return Math.max(0, rawTotalCostUsd);
  }
  if (rawTotalCostUsd < previousRawTotalCostUsd) {
    // Counter went backwards — treat it as a fresh session rather than a negative cost.
    return Math.max(0, rawTotalCostUsd);
  }
  return rawTotalCostUsd - previousRawTotalCostUsd;
}

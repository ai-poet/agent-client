import { z } from "zod";

/**
 * Providers disagree on what `totalCostUsd` means: Claude reports a running session
 * total, OpenCode accumulates internally, and others report just the last turn. Summing
 * the raw value would multiply-count the cumulative ones, so each record stores both the
 * raw reading and the derived per-turn delta.
 */
export type CostSemantics = "cumulative" | "delta";

export const UsageRecordSchema = z.object({
  /** ISO timestamp of turn completion. */
  ts: z.string(),
  agentId: z.string(),
  provider: z.string(),
  /** Working directory, used as the project dimension. */
  cwd: z.string(),
  model: z.string().optional(),
  inputTokens: z.number().optional(),
  cachedInputTokens: z.number().optional(),
  outputTokens: z.number().optional(),
  /** Exactly what the provider reported, kept for auditability. */
  rawTotalCostUsd: z.number().optional(),
  /** Cost attributable to this turn alone. */
  costUsd: z.number().optional(),
  costSemantics: z.union([z.literal("cumulative"), z.literal("delta")]).optional(),
  durationMs: z.number().optional(),
});

export type UsageRecord = z.infer<typeof UsageRecordSchema>;

export type UsageGroupBy = "day" | "provider" | "project" | "model";

export interface UsageTotals {
  turns: number;
  inputTokens: number;
  cachedInputTokens: number;
  outputTokens: number;
  totalTokens: number;
  costUsd: number;
  durationMs: number;
}

export interface UsageBucket extends UsageTotals {
  /** The grouping value: an ISO date, provider id, cwd, or model id. */
  key: string;
}

export interface UsageStats {
  from: string;
  to: string;
  groupBy: UsageGroupBy;
  totals: UsageTotals;
  buckets: UsageBucket[];
}

export function emptyTotals(): UsageTotals {
  return {
    turns: 0,
    inputTokens: 0,
    cachedInputTokens: 0,
    outputTokens: 0,
    totalTokens: 0,
    costUsd: 0,
    durationMs: 0,
  };
}

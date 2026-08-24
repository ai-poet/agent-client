import type { Logger } from "pino";

import type { AgentUsage } from "../agent/agent-sdk-types.js";
import { resolveCostSemantics, resolveTurnCostUsd } from "./cost.js";
import { UsageStore } from "./store.js";
import {
  emptyTotals,
  type UsageBucket,
  type UsageGroupBy,
  type UsageRecord,
  type UsageStats,
  type UsageTotals,
} from "./types.js";

export interface RecordTurnInput {
  agentId: string;
  provider: string;
  cwd: string;
  model?: string;
  usage: AgentUsage | undefined;
  durationMs?: number;
  /** The previous raw cost reading for this agent, used to derive a per-turn delta. */
  previousRawTotalCostUsd?: number;
}

export class UsageService {
  private readonly store: UsageStore;
  private readonly logger: Logger;

  constructor(options: { paseoHome: string; logger: Logger; store?: UsageStore }) {
    this.logger = options.logger;
    this.store =
      options.store ?? new UsageStore({ paseoHome: options.paseoHome, logger: options.logger });
  }

  /** Prunes expired shards. Safe to call on every daemon start. */
  async start(): Promise<void> {
    await this.store.prune();
  }

  /**
   * Records one completed turn. Never throws — usage accounting must not be able to break
   * an agent run.
   */
  async recordTurn(input: RecordTurnInput): Promise<void> {
    try {
      const semantics = resolveCostSemantics(input.provider);
      const rawTotalCostUsd = input.usage?.totalCostUsd;
      const record: UsageRecord = {
        ts: new Date().toISOString(),
        agentId: input.agentId,
        provider: input.provider,
        cwd: input.cwd,
        model: input.model,
        inputTokens: input.usage?.inputTokens,
        cachedInputTokens: input.usage?.cachedInputTokens,
        outputTokens: input.usage?.outputTokens,
        rawTotalCostUsd,
        costUsd: resolveTurnCostUsd({
          semantics,
          rawTotalCostUsd,
          previousRawTotalCostUsd: input.previousRawTotalCostUsd,
        }),
        costSemantics: rawTotalCostUsd === undefined ? undefined : semantics,
        durationMs: input.durationMs,
      };
      await this.store.append(record);
    } catch (error) {
      this.logger.debug({ err: error, agentId: input.agentId }, "Failed to record usage");
    }
  }

  async getStats(options: {
    from: Date;
    to: Date;
    groupBy: UsageGroupBy;
    provider?: string;
    cwd?: string;
  }): Promise<UsageStats> {
    const records = (await this.store.query({ from: options.from, to: options.to })).filter(
      (record) =>
        (!options.provider || record.provider === options.provider) &&
        (!options.cwd || record.cwd === options.cwd),
    );

    return {
      from: options.from.toISOString(),
      to: options.to.toISOString(),
      groupBy: options.groupBy,
      totals: accumulate(records),
      buckets: buildBuckets(records, options.groupBy),
    };
  }
}

function addRecord(totals: UsageTotals, record: UsageRecord): void {
  totals.turns += 1;
  totals.inputTokens += record.inputTokens ?? 0;
  totals.cachedInputTokens += record.cachedInputTokens ?? 0;
  totals.outputTokens += record.outputTokens ?? 0;
  totals.totalTokens += (record.inputTokens ?? 0) + (record.outputTokens ?? 0);
  totals.costUsd += record.costUsd ?? 0;
  totals.durationMs += record.durationMs ?? 0;
}

function accumulate(records: UsageRecord[]): UsageTotals {
  const totals = emptyTotals();
  for (const record of records) {
    addRecord(totals, record);
  }
  return totals;
}

function resolveBucketKey(record: UsageRecord, groupBy: UsageGroupBy): string {
  switch (groupBy) {
    case "day":
      // UTC day so buckets stay stable regardless of the reader's timezone.
      return record.ts.slice(0, 10);
    case "provider":
      return record.provider;
    case "project":
      return record.cwd;
    case "model":
      return record.model ?? "unknown";
  }
}

function buildBuckets(records: UsageRecord[], groupBy: UsageGroupBy): UsageBucket[] {
  const byKey = new Map<string, UsageTotals>();

  for (const record of records) {
    const key = resolveBucketKey(record, groupBy);
    let totals = byKey.get(key);
    if (!totals) {
      totals = emptyTotals();
      byKey.set(key, totals);
    }
    addRecord(totals, record);
  }

  const buckets = Array.from(byKey.entries(), ([key, totals]) => ({ key, ...totals }));
  // Days read best chronologically; every other dimension reads best largest-first.
  return groupBy === "day"
    ? buckets.sort((a, b) => a.key.localeCompare(b.key))
    : buckets.sort((a, b) => b.totalTokens - a.totalTokens || a.key.localeCompare(b.key));
}

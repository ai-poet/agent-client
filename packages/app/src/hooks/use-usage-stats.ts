import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";

import { useHostRuntimeClient } from "@/runtime/host-runtime";
import { useSessionStore } from "@/stores/session-store";
import { rangeDayCount, type UsageRangeKey } from "@/utils/usage-trend";

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
  key: string;
}

export interface UsageStats {
  from: string;
  to: string;
  groupBy: UsageGroupBy;
  totals: UsageTotals;
  buckets: UsageBucket[];
}

/** Resolves a range key to an inclusive UTC window ending now. */
export function resolveUsageRange(
  range: UsageRangeKey,
  now: Date = new Date(),
): {
  from: Date;
  to: Date;
} {
  const to = now;
  const dayCount = rangeDayCount(range);
  if (dayCount === null) {
    // The daemon only retains 90 days, so "all" is bounded by retention anyway.
    return { from: new Date(0), to };
  }
  const from = new Date(now.getTime());
  from.setUTCDate(from.getUTCDate() - (dayCount - 1));
  from.setUTCHours(0, 0, 0, 0);
  return { from, to };
}

interface UseUsageStatsOptions {
  serverId: string;
  range: UsageRangeKey;
  groupBy: UsageGroupBy;
  provider?: string;
  cwd?: string;
  enabled?: boolean;
}

export function useUsageStats(options: UseUsageStatsOptions) {
  const { serverId, range, groupBy, provider, cwd, enabled = true } = options;
  const client = useHostRuntimeClient(serverId);
  const supportsUsageStats = useSessionStore(
    (state) => state.sessions[serverId]?.serverInfo?.features?.usageStats === true,
  );

  // Bucketed to the hour so the window does not change identity on every render.
  const rangeAnchor = useMemo(() => {
    const now = new Date();
    now.setUTCMinutes(0, 0, 0);
    return now.toISOString();
  }, []);

  const query = useQuery({
    queryKey: ["usageStats", serverId, range, groupBy, provider ?? null, cwd ?? null, rangeAnchor],
    enabled: Boolean(enabled && client && supportsUsageStats),
    staleTime: 30_000,
    queryFn: async (): Promise<UsageStats | null> => {
      if (!client) {
        return null;
      }
      const { from, to } = resolveUsageRange(range, new Date(rangeAnchor));
      const payload = await client.getUsageStats({
        from: from.toISOString(),
        to: to.toISOString(),
        groupBy,
        provider,
        cwd,
      });
      if (payload.error) {
        throw new Error(payload.error);
      }
      return payload.stats as UsageStats | null;
    },
  });

  return {
    stats: query.data ?? null,
    isLoading: query.isLoading,
    isFetching: query.isFetching,
    error: query.error instanceof Error ? query.error.message : null,
    supportsUsageStats,
    refetch: query.refetch,
  };
}

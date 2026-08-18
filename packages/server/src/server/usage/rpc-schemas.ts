import { z } from "zod";

export const UsageGroupBySchema = z.union([
  z.literal("day"),
  z.literal("provider"),
  z.literal("project"),
  z.literal("model"),
]);

export const UsageStatsRequestSchema = z.object({
  type: z.literal("usage/stats"),
  requestId: z.string(),
  /** ISO timestamps bounding the query, inclusive. */
  from: z.string(),
  to: z.string(),
  groupBy: UsageGroupBySchema,
  provider: z.string().optional(),
  cwd: z.string().optional(),
});

const UsageTotalsSchema = z.object({
  turns: z.number(),
  inputTokens: z.number(),
  cachedInputTokens: z.number(),
  outputTokens: z.number(),
  totalTokens: z.number(),
  costUsd: z.number(),
  durationMs: z.number(),
});

const UsageBucketSchema = UsageTotalsSchema.extend({
  key: z.string(),
});

export const UsageStatsResponseSchema = z.object({
  type: z.literal("usage/stats/response"),
  payload: z.object({
    requestId: z.string(),
    stats: z
      .object({
        from: z.string(),
        to: z.string(),
        groupBy: UsageGroupBySchema,
        totals: UsageTotalsSchema,
        buckets: z.array(UsageBucketSchema),
      })
      .nullable(),
    error: z.string().nullable(),
  }),
});

export type UsageStatsRequest = z.infer<typeof UsageStatsRequestSchema>;
export type UsageStatsResponse = z.infer<typeof UsageStatsResponseSchema>;

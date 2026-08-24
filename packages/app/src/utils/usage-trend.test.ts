import { describe, expect, it } from "vitest";

import { formatCost, formatDuration, formatTokenValue } from "./usage-format";
import {
  buildTrendPoints,
  findLatestPopulatedIndex,
  resolveBarHeightPercent,
  resolveTrendUnit,
  shouldShowTrendLabel,
  type TrendSourceBucket,
} from "./usage-trend";

function bucket(key: string, totalTokens: number): TrendSourceBucket {
  return { key, totalTokens, costUsd: 0.01, durationMs: 1000, turns: 1 };
}

describe("buildTrendPoints", () => {
  it("zero-fills a fixed range so sparse data keeps a stable axis", () => {
    const { points, unit } = buildTrendPoints({
      buckets: [bucket("2026-03-10", 500)],
      range: "7",
      now: new Date("2026-03-12T08:00:00Z"),
    });

    expect(unit).toBe("day");
    expect(points).toHaveLength(7);
    expect(points[0]?.start).toBe("2026-03-06");
    expect(points[6]?.start).toBe("2026-03-12");
    expect(points[4]?.totalTokens).toBe(500);
    expect(points[6]?.totalTokens).toBe(0);
  });

  it("produces exactly one bucket for the single-day range", () => {
    const { points } = buildTrendPoints({
      buckets: [],
      range: "1",
      now: new Date("2026-03-12T08:00:00Z"),
    });

    expect(points).toHaveLength(1);
    expect(points[0]?.start).toBe("2026-03-12");
  });

  it("ignores data outside a fixed range", () => {
    const { points } = buildTrendPoints({
      buckets: [bucket("2026-01-01", 999)],
      range: "7",
      now: new Date("2026-03-12T08:00:00Z"),
    });

    expect(points.every((point) => point.totalTokens === 0)).toBe(true);
  });

  it("uses day buckets for a short all-time span", () => {
    const { points, unit } = buildTrendPoints({
      buckets: [bucket("2026-03-01", 10), bucket("2026-03-03", 20)],
      range: "all",
      now: new Date("2026-03-12T08:00:00Z"),
    });

    expect(unit).toBe("day");
    expect(points.map((point) => point.start)).toEqual(["2026-03-01", "2026-03-02", "2026-03-03"]);
    expect(points[1]?.totalTokens).toBe(0);
  });

  it("switches to weekly buckets anchored at the first data day", () => {
    const { points, unit } = buildTrendPoints({
      buckets: [bucket("2026-01-01", 10), bucket("2026-01-09", 20), bucket("2026-05-01", 30)],
      range: "all",
      now: new Date("2026-05-02T00:00:00Z"),
    });

    expect(unit).toBe("week");
    expect(points[0]?.start).toBe("2026-01-01");
    expect(points[0]?.end).toBe("2026-01-07");
    expect(points[0]?.totalTokens).toBe(10);
    expect(points[1]?.totalTokens).toBe(20);
  });

  it("switches to monthly buckets beyond a year", () => {
    const { points, unit } = buildTrendPoints({
      buckets: [bucket("2024-01-15", 10), bucket("2026-02-20", 20)],
      range: "all",
      now: new Date("2026-03-01T00:00:00Z"),
    });

    expect(unit).toBe("month");
    expect(points[0]?.start).toBe("2024-01-01");
    expect(points[0]?.end).toBe("2024-01-31");
    expect(points[points.length - 1]?.totalTokens).toBe(20);
  });

  it("returns nothing for an empty all-time range", () => {
    expect(
      buildTrendPoints({ buckets: [], range: "all", now: new Date("2026-03-12T00:00:00Z") }).points,
    ).toEqual([]);
  });

  it("is timezone independent", () => {
    const lateEvening = buildTrendPoints({
      buckets: [bucket("2026-03-12", 5)],
      range: "7",
      now: new Date("2026-03-12T23:30:00Z"),
    });
    const earlyMorning = buildTrendPoints({
      buckets: [bucket("2026-03-12", 5)],
      range: "7",
      now: new Date("2026-03-12T00:30:00Z"),
    });

    expect(lateEvening.points.map((point) => point.start)).toEqual(
      earlyMorning.points.map((point) => point.start),
    );
  });
});

describe("resolveTrendUnit", () => {
  it("always uses days for fixed ranges regardless of span", () => {
    expect(resolveTrendUnit("90", 900)).toBe("day");
  });

  it("scales the unit with the span for all-time", () => {
    expect(resolveTrendUnit("all", 30)).toBe("day");
    expect(resolveTrendUnit("all", 120)).toBe("week");
    expect(resolveTrendUnit("all", 400)).toBe("month");
  });
});

describe("findLatestPopulatedIndex", () => {
  it("selects the last bucket that has data", () => {
    const { points } = buildTrendPoints({
      buckets: [bucket("2026-03-10", 5)],
      range: "7",
      now: new Date("2026-03-12T00:00:00Z"),
    });

    // Without this, a touch device would open the card on an empty trailing day.
    expect(findLatestPopulatedIndex(points)).toBe(4);
  });

  it("falls back to the last bucket when nothing has data", () => {
    const { points } = buildTrendPoints({
      buckets: [],
      range: "7",
      now: new Date("2026-03-12T00:00:00Z"),
    });

    expect(findLatestPopulatedIndex(points)).toBe(6);
  });

  it("returns -1 for an empty series", () => {
    expect(findLatestPopulatedIndex([])).toBe(-1);
  });
});

describe("shouldShowTrendLabel", () => {
  it("always shows the first and last label", () => {
    expect(shouldShowTrendLabel({ index: 0, total: 90, unit: "day" })).toBe(true);
    expect(shouldShowTrendLabel({ index: 89, total: 90, unit: "day" })).toBe(true);
  });

  it("shows every label for short series", () => {
    expect(shouldShowTrendLabel({ index: 3, total: 7, unit: "day" })).toBe(true);
  });

  it("thins long day series", () => {
    expect(shouldShowTrendLabel({ index: 5, total: 30, unit: "day" })).toBe(true);
    expect(shouldShowTrendLabel({ index: 6, total: 30, unit: "day" })).toBe(false);
    expect(shouldShowTrendLabel({ index: 15, total: 90, unit: "day" })).toBe(true);
    expect(shouldShowTrendLabel({ index: 16, total: 90, unit: "day" })).toBe(false);
  });
});

describe("resolveBarHeightPercent", () => {
  it("keeps a tiny nonzero value visible", () => {
    expect(resolveBarHeightPercent(1, 100_000)).toBe(2);
  });

  it("renders nothing for zero", () => {
    expect(resolveBarHeightPercent(0, 100)).toBe(0);
  });

  it("scales proportionally", () => {
    expect(resolveBarHeightPercent(50, 100)).toBe(50);
    expect(resolveBarHeightPercent(100, 100)).toBe(100);
  });
});

describe("usage formatters", () => {
  it("abbreviates token counts and trims trailing zeros", () => {
    expect(formatTokenValue(0)).toBe("0");
    expect(formatTokenValue(950)).toBe("950");
    expect(formatTokenValue(1_000)).toBe("1K");
    expect(formatTokenValue(1_500)).toBe("1.5K");
    expect(formatTokenValue(2_000_000)).toBe("2M");
    expect(formatTokenValue(3_200_000_000)).toBe("3.2B");
  });

  it("formats durations and drops zero sub-units", () => {
    expect(formatDuration(0)).toBe("0s");
    expect(formatDuration(500)).toBe("<1s");
    expect(formatDuration(45_000)).toBe("45s");
    expect(formatDuration(90_000)).toBe("1m 30s");
    expect(formatDuration(120_000)).toBe("2m");
    expect(formatDuration(3_600_000)).toBe("1h");
    expect(formatDuration(5_400_000)).toBe("1h 30m");
    expect(formatDuration(90_000_000)).toBe("1d 1h");
  });

  it("keeps four decimals for nonzero costs", () => {
    expect(formatCost(0)).toBe("$0");
    expect(formatCost(0.0123)).toBe("$0.0123");
    expect(formatCost(12.5)).toBe("$12.5000");
  });
});

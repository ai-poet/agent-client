/**
 * Trend bucketing for the local usage screen. All arithmetic is UTC so buckets do not
 * drift with the reader's timezone or across DST.
 */

export type UsageRangeKey = "1" | "7" | "30" | "90" | "all";
export type TrendUnit = "day" | "week" | "month";

/** Number of days a fixed range covers; `all` has no fixed length. */
export function rangeDayCount(range: UsageRangeKey): number | null {
  return range === "all" ? null : Number.parseInt(range, 10);
}

export interface TrendSourceBucket {
  /** ISO date (YYYY-MM-DD) from the daemon's day grouping. */
  key: string;
  totalTokens: number;
  costUsd: number;
  durationMs: number;
  turns: number;
}

export interface TrendPoint extends TrendSourceBucket {
  /** Start of the bucket, ISO date. */
  start: string;
  /** Inclusive end of the bucket, ISO date. Equals `start` for day buckets. */
  end: string;
}

export function toIsoDate(date: Date): string {
  return date.toISOString().slice(0, 10);
}

function utcFromIsoDate(iso: string): Date {
  const [year, month, day] = iso.split("-").map((part) => Number.parseInt(part, 10));
  return new Date(Date.UTC(year ?? 1970, (month ?? 1) - 1, day ?? 1));
}

function addDays(date: Date, days: number): Date {
  const next = new Date(date.getTime());
  next.setUTCDate(next.getUTCDate() + days);
  return next;
}

function emptyPoint(start: string, end: string): TrendPoint {
  return { key: start, start, end, totalTokens: 0, costUsd: 0, durationMs: 0, turns: 0 };
}

function accumulate(target: TrendPoint, source: TrendSourceBucket): void {
  target.totalTokens += source.totalTokens;
  target.costUsd += source.costUsd;
  target.durationMs += source.durationMs;
  target.turns += source.turns;
}

/** `all` picks a unit from the span so a long history does not render hundreds of bars. */
export function resolveTrendUnit(range: UsageRangeKey, spanDays: number): TrendUnit {
  if (range !== "all") {
    return "day";
  }
  if (spanDays > 365) {
    return "month";
  }
  if (spanDays > 90) {
    return "week";
  }
  return "day";
}

/**
 * Builds the trend series. Fixed ranges always produce exactly `range` day buckets,
 * zero-filled, so a sparse period still renders a stable axis instead of collapsing.
 */
export function buildTrendPoints(options: {
  buckets: TrendSourceBucket[];
  range: UsageRangeKey;
  /** Anchor for fixed ranges — normally "today". */
  now: Date;
}): { points: TrendPoint[]; unit: TrendUnit } {
  const { buckets, range, now } = options;
  const byDate = new Map(buckets.map((bucket) => [bucket.key, bucket]));

  const dayCount = rangeDayCount(range);
  if (dayCount !== null) {
    const points: TrendPoint[] = [];
    const today = utcFromIsoDate(toIsoDate(now));
    for (let offset = dayCount - 1; offset >= 0; offset -= 1) {
      const date = toIsoDate(addDays(today, -offset));
      const point = emptyPoint(date, date);
      const source = byDate.get(date);
      if (source) {
        accumulate(point, source);
      }
      points.push(point);
    }
    return { points, unit: "day" };
  }

  const sorted = [...buckets].sort((a, b) => a.key.localeCompare(b.key));
  const first = sorted[0];
  const last = sorted[sorted.length - 1];
  if (!first || !last) {
    return { points: [], unit: "day" };
  }

  const firstDate = utcFromIsoDate(first.key);
  const lastDate = utcFromIsoDate(last.key);
  const spanDays = Math.round((lastDate.getTime() - firstDate.getTime()) / 86_400_000) + 1;
  const unit = resolveTrendUnit(range, spanDays);

  if (unit === "day") {
    const points: TrendPoint[] = [];
    for (let cursor = firstDate; cursor <= lastDate; cursor = addDays(cursor, 1)) {
      const date = toIsoDate(cursor);
      const point = emptyPoint(date, date);
      const source = byDate.get(date);
      if (source) {
        accumulate(point, source);
      }
      points.push(point);
    }
    return { points, unit };
  }

  if (unit === "week") {
    const points: TrendPoint[] = [];
    // Anchored to the first day with data rather than ISO weeks, so bucket 0 always
    // starts where the history actually starts.
    for (let cursor = firstDate; cursor <= lastDate; cursor = addDays(cursor, 7)) {
      const end = addDays(cursor, 6);
      const point = emptyPoint(toIsoDate(cursor), toIsoDate(end > lastDate ? lastDate : end));
      for (const bucket of sorted) {
        const date = utcFromIsoDate(bucket.key);
        if (date >= cursor && date <= end) {
          accumulate(point, bucket);
        }
      }
      points.push(point);
    }
    return { points, unit };
  }

  const points: TrendPoint[] = [];
  const cursor = new Date(Date.UTC(firstDate.getUTCFullYear(), firstDate.getUTCMonth(), 1));
  const lastMonth = Date.UTC(lastDate.getUTCFullYear(), lastDate.getUTCMonth(), 1);
  while (cursor.getTime() <= lastMonth) {
    const monthStart = new Date(cursor.getTime());
    const monthEnd = new Date(Date.UTC(cursor.getUTCFullYear(), cursor.getUTCMonth() + 1, 0));
    const point = emptyPoint(toIsoDate(monthStart), toIsoDate(monthEnd));
    for (const bucket of sorted) {
      const date = utcFromIsoDate(bucket.key);
      if (date >= monthStart && date <= monthEnd) {
        accumulate(point, bucket);
      }
    }
    points.push(point);
    cursor.setUTCMonth(cursor.getUTCMonth() + 1);
  }
  return { points, unit: "month" };
}

/**
 * The last bucket that has data. Used as the default selection: on touch there is no
 * hover, so the detail card would otherwise start empty.
 */
export function findLatestPopulatedIndex(points: TrendPoint[]): number {
  for (let index = points.length - 1; index >= 0; index -= 1) {
    const point = points[index];
    if (point && (point.turns > 0 || point.totalTokens > 0)) {
      return index;
    }
  }
  return points.length > 0 ? points.length - 1 : -1;
}

/** Keeps x-axis labels from colliding; first and last are always shown. */
export function shouldShowTrendLabel(options: {
  index: number;
  total: number;
  unit: TrendUnit;
}): boolean {
  const { index, total, unit } = options;
  if (total <= 1) {
    return true;
  }
  if (index === 0 || index === total - 1) {
    return true;
  }
  if (unit === "month" || total <= 7) {
    return true;
  }
  if (unit === "week") {
    return index % Math.ceil(total / 6) === 0;
  }
  return total <= 30 ? index % 5 === 0 : index % 15 === 0;
}

/** A 2% floor so a small nonzero bucket never renders as an invisible bar. */
export function resolveBarHeightPercent(value: number, max: number): number {
  if (max <= 0 || value <= 0) {
    return 0;
  }
  return Math.max(2, Math.round((value / max) * 100));
}

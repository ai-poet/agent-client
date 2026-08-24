/**
 * Presentation helpers for the automation screen. The daemon's cadence model is either a
 * fixed interval or a cron expression; the UI offers friendlier presets on top of those.
 */

export type CadenceKind = "interval" | "daily" | "weekdays" | "weekly" | "monthly";

export interface CadenceDraft {
  kind: CadenceKind;
  /** interval only */
  intervalValue: number;
  intervalUnit: "minutes" | "hours";
  /** daily / weekdays / weekly / monthly — 24h clock */
  hour: number;
  minute: number;
  /** weekly only: 0 = Sunday, matching JS getDay(). */
  weekday: number;
  /** monthly only */
  dayOfMonth: number;
}

export type ScheduleCadence =
  | { type: "every"; everyMs: number }
  | { type: "cron"; expression: string };

export const MIN_INTERVAL_MINUTES = 15;
export const MIN_INTERVAL_HOURS = 1;
export const MAX_INTERVAL_HOURS = 168;

export function defaultCadenceDraft(kind: CadenceKind): CadenceDraft {
  return {
    kind,
    intervalValue: kind === "interval" ? 1 : 1,
    intervalUnit: "hours",
    hour: 9,
    minute: 0,
    weekday: 1,
    dayOfMonth: 1,
  };
}

function clampInterval(draft: CadenceDraft): number {
  if (draft.intervalUnit === "minutes") {
    return Math.max(MIN_INTERVAL_MINUTES, Math.round(draft.intervalValue));
  }
  const hours = Math.min(
    MAX_INTERVAL_HOURS,
    Math.max(MIN_INTERVAL_HOURS, Math.round(draft.intervalValue)),
  );
  return hours * 60;
}

/** Converts the UI draft into the cadence the daemon stores. */
export function toCadence(draft: CadenceDraft): ScheduleCadence {
  const minute = Math.min(59, Math.max(0, Math.round(draft.minute)));
  const hour = Math.min(23, Math.max(0, Math.round(draft.hour)));

  switch (draft.kind) {
    case "interval":
      return { type: "every", everyMs: clampInterval(draft) * 60_000 };
    case "daily":
      return { type: "cron", expression: `${minute} ${hour} * * *` };
    case "weekdays":
      return { type: "cron", expression: `${minute} ${hour} * * 1-5` };
    case "weekly":
      return { type: "cron", expression: `${minute} ${hour} * * ${draft.weekday}` };
    case "monthly": {
      const day = Math.min(31, Math.max(1, Math.round(draft.dayOfMonth)));
      return { type: "cron", expression: `${minute} ${hour} ${day} * *` };
    }
  }
}

function pad(value: number): string {
  return String(value).padStart(2, "0");
}

export function formatTimeOfDay(hour: number, minute: number): string {
  return `${pad(hour)}:${pad(minute)}`;
}

export interface ScheduleSummaryText {
  everyMinutes: (minutes: number) => string;
  everyHours: (hours: number) => string;
  daily: (time: string) => string;
  weekdays: (time: string) => string;
  weekly: (day: string, time: string) => string;
  monthly: (day: number, time: string) => string;
  cron: (expression: string) => string;
  weekdayNames: readonly string[];
}

/**
 * Human-readable one-liner for a stored cadence. Recognises the cron shapes the UI
 * produces and falls back to showing the raw expression for anything hand-written.
 */
export function formatCadence(cadence: ScheduleCadence, text: ScheduleSummaryText): string {
  if (cadence.type === "every") {
    const minutes = Math.round(cadence.everyMs / 60_000);
    return minutes % 60 === 0 ? text.everyHours(minutes / 60) : text.everyMinutes(minutes);
  }

  const parts = cadence.expression.trim().split(/\s+/);
  if (parts.length !== 5) {
    return text.cron(cadence.expression);
  }
  const [minuteRaw, hourRaw, domRaw, monthRaw, dowRaw] = parts as [
    string,
    string,
    string,
    string,
    string,
  ];
  const minute = Number.parseInt(minuteRaw, 10);
  const hour = Number.parseInt(hourRaw, 10);
  if (Number.isNaN(minute) || Number.isNaN(hour) || monthRaw !== "*") {
    return text.cron(cadence.expression);
  }
  const time = formatTimeOfDay(hour, minute);

  if (domRaw === "*" && dowRaw === "*") {
    return text.daily(time);
  }
  if (domRaw === "*" && dowRaw === "1-5") {
    return text.weekdays(time);
  }
  if (domRaw === "*" && /^[0-6]$/.test(dowRaw)) {
    const name = text.weekdayNames[Number.parseInt(dowRaw, 10)] ?? dowRaw;
    return text.weekly(name, time);
  }
  if (dowRaw === "*" && /^\d{1,2}$/.test(domRaw)) {
    return text.monthly(Number.parseInt(domRaw, 10), time);
  }
  return text.cron(cadence.expression);
}

/** Reconstructs an editable draft from a stored cadence, for the edit flow. */
export function toCadenceDraft(cadence: ScheduleCadence): CadenceDraft {
  const draft = defaultCadenceDraft("interval");

  if (cadence.type === "every") {
    const minutes = Math.round(cadence.everyMs / 60_000);
    // Whole hours read better as hours; anything else stays in minutes.
    if (minutes % 60 === 0) {
      return { ...draft, kind: "interval", intervalUnit: "hours", intervalValue: minutes / 60 };
    }
    return { ...draft, kind: "interval", intervalUnit: "minutes", intervalValue: minutes };
  }

  const parts = cadence.expression.trim().split(/\s+/);
  if (parts.length !== 5) {
    return draft;
  }
  const minute = Number.parseInt(parts[0] ?? "", 10);
  const hour = Number.parseInt(parts[1] ?? "", 10);
  const dom = parts[2] ?? "*";
  const dow = parts[4] ?? "*";
  if (Number.isNaN(minute) || Number.isNaN(hour)) {
    return draft;
  }

  const base = { ...draft, hour, minute };
  if (dom === "*" && dow === "1-5") {
    return { ...base, kind: "weekdays" };
  }
  if (dom === "*" && /^[0-6]$/.test(dow)) {
    return { ...base, kind: "weekly", weekday: Number.parseInt(dow, 10) };
  }
  if (dow === "*" && /^\d{1,2}$/.test(dom)) {
    return { ...base, kind: "monthly", dayOfMonth: Number.parseInt(dom, 10) };
  }
  return { ...base, kind: "daily" };
}

/** Short relative label for a next/last run timestamp. */
export function formatRunTimestamp(iso: string | null, locale: string): string | null {
  if (!iso) {
    return null;
  }
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) {
    return null;
  }
  return new Intl.DateTimeFormat(locale === "zh" ? "zh-CN" : "en-US", {
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

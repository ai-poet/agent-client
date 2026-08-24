import { describe, expect, it } from "vitest";

import {
  defaultCadenceDraft,
  formatCadence,
  formatTimeOfDay,
  toCadence,
  toCadenceDraft,
  type ScheduleSummaryText,
} from "./schedule-format";

const text: ScheduleSummaryText = {
  everyMinutes: (minutes) => `every ${minutes}m`,
  everyHours: (hours) => `every ${hours}h`,
  daily: (time) => `daily ${time}`,
  weekdays: (time) => `weekdays ${time}`,
  weekly: (day, time) => `${day} ${time}`,
  monthly: (day, time) => `day ${day} ${time}`,
  cron: (expression) => `cron(${expression})`,
  weekdayNames: ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
};

describe("toCadence", () => {
  it("converts hourly intervals to milliseconds", () => {
    expect(toCadence({ ...defaultCadenceDraft("interval"), intervalValue: 2 })).toEqual({
      type: "every",
      everyMs: 2 * 60 * 60_000,
    });
  });

  it("enforces the interval floors", () => {
    expect(
      toCadence({
        ...defaultCadenceDraft("interval"),
        intervalUnit: "minutes",
        intervalValue: 1,
      }),
    ).toEqual({ type: "every", everyMs: 15 * 60_000 });

    expect(
      toCadence({ ...defaultCadenceDraft("interval"), intervalUnit: "hours", intervalValue: 0 }),
    ).toEqual({ type: "every", everyMs: 60 * 60_000 });
  });

  it("caps very long intervals", () => {
    expect(
      toCadence({ ...defaultCadenceDraft("interval"), intervalUnit: "hours", intervalValue: 999 }),
    ).toEqual({ type: "every", everyMs: 168 * 60 * 60_000 });
  });

  it("builds cron expressions for the calendar presets", () => {
    expect(toCadence({ ...defaultCadenceDraft("daily"), hour: 9, minute: 30 })).toEqual({
      type: "cron",
      expression: "30 9 * * *",
    });
    expect(toCadence({ ...defaultCadenceDraft("weekdays"), hour: 8, minute: 0 })).toEqual({
      type: "cron",
      expression: "0 8 * * 1-5",
    });
    expect(
      toCadence({ ...defaultCadenceDraft("weekly"), hour: 18, minute: 5, weekday: 3 }),
    ).toEqual({ type: "cron", expression: "5 18 * * 3" });
    expect(
      toCadence({ ...defaultCadenceDraft("monthly"), hour: 7, minute: 0, dayOfMonth: 15 }),
    ).toEqual({ type: "cron", expression: "0 7 15 * *" });
  });

  it("clamps an out-of-range day of month", () => {
    expect(toCadence({ ...defaultCadenceDraft("monthly"), dayOfMonth: 99 })).toEqual({
      type: "cron",
      expression: "0 9 31 * *",
    });
  });
});

describe("formatCadence", () => {
  it("describes intervals in the friendlier unit", () => {
    expect(formatCadence({ type: "every", everyMs: 2 * 60 * 60_000 }, text)).toBe("every 2h");
    expect(formatCadence({ type: "every", everyMs: 30 * 60_000 }, text)).toBe("every 30m");
  });

  it("recognises the presets it generates", () => {
    expect(formatCadence({ type: "cron", expression: "30 9 * * *" }, text)).toBe("daily 09:30");
    expect(formatCadence({ type: "cron", expression: "0 8 * * 1-5" }, text)).toBe("weekdays 08:00");
    expect(formatCadence({ type: "cron", expression: "5 18 * * 3" }, text)).toBe("Wed 18:05");
    expect(formatCadence({ type: "cron", expression: "0 7 15 * *" }, text)).toBe("day 15 07:00");
  });

  it("falls back to the raw expression for hand-written crons", () => {
    expect(formatCadence({ type: "cron", expression: "*/5 * * * *" }, text)).toBe(
      "cron(*/5 * * * *)",
    );
    expect(formatCadence({ type: "cron", expression: "bogus" }, text)).toBe("cron(bogus)");
  });
});

describe("toCadenceDraft", () => {
  it("round-trips every preset", () => {
    const drafts = [
      { ...defaultCadenceDraft("daily"), hour: 9, minute: 30 },
      { ...defaultCadenceDraft("weekdays"), hour: 8, minute: 0 },
      { ...defaultCadenceDraft("weekly"), hour: 18, minute: 5, weekday: 3 },
      { ...defaultCadenceDraft("monthly"), hour: 7, minute: 0, dayOfMonth: 15 },
    ];

    for (const draft of drafts) {
      const restored = toCadenceDraft(toCadence(draft));
      expect(restored.kind).toBe(draft.kind);
      expect(restored.hour).toBe(draft.hour);
      expect(restored.minute).toBe(draft.minute);
    }
  });

  it("infers the interval unit from whether the value divides into hours", () => {
    expect(toCadenceDraft({ type: "every", everyMs: 90 * 60_000 })).toMatchObject({
      intervalUnit: "minutes",
      intervalValue: 90,
    });
    expect(toCadenceDraft({ type: "every", everyMs: 3 * 60 * 60_000 })).toMatchObject({
      intervalUnit: "hours",
      intervalValue: 3,
    });
  });

  it("keeps the weekday when restoring a weekly schedule", () => {
    expect(toCadenceDraft({ type: "cron", expression: "0 9 * * 5" })).toMatchObject({
      kind: "weekly",
      weekday: 5,
    });
  });
});

describe("formatTimeOfDay", () => {
  it("zero pads both parts", () => {
    expect(formatTimeOfDay(9, 5)).toBe("09:05");
    expect(formatTimeOfDay(18, 30)).toBe("18:30");
  });
});

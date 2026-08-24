import { describe, expect, test } from "vitest";

import { resolveCostSemantics, resolveTurnCostUsd } from "./cost.js";

describe("resolveCostSemantics", () => {
  test("marks session-total providers as cumulative", () => {
    expect(resolveCostSemantics("claude")).toBe("cumulative");
    expect(resolveCostSemantics("opencode")).toBe("cumulative");
    expect(resolveCostSemantics("pi")).toBe("cumulative");
  });

  test("treats unknown providers as per-turn", () => {
    expect(resolveCostSemantics("codex")).toBe("delta");
    expect(resolveCostSemantics("grok")).toBe("delta");
  });
});

describe("resolveTurnCostUsd", () => {
  test("passes through a per-turn reading", () => {
    expect(
      resolveTurnCostUsd({
        semantics: "delta",
        rawTotalCostUsd: 0.02,
        previousRawTotalCostUsd: 0.5,
      }),
    ).toBe(0.02);
  });

  test("charges only the increase for a cumulative provider", () => {
    expect(
      resolveTurnCostUsd({
        semantics: "cumulative",
        rawTotalCostUsd: 0.3,
        previousRawTotalCostUsd: 0.2,
      }),
    ).toBeCloseTo(0.1, 10);
  });

  test("charges the full reading on the first turn of a cumulative session", () => {
    expect(
      resolveTurnCostUsd({
        semantics: "cumulative",
        rawTotalCostUsd: 0.2,
        previousRawTotalCostUsd: undefined,
      }),
    ).toBe(0.2);
  });

  test("treats a counter reset as a fresh session instead of a negative cost", () => {
    expect(
      resolveTurnCostUsd({
        semantics: "cumulative",
        rawTotalCostUsd: 0.05,
        previousRawTotalCostUsd: 0.9,
      }),
    ).toBe(0.05);
  });

  test("returns undefined when the provider reported no cost", () => {
    expect(
      resolveTurnCostUsd({
        semantics: "cumulative",
        rawTotalCostUsd: undefined,
        previousRawTotalCostUsd: 0.2,
      }),
    ).toBeUndefined();
  });

  test("ignores non-finite readings", () => {
    expect(
      resolveTurnCostUsd({
        semantics: "delta",
        rawTotalCostUsd: Number.NaN,
        previousRawTotalCostUsd: undefined,
      }),
    ).toBeUndefined();
  });

  test("a cumulative session summed turn-by-turn equals the final reading", () => {
    // The regression this whole mechanism exists to prevent.
    const readings = [0.1, 0.25, 0.4, 0.75];
    let previous: number | undefined;
    let summed = 0;
    for (const reading of readings) {
      summed +=
        resolveTurnCostUsd({
          semantics: "cumulative",
          rawTotalCostUsd: reading,
          previousRawTotalCostUsd: previous,
        }) ?? 0;
      previous = reading;
    }

    expect(summed).toBeCloseTo(0.75, 10);
  });
});

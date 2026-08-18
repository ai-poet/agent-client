import { mkdtemp, readdir, readFile, writeFile, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, describe, expect, test } from "vitest";

import { createTestLogger } from "../../test-utils/test-logger.js";
import { UsageService } from "./service.js";
import { UsageStore, monthKeysBetween } from "./store.js";

const created: string[] = [];

async function makeHome(): Promise<string> {
  const home = await mkdtemp(path.join(tmpdir(), "paseo-usage-"));
  created.push(home);
  return home;
}

function makeService(home: string): UsageService {
  return new UsageService({ paseoHome: home, logger: createTestLogger() });
}

afterEach(() => {
  created.length = 0;
});

describe("UsageService.recordTurn", () => {
  test("writes one line per turn into a month-sharded log", async () => {
    const home = await makeHome();
    const service = makeService(home);

    await service.recordTurn({
      agentId: "a1",
      provider: "codex",
      cwd: "/repo",
      model: "gpt-5.4",
      usage: { inputTokens: 100, outputTokens: 20, totalCostUsd: 0.01 },
      durationMs: 1500,
    });
    await service.recordTurn({
      agentId: "a1",
      provider: "codex",
      cwd: "/repo",
      usage: { inputTokens: 10, outputTokens: 2 },
    });

    const files = await readdir(path.join(home, "usage"));
    expect(files).toHaveLength(1);
    expect(files[0]).toMatch(/^\d{4}-\d{2}\.jsonl$/);

    const lines = (await readFile(path.join(home, "usage", files[0]!), "utf8"))
      .split("\n")
      .filter(Boolean);
    expect(lines).toHaveLength(2);
    expect(JSON.parse(lines[0]!)).toMatchObject({
      agentId: "a1",
      provider: "codex",
      model: "gpt-5.4",
      inputTokens: 100,
      costUsd: 0.01,
      costSemantics: "delta",
      durationMs: 1500,
    });
  });

  test("stores the per-turn delta for a cumulative provider", async () => {
    const home = await makeHome();
    const service = makeService(home);

    await service.recordTurn({
      agentId: "a1",
      provider: "claude",
      cwd: "/repo",
      usage: { totalCostUsd: 0.2 },
    });
    await service.recordTurn({
      agentId: "a1",
      provider: "claude",
      cwd: "/repo",
      usage: { totalCostUsd: 0.5 },
      previousRawTotalCostUsd: 0.2,
    });

    const stats = await service.getStats({
      from: new Date(Date.now() - 60_000),
      to: new Date(Date.now() + 60_000),
      groupBy: "provider",
    });

    // Naive summing of the raw readings would give 0.7.
    expect(stats.totals.costUsd).toBeCloseTo(0.5, 10);
  });

  test("never throws when the log cannot be written", async () => {
    const service = new UsageService({
      paseoHome: path.join("\0invalid"),
      logger: createTestLogger(),
    });

    await expect(
      service.recordTurn({ agentId: "a1", provider: "codex", cwd: "/repo", usage: undefined }),
    ).resolves.toBeUndefined();
  });
});

describe("UsageService.getStats", () => {
  async function seed(home: string): Promise<void> {
    const service = makeService(home);
    await service.recordTurn({
      agentId: "a1",
      provider: "codex",
      cwd: "/repo-a",
      model: "gpt-5.4",
      usage: { inputTokens: 100, outputTokens: 50, totalCostUsd: 0.01 },
      durationMs: 1000,
    });
    await service.recordTurn({
      agentId: "a2",
      provider: "grok",
      cwd: "/repo-b",
      model: "grok-4.5",
      usage: { inputTokens: 200, outputTokens: 100, totalCostUsd: 0.02 },
      durationMs: 2000,
    });
  }

  const wideRange = () => ({
    from: new Date(Date.now() - 60_000),
    to: new Date(Date.now() + 60_000),
  });

  test("totals every dimension across records", async () => {
    const home = await makeHome();
    await seed(home);

    const stats = await makeService(home).getStats({ ...wideRange(), groupBy: "provider" });

    expect(stats.totals).toMatchObject({
      turns: 2,
      inputTokens: 300,
      outputTokens: 150,
      totalTokens: 450,
      durationMs: 3000,
    });
    expect(stats.totals.costUsd).toBeCloseTo(0.03, 10);
  });

  test("groups by provider, project and model", async () => {
    const home = await makeHome();
    await seed(home);
    const service = makeService(home);

    const byProvider = await service.getStats({ ...wideRange(), groupBy: "provider" });
    expect(byProvider.buckets.map((bucket) => bucket.key)).toEqual(["grok", "codex"]);

    const byProject = await service.getStats({ ...wideRange(), groupBy: "project" });
    expect(byProject.buckets.map((bucket) => bucket.key)).toEqual(["/repo-b", "/repo-a"]);

    const byModel = await service.getStats({ ...wideRange(), groupBy: "model" });
    expect(byModel.buckets.map((bucket) => bucket.key)).toEqual(["grok-4.5", "gpt-5.4"]);
  });

  test("day buckets are ordered chronologically", async () => {
    const home = await makeHome();
    const dir = path.join(home, "usage");
    await mkdir(dir, { recursive: true });
    await writeFile(
      path.join(dir, "2026-03.jsonl"),
      [
        JSON.stringify({
          ts: "2026-03-05T00:00:00.000Z",
          agentId: "a",
          provider: "codex",
          cwd: "/r",
          outputTokens: 5,
        }),
        JSON.stringify({
          ts: "2026-03-01T00:00:00.000Z",
          agentId: "a",
          provider: "codex",
          cwd: "/r",
          outputTokens: 1,
        }),
      ].join("\n") + "\n",
      "utf8",
    );

    const stats = await makeService(home).getStats({
      from: new Date("2026-03-01T00:00:00.000Z"),
      to: new Date("2026-03-31T00:00:00.000Z"),
      groupBy: "day",
    });

    expect(stats.buckets.map((bucket) => bucket.key)).toEqual(["2026-03-01", "2026-03-05"]);
  });

  test("filters by provider and project", async () => {
    const home = await makeHome();
    await seed(home);
    const service = makeService(home);

    const filtered = await service.getStats({
      ...wideRange(),
      groupBy: "provider",
      provider: "grok",
    });
    expect(filtered.totals.turns).toBe(1);
    expect(filtered.buckets.map((bucket) => bucket.key)).toEqual(["grok"]);

    const byCwd = await service.getStats({ ...wideRange(), groupBy: "provider", cwd: "/repo-a" });
    expect(byCwd.totals.turns).toBe(1);
  });

  test("excludes records outside the requested range", async () => {
    const home = await makeHome();
    const dir = path.join(home, "usage");
    await mkdir(dir, { recursive: true });
    await writeFile(
      path.join(dir, "2026-03.jsonl"),
      `${JSON.stringify({ ts: "2026-03-20T00:00:00.000Z", agentId: "a", provider: "codex", cwd: "/r", outputTokens: 7 })}\n`,
      "utf8",
    );

    const stats = await makeService(home).getStats({
      from: new Date("2026-03-01T00:00:00.000Z"),
      to: new Date("2026-03-10T00:00:00.000Z"),
      groupBy: "day",
    });

    expect(stats.totals.turns).toBe(0);
    expect(stats.buckets).toEqual([]);
  });

  test("skips malformed lines instead of failing the query", async () => {
    const home = await makeHome();
    const dir = path.join(home, "usage");
    await mkdir(dir, { recursive: true });
    await writeFile(
      path.join(dir, "2026-03.jsonl"),
      [
        "{ not json",
        JSON.stringify({ missing: "required fields" }),
        JSON.stringify({
          ts: "2026-03-02T00:00:00.000Z",
          agentId: "a",
          provider: "codex",
          cwd: "/r",
          outputTokens: 3,
        }),
      ].join("\n") + "\n",
      "utf8",
    );

    const stats = await makeService(home).getStats({
      from: new Date("2026-03-01T00:00:00.000Z"),
      to: new Date("2026-03-31T00:00:00.000Z"),
      groupBy: "day",
    });

    expect(stats.totals.turns).toBe(1);
  });
});

describe("UsageStore retention", () => {
  test("removes month shards older than the retention window and keeps recent ones", async () => {
    const home = await makeHome();
    const dir = path.join(home, "usage");
    await mkdir(dir, { recursive: true });
    await writeFile(path.join(dir, "2025-01.jsonl"), "", "utf8");
    await writeFile(path.join(dir, "2026-03.jsonl"), "", "utf8");
    await writeFile(path.join(dir, "not-a-shard.txt"), "", "utf8");

    const store = new UsageStore({ paseoHome: home, logger: createTestLogger() });
    await store.prune(new Date("2026-03-15T00:00:00.000Z"));

    const remaining = await readdir(dir);
    expect(remaining).toContain("2026-03.jsonl");
    expect(remaining).toContain("not-a-shard.txt");
    expect(remaining).not.toContain("2025-01.jsonl");
  });
});

describe("monthKeysBetween", () => {
  test("covers every month spanned by the range", () => {
    expect(
      monthKeysBetween(new Date("2025-11-20T00:00:00Z"), new Date("2026-02-03T00:00:00Z")),
    ).toEqual(["2025-11", "2025-12", "2026-01", "2026-02"]);
  });

  test("returns a single month when the range stays inside one", () => {
    expect(
      monthKeysBetween(new Date("2026-03-01T00:00:00Z"), new Date("2026-03-31T00:00:00Z")),
    ).toEqual(["2026-03"]);
  });
});

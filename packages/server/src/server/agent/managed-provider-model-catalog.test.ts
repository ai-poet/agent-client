import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Logger } from "pino";
import { afterEach, describe, expect, it, vi } from "vitest";

import { createTestLogger } from "../../test-utils/test-logger.js";
import type { AgentModelDefinition } from "./agent-sdk-types.js";
import { getClaudeModels } from "./providers/claude/claude-models.js";
import {
  ManagedProviderModelCatalog,
  filterManagedModelsForProvider,
  isManagedModelProvider,
  normalizeManagedProviderEndpoint,
} from "./managed-provider-model-catalog.js";

const fallbackModels: AgentModelDefinition[] = [
  {
    provider: "claude",
    id: "known-default",
    label: "Known Default",
    description: "Local description",
    isDefault: true,
    thinkingOptions: [{ id: "high", label: "High" }],
    defaultThinkingOptionId: "high",
  },
  { provider: "claude", id: "known-other", label: "Known Other" },
];

const tempDirs: string[] = [];

async function createProvidersFile(store: unknown): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), "managed-model-catalog-"));
  tempDirs.push(dir);
  const file = join(dir, "providers.json");
  await writeFile(file, JSON.stringify(store));
  return file;
}

function managedStore(input?: { claudeKey?: string; codexKey?: string }) {
  return {
    providers: [
      {
        id: "paseo-managed-claude",
        endpoint: "https://gateway.example.com/base/v1/",
        apiKey: input?.claudeKey ?? "sk-claude",
        target: "claude",
      },
      {
        id: "paseo-managed-codex",
        endpoint: "https://gateway.example.com/v1",
        apiKey: input?.codexKey ?? "sk-codex",
        target: "codex",
      },
    ],
    activeClaudeProviderId: "paseo-managed-claude",
    activeCodexProviderId: "paseo-managed-codex",
  };
}

afterEach(async () => {
  await Promise.all(tempDirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true })));
  vi.restoreAllMocks();
});

describe("normalizeManagedProviderEndpoint", () => {
  it("normalizes trailing slashes and a trailing /v1 path", () => {
    expect(normalizeManagedProviderEndpoint(" https://gateway.example.com/base/V1/// ")).toBe(
      "https://gateway.example.com/base",
    );
  });

  it("rejects non-HTTP and credential-bearing endpoints", () => {
    expect(normalizeManagedProviderEndpoint("file:///tmp/models")).toBeNull();
    expect(normalizeManagedProviderEndpoint("https://user:pass@example.com")).toBeNull();
  });
});

describe("ManagedProviderModelCatalog", () => {
  it("uses independent Claude and Codex keys and normalizes request URLs", async () => {
    const providersFile = await createProvidersFile(managedStore());
    const fetchImpl = vi.fn<typeof fetch>(async (_input, init) => {
      const authorization = new Headers(init?.headers).get("Authorization");
      const id = authorization === "Bearer sk-claude" ? "claude-remote" : "codex-remote";
      return Response.json({ object: "list", data: [{ id }] });
    });
    const catalog = new ManagedProviderModelCatalog(createTestLogger(), {
      providersFile,
      fetchImpl,
    });

    await expect(catalog.getModels("claude", fallbackModels)).resolves.toEqual([
      expect.objectContaining({ id: "claude-remote", isDefault: true }),
    ]);
    await expect(
      catalog.getModels(
        "codex",
        fallbackModels.map((model) => ({ ...model, provider: "codex" })),
      ),
    ).resolves.toEqual([expect.objectContaining({ id: "codex-remote", isDefault: true })]);
    expect(fetchImpl.mock.calls.map(([url]) => String(url))).toEqual([
      "https://gateway.example.com/base/v1/models",
      "https://gateway.example.com/v1/models",
    ]);
    expect(
      fetchImpl.mock.calls.map(([, init]) => new Headers(init?.headers).get("Authorization")),
    ).toEqual(["Bearer sk-claude", "Bearer sk-codex"]);
  });

  it("fully overrides fallback IDs while preserving known metadata and admitting unknown models", async () => {
    const providersFile = await createProvidersFile(managedStore());
    const fetchImpl = vi.fn<typeof fetch>(async () =>
      Response.json({
        object: "list",
        data: [
          { id: "known-other", display_name: "Remote label ignored" },
          { id: "unknown", display_name: "Remote Unknown" },
          { id: "unknown", display_name: "Duplicate" },
          { id: "   " },
        ],
      }),
    );
    const catalog = new ManagedProviderModelCatalog(createTestLogger(), {
      providersFile,
      fetchImpl,
    });

    const models = await catalog.getModels("claude", fallbackModels);

    expect(models).toEqual([
      expect.objectContaining({ id: "known-other", label: "Known Other", isDefault: true }),
      { provider: "claude", id: "unknown", label: "Remote Unknown" },
    ]);
    expect(models[0]!.thinkingOptions).toBeUndefined();
    expect(models[1]!.thinkingOptions).toBeUndefined();
  });

  it("retains the hardcoded default when the remote list contains it", async () => {
    const providersFile = await createProvidersFile(managedStore());
    const catalog = new ManagedProviderModelCatalog(createTestLogger(), {
      providersFile,
      fetchImpl: vi.fn(async () =>
        Response.json({ data: [{ id: "known-other" }, { id: "known-default" }] }),
      ),
    });

    const models = await catalog.getModels("claude", fallbackModels);

    expect(models.filter((model) => model.isDefault).map((model) => model.id)).toEqual([
      "known-default",
    ]);
    expect(models[1]).toMatchObject({
      label: "Known Default",
      description: "Local description",
      defaultThinkingOptionId: "high",
      thinkingOptions: [{ id: "high", label: "High" }],
    });
  });

  it("uses canonical Claude metadata for a dated remote model ID", async () => {
    const providersFile = await createProvidersFile(managedStore());
    const catalog = new ManagedProviderModelCatalog(createTestLogger(), {
      providersFile,
      fetchImpl: vi.fn(async () =>
        Response.json({
          data: [
            {
              id: "claude-haiku-4-5-20251001",
              display_name: "claude-haiku-4-5-20251001",
            },
          ],
        }),
      ),
    });

    const models = await catalog.getModels("claude", getClaudeModels());

    expect(models).toEqual([
      expect.objectContaining({
        id: "claude-haiku-4-5-20251001",
        label: "Haiku 4.5",
        description: "Haiku 4.5 · Fastest for quick answers",
        isDefault: true,
        thinkingOptions: [
          { id: "low", label: "Low" },
          { id: "medium", label: "Medium" },
          { id: "high", label: "High" },
          { id: "max", label: "Max" },
        ],
      }),
    ]);
  });

  it("uses Opus 5 metadata for a dated remote model ID", async () => {
    const providersFile = await createProvidersFile(managedStore());
    const catalog = new ManagedProviderModelCatalog(createTestLogger(), {
      providersFile,
      fetchImpl: vi.fn(async () =>
        Response.json({
          data: [{ id: "claude-opus-5-20260727", display_name: "claude-opus-5-20260727" }],
        }),
      ),
    });

    const models = await catalog.getModels("claude", getClaudeModels());

    expect(models).toEqual([
      expect.objectContaining({
        id: "claude-opus-5-20260727",
        label: "Opus 5",
        description: "Opus 5 · Latest release",
        isDefault: true,
        thinkingOptions: [
          { id: "low", label: "Low" },
          { id: "medium", label: "Medium" },
          { id: "high", label: "High" },
          { id: "xhigh", label: "Extra High" },
          { id: "max", label: "Max" },
        ],
      }),
    ]);
  });

  it("uses a successful same-route cache on failure but does not share it after a key change", async () => {
    const store = managedStore();
    const providersFile = await createProvidersFile(store);
    const fetchImpl = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(Response.json({ data: [{ id: "cached-remote" }] }))
      .mockRejectedValue(new Error("offline"));
    const catalog = new ManagedProviderModelCatalog(createTestLogger(), {
      providersFile,
      fetchImpl,
    });

    await expect(catalog.getModels("claude", fallbackModels)).resolves.toEqual([
      expect.objectContaining({ id: "cached-remote" }),
    ]);
    await expect(catalog.getModels("claude", fallbackModels)).resolves.toEqual([
      expect.objectContaining({ id: "cached-remote" }),
    ]);

    store.providers[0]!.apiKey = "sk-claude-replaced";
    await writeFile(providersFile, JSON.stringify(store));
    await expect(catalog.getModels("claude", fallbackModels)).resolves.toEqual(fallbackModels);
  });

  it.each([
    ["http error", async () => new Response("bad", { status: 502 })],
    ["malformed body", async () => new Response("not-json")],
    ["empty list", async () => Response.json({ data: [{ id: "" }] })],
  ])("falls back without a cache on %s", async (_name, responseFactory) => {
    const providersFile = await createProvidersFile(managedStore());
    const catalog = new ManagedProviderModelCatalog(createTestLogger(), {
      providersFile,
      fetchImpl: vi.fn(responseFactory),
    });

    await expect(catalog.getModels("claude", fallbackModels)).resolves.toEqual(fallbackModels);
  });

  it("aborts after the configured timeout", async () => {
    const providersFile = await createProvidersFile(managedStore());
    const fetchImpl = vi.fn<typeof fetch>(
      async (_input, init) =>
        await new Promise<Response>((_resolve, reject) => {
          init?.signal?.addEventListener("abort", () => reject(new Error("aborted")));
        }),
    );
    const catalog = new ManagedProviderModelCatalog(createTestLogger(), {
      providersFile,
      fetchImpl,
      timeoutMs: 5,
    });

    await expect(catalog.getModels("claude", fallbackModels)).resolves.toEqual(fallbackModels);
    expect(fetchImpl.mock.calls[0]?.[1]?.signal?.aborted).toBe(true);
  });

  it("does not include the API key in failure logs", async () => {
    const providersFile = await createProvidersFile(managedStore({ claudeKey: "secret-key" }));
    const debug = vi.fn();
    const catalog = new ManagedProviderModelCatalog({ debug } as unknown as Logger, {
      providersFile,
      fetchImpl: vi.fn(async () => new Response("bad", { status: 503 })),
    });

    await catalog.getModels("claude", fallbackModels);

    expect(debug).toHaveBeenCalledOnce();
    expect(JSON.stringify(debug.mock.calls)).not.toContain("secret-key");
  });

  it("does not request models for BYOK, custom, missing, or invalid managed configurations", async () => {
    const stores = [
      { ...managedStore(), activeClaudeProviderId: "custom-claude" },
      { ...managedStore(), activeClaudeProviderId: null },
      { ...managedStore(), providers: [] },
      {
        ...managedStore(),
        providers: [
          {
            id: "paseo-managed-claude",
            endpoint: "not-a-url",
            apiKey: "sk-claude",
            target: "claude",
          },
        ],
      },
    ];

    for (const store of stores) {
      const providersFile = await createProvidersFile(store);
      const fetchImpl = vi.fn<typeof fetch>();
      const catalog = new ManagedProviderModelCatalog(createTestLogger(), {
        providersFile,
        fetchImpl,
      });

      await expect(catalog.getModels("claude", fallbackModels)).resolves.toEqual(fallbackModels);
      expect(fetchImpl).not.toHaveBeenCalled();
    }
  });
});

describe("isManagedModelProvider", () => {
  it("recognises the providers that can route through the gateway", () => {
    expect(isManagedModelProvider("claude")).toBe(true);
    expect(isManagedModelProvider("codex")).toBe(true);
    expect(isManagedModelProvider("grok")).toBe(true);
  });

  it("rejects providers with no managed route", () => {
    expect(isManagedModelProvider("opencode")).toBe(false);
    expect(isManagedModelProvider("pi")).toBe(false);
  });
});

describe("filterManagedModelsForProvider", () => {
  const catalog = [
    { id: "grok-4.5" },
    { id: "Grok-Build" },
    { id: "gpt-5.4" },
    { id: "claude-sonnet-4" },
  ];

  it("narrows the shared gateway catalog to the xAI family for grok", () => {
    // The openai endpoint also lists models grok cannot run.
    expect(filterManagedModelsForProvider("grok", catalog).map((model) => model.id)).toEqual([
      "grok-4.5",
      "Grok-Build",
    ]);
  });

  it("leaves other providers' catalogs untouched", () => {
    expect(filterManagedModelsForProvider("codex", catalog)).toHaveLength(4);
    expect(filterManagedModelsForProvider("claude", catalog)).toHaveLength(4);
  });
});

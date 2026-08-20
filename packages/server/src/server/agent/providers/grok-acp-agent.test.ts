import { homedir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import { createTestLogger } from "../../../test-utils/test-logger.js";

const mockState = vi.hoisted(() => ({
  superConstructorOptions: [] as unknown[],
  baseIsAvailable: true,
  existingPaths: new Set<string>(),
  initializeResponse: {} as unknown,
  newSessionResponse: {} as unknown,
  spawnedEnvs: [] as unknown[],
  closedProbes: 0,
}));

vi.mock("node:fs", () => ({
  existsSync: (path: string) => mockState.existingPaths.has(path),
}));

vi.mock("./acp-agent.js", () => ({
  PROBE_ENV: { NO_BROWSER: "true" },
  deriveModelDefinitionsFromACP: (provider: string, models: unknown) => {
    const available = (models as { availableModels?: { modelId: string; name: string }[] } | null)
      ?.availableModels;
    return (available ?? []).map((model) => ({
      provider,
      id: model.modelId,
      label: model.name,
      source: "session/new",
    }));
  },
  ACPAgentClient: class ACPAgentClient {
    readonly provider: string;

    constructor(options: { provider: string }) {
      this.provider = options.provider;
      mockState.superConstructorOptions.push(options);
    }

    async isAvailable(): Promise<boolean> {
      return mockState.baseIsAvailable;
    }

    protected async spawnProcess(launchEnv?: Record<string, string>) {
      mockState.spawnedEnvs.push(launchEnv);
      return {
        initialize: mockState.initializeResponse,
        connection: {
          newSession: async () => mockState.newSessionResponse,
        },
      };
    }

    protected async closeProbe(): Promise<void> {
      mockState.closedProbes += 1;
    }

    protected transformSessionResponse(response: unknown): unknown {
      return response;
    }
  },
}));

vi.mock("../../../utils/executable.js", () => ({
  findExecutable: async () => "/usr/local/bin/grok",
}));

vi.mock("./diagnostic-utils.js", () => ({
  formatDiagnosticStatus: (available: boolean) => (available ? "Available" : "Unavailable"),
  formatProviderDiagnostic: (name: string, rows: { label: string; value: string }[]) =>
    `${name}\n${rows.map((row) => `${row.label}: ${row.value}`).join("\n")}`,
  formatProviderDiagnosticError: (name: string, error: unknown) =>
    `${name} failed: ${String(error)}`,
  resolveBinaryVersion: async () => "1.0.4",
  toDiagnosticErrorMessage: (error: unknown) => String(error),
}));

import { GrokACPAgentClient } from "./grok-acp-agent.js";

const DEFAULT_AUTH_PATH = join(homedir(), ".grok", "auth.json");

function createClient() {
  return new GrokACPAgentClient({ logger: createTestLogger() });
}

describe("GrokACPAgentClient", () => {
  beforeEach(() => {
    mockState.superConstructorOptions = [];
    mockState.baseIsAvailable = true;
    mockState.existingPaths = new Set<string>();
    mockState.initializeResponse = {};
    mockState.newSessionResponse = {};
    mockState.spawnedEnvs = [];
    mockState.closedProbes = 0;
    delete process.env.XAI_API_KEY;
    delete process.env.GROK_HOME;
  });

  afterEach(() => {
    delete process.env.XAI_API_KEY;
    delete process.env.GROK_HOME;
  });

  test("launches the CLI in ACP stdio mode without declaring static modes", () => {
    createClient();

    expect(mockState.superConstructorOptions).toEqual([
      expect.objectContaining({
        provider: "grok",
        defaultCommand: ["grok", "agent", "stdio"],
        defaultModes: [],
      }),
    ]);
  });

  describe("isAvailable", () => {
    test("is false when the binary is missing even with credentials", async () => {
      mockState.baseIsAvailable = false;
      process.env.XAI_API_KEY = "xai-test";

      await expect(createClient().isAvailable()).resolves.toBe(false);
    });

    // Availability tracks installation, not configuration — the same rule Claude and Codex
    // follow. A missing key is reported by getDiagnostic, it does not hide the provider.
    test("is true when the binary exists but no credentials are configured", async () => {
      await expect(createClient().isAvailable()).resolves.toBe(true);
    });

    test("is true when XAI_API_KEY is set", async () => {
      process.env.XAI_API_KEY = "xai-test";

      await expect(createClient().isAvailable()).resolves.toBe(true);
    });

    test("is true when the default auth file exists", async () => {
      mockState.existingPaths.add(DEFAULT_AUTH_PATH);

      await expect(createClient().isAvailable()).resolves.toBe(true);
    });
  });

  describe("listModels", () => {
    test("uses the catalog reported on the initialize response", async () => {
      mockState.initializeResponse = {
        models: {
          availableModels: [
            { modelId: "grok-4.5", name: "Grok 4.5", description: "Flagship" },
            { modelId: "grok-build", name: "Grok Build" },
          ],
          currentModelId: "grok-build",
        },
      };

      const models = await createClient().listModels({ cwd: "/repo", force: false });

      expect(models).toEqual([
        {
          provider: "grok",
          id: "grok-4.5",
          label: "Grok 4.5",
          description: "Flagship",
          isDefault: false,
        },
        {
          provider: "grok",
          id: "grok-build",
          label: "Grok Build",
          description: undefined,
          isDefault: true,
        },
      ]);
      expect(mockState.spawnedEnvs).toEqual([{ NO_BROWSER: "true" }]);
      expect(mockState.closedProbes).toBe(1);
    });

    test("falls back to the session/new catalog when initialize reports no models", async () => {
      mockState.initializeResponse = {};
      mockState.newSessionResponse = {
        models: {
          availableModels: [{ modelId: "grok-4.5", name: "Grok 4.5" }],
        },
      };

      const models = await createClient().listModels({ cwd: "/repo", force: false });

      expect(models).toEqual([
        { provider: "grok", id: "grok-4.5", label: "Grok 4.5", source: "session/new" },
      ]);
      expect(mockState.closedProbes).toBe(1);
    });

    test("ignores malformed entries in the initialize catalog", async () => {
      mockState.initializeResponse = {
        models: {
          availableModels: [{ name: "missing id" }, null, { modelId: "grok-4.5" }],
        },
      };

      const models = await createClient().listModels({ cwd: "/repo", force: false });

      expect(models).toEqual([
        {
          provider: "grok",
          id: "grok-4.5",
          label: "grok-4.5",
          description: undefined,
          isDefault: false,
        },
      ]);
    });

    test("closes the probe and falls back when the session request fails", async () => {
      mockState.initializeResponse = {};
      mockState.newSessionResponse = {};
      const client = createClient();
      vi.spyOn(
        client as unknown as { transformSessionResponse: () => unknown },
        "transformSessionResponse",
      ).mockImplementation(() => {
        throw new Error("probe failed");
      });

      // Grok will not enumerate models until authenticated; surfacing nothing would drop it
      // out of the picker, so a known catalog stands in.
      const models = await client.listModels({ cwd: "/repo", force: false });

      expect(models.map((model) => model.id)).toEqual(["grok-4.6", "grok-4.5"]);
      expect(models[0]?.isDefault).toBe(true);
      expect(mockState.closedProbes).toBe(1);
    });
  });

  describe("getDiagnostic", () => {
    test("reports credential sources and the resolved auth path", async () => {
      process.env.XAI_API_KEY = "xai-test";
      mockState.initializeResponse = {
        models: { availableModels: [{ modelId: "grok-4.5", name: "Grok 4.5" }] },
      };

      const { diagnostic } = await createClient().getDiagnostic();

      expect(diagnostic).toContain("Binary: /usr/local/bin/grok");
      expect(diagnostic).toContain("Version: 1.0.4");
      expect(diagnostic).toContain("XAI_API_KEY: set");
      expect(diagnostic).toContain(`Auth config: not found (${DEFAULT_AUTH_PATH})`);
      expect(diagnostic).toContain("Models: 1");
      expect(diagnostic).toContain("Status: Available");
    });

    test("does not probe for models when the provider is unavailable", async () => {
      mockState.baseIsAvailable = false;

      const { diagnostic } = await createClient().getDiagnostic();

      expect(diagnostic).toContain("XAI_API_KEY: not set");
      expect(diagnostic).toContain("Models: Not checked");
      expect(diagnostic).toContain("Status: Unavailable");
      expect(mockState.spawnedEnvs).toEqual([]);
    });
  });
});

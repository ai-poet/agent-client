import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { mockedHome } = vi.hoisted(() => ({
  mockedHome: {
    dir: "",
  },
}));

vi.mock("node:os", async (importOriginal) => {
  const actual = await importOriginal<typeof import("node:os")>();
  return {
    ...actual,
    homedir: () => mockedHome.dir,
  };
});

describe("setupDefaultProvider scoped writes", () => {
  beforeEach(async () => {
    vi.resetModules();
    mockedHome.dir = await mkdtemp(join(tmpdir(), "paseo-provider-switch-"));
  });

  afterEach(async () => {
    if (mockedHome.dir) {
      await rm(mockedHome.dir, { recursive: true, force: true });
    }
  });

  it("writes only Claude config when scope is claude", async () => {
    const mod = await import("./provider-switch");

    await mod.setupDefaultProvider({
      endpoint: "https://api.example.com",
      apiKey: "sk-claude",
      name: "Claude only",
      scope: "claude",
    });

    const claudePath = join(mockedHome.dir, ".claude", "settings.json");
    const codexConfigPath = join(mockedHome.dir, ".codex", "config.toml");
    const codexAuthPath = join(mockedHome.dir, ".codex", "auth.json");
    const storePath = join(mockedHome.dir, ".paseo", "providers.json");

    expect(existsSync(claudePath)).toBe(true);
    expect(existsSync(codexConfigPath)).toBe(false);
    expect(existsSync(codexAuthPath)).toBe(false);

    const claudeSettings = JSON.parse(await readFile(claudePath, "utf8")) as {
      env?: Record<string, unknown>;
    };
    expect(claudeSettings).toEqual({
      env: {
        ANTHROPIC_BASE_URL: "https://api.example.com",
        ANTHROPIC_AUTH_TOKEN: "sk-claude",
        CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
        CLAUDE_CODE_ATTRIBUTION_HEADER: "0",
      },
    });

    const store = JSON.parse(await readFile(storePath, "utf8")) as {
      activeClaudeProviderId: string | null;
      activeCodexProviderId: string | null;
    };
    expect(store.activeClaudeProviderId).toBe(mod.PASEO_MANAGED_CLAUDE_PROVIDER_ID);
    expect(store.activeCodexProviderId).toBeNull();
  });

  it("keeps Claude Code Git Bash path after Windows Claude key switches", async () => {
    const mod = await import("./provider-switch");
    const claudePath = join(mockedHome.dir, ".claude", "settings.json");
    await mkdir(join(mockedHome.dir, ".claude"), { recursive: true });
    await writeFile(
      claudePath,
      JSON.stringify({
        env: {
          CLAUDE_CODE_GIT_BASH_PATH: "C:\\Program Files\\Git\\bin\\bash.exe",
        },
      }),
    );

    await mod.setupDefaultProvider({
      endpoint: "https://api.example.com",
      apiKey: "sk-new-claude",
      name: "Claude switch",
      scope: "claude",
      platform: "win32",
    });

    const claudeSettings = JSON.parse(await readFile(claudePath, "utf8")) as {
      env?: Record<string, unknown>;
    };
    expect(claudeSettings.env?.ANTHROPIC_AUTH_TOKEN).toBe("sk-new-claude");
    expect(claudeSettings.env?.CLAUDE_CODE_GIT_BASH_PATH).toBe(
      "C:\\Program Files\\Git\\bin\\bash.exe",
    );
  });

  it("safely patches Claude Code Git Bash path without touching Claude auth env", async () => {
    const mod = await import("./provider-switch");
    const claudePath = join(mockedHome.dir, ".claude", "settings.json");
    await mkdir(join(mockedHome.dir, ".claude"), { recursive: true });
    await writeFile(
      claudePath,
      JSON.stringify({
        env: {
          ANTHROPIC_AUTH_TOKEN: "sk-existing",
          ANTHROPIC_BASE_URL: "https://api.example.com",
        },
        permissions: {
          allow: ["Bash(git status)"],
        },
      }),
    );

    await mod.patchClaudeCodeGitBashPathForWindows("C:\\Program Files\\Git\\bin\\bash.exe", {
      platform: "win32",
    });

    const claudeSettings = JSON.parse(await readFile(claudePath, "utf8")) as {
      env?: Record<string, unknown>;
      permissions?: Record<string, unknown>;
    };
    expect(claudeSettings.env).toEqual({
      ANTHROPIC_AUTH_TOKEN: "sk-existing",
      ANTHROPIC_BASE_URL: "https://api.example.com",
      CLAUDE_CODE_GIT_BASH_PATH: "C:\\Program Files\\Git\\bin\\bash.exe",
    });
    expect(claudeSettings.permissions).toEqual({
      allow: ["Bash(git status)"],
    });
  });

  it("does not patch Claude Code Git Bash path on non-Windows platforms", async () => {
    const mod = await import("./provider-switch");
    const claudePath = join(mockedHome.dir, ".claude", "settings.json");

    await mod.patchClaudeCodeGitBashPathForWindows("C:\\Program Files\\Git\\bin\\bash.exe", {
      platform: "darwin",
    });

    expect(existsSync(claudePath)).toBe(false);
  });

  it("writes only Codex config when scope is codex", async () => {
    const mod = await import("./provider-switch");

    await mod.setupDefaultProvider({
      endpoint: "https://api.example.com",
      apiKey: "sk-codex",
      name: "Codex only",
      scope: "codex",
    });

    const claudePath = join(mockedHome.dir, ".claude", "settings.json");
    const codexConfigPath = join(mockedHome.dir, ".codex", "config.toml");
    const codexAuthPath = join(mockedHome.dir, ".codex", "auth.json");
    const storePath = join(mockedHome.dir, ".paseo", "providers.json");

    expect(existsSync(claudePath)).toBe(false);
    expect(existsSync(codexConfigPath)).toBe(true);
    expect(existsSync(codexAuthPath)).toBe(true);

    const codexConfig = await readFile(codexConfigPath, "utf8");
    const codexAuth = await readFile(codexAuthPath, "utf8");
    expect(codexConfig).toContain('model_provider = "OpenAI"');
    expect(codexAuth).toContain('"OPENAI_API_KEY": "sk-codex"');

    const store = JSON.parse(await readFile(storePath, "utf8")) as {
      activeClaudeProviderId: string | null;
      activeCodexProviderId: string | null;
    };
    expect(store.activeClaudeProviderId).toBeNull();
    expect(store.activeCodexProviderId).toBe(mod.PASEO_MANAGED_CODEX_PROVIDER_ID);
  });

  it("does not mirror Claude active id into Codex when only Claude has been set", async () => {
    const mod = await import("./provider-switch");

    await mod.setupDefaultProvider({
      endpoint: "https://api.example.com",
      apiKey: "sk-claude",
      scope: "claude",
    });

    const providers = await mod.getProviders();
    expect(providers.activeClaudeProviderId).toBe(mod.PASEO_MANAGED_CLAUDE_PROVIDER_ID);
    expect(providers.activeCodexProviderId).toBeNull();
  });

  it("does not mirror Codex active id into Claude when only Codex has been set", async () => {
    const mod = await import("./provider-switch");

    await mod.setupDefaultProvider({
      endpoint: "https://api.example.com",
      apiKey: "sk-codex",
      scope: "codex",
    });

    const providers = await mod.getProviders();
    expect(providers.activeClaudeProviderId).toBeNull();
    expect(providers.activeCodexProviderId).toBe(mod.PASEO_MANAGED_CODEX_PROVIDER_ID);
  });

  it("migrates legacy unscoped default rows into scoped Claude/Codex rows", async () => {
    const mod = await import("./provider-switch");
    const storePath = join(mockedHome.dir, ".paseo", "providers.json");
    await mkdir(join(mockedHome.dir, ".paseo"), { recursive: true });
    await writeFile(
      storePath,
      JSON.stringify(
        {
          providers: [
            {
              id: mod.DEFAULT_PROVIDER_ID,
              name: "Legacy Route",
              type: "default",
              endpoint: "https://api.example.com",
              apiKey: "sk-legacy",
              isDefault: true,
            },
          ],
          activeProviderId: mod.DEFAULT_PROVIDER_ID,
          activeClaudeProviderId: null,
          activeCodexProviderId: null,
        },
        null,
        2,
      ),
      "utf8",
    );

    const providers = await mod.getProviders();
    expect(providers.providers.some((p) => p.target === undefined)).toBe(false);
    expect(providers.providers.some((p) => p.id === mod.PASEO_MANAGED_CLAUDE_PROVIDER_ID)).toBe(
      true,
    );
    expect(providers.providers.some((p) => p.id === mod.PASEO_MANAGED_CODEX_PROVIDER_ID)).toBe(
      true,
    );
    expect(providers.activeClaudeProviderId).toBe(mod.PASEO_MANAGED_CLAUDE_PROVIDER_ID);
    expect(providers.activeCodexProviderId).toBe(mod.PASEO_MANAGED_CODEX_PROVIDER_ID);
  });

  it("deduplicates duplicated managed scoped rows into one row per CLI", async () => {
    const mod = await import("./provider-switch");
    const storePath = join(mockedHome.dir, ".paseo", "providers.json");
    await mkdir(join(mockedHome.dir, ".paseo"), { recursive: true });
    await writeFile(
      storePath,
      JSON.stringify(
        {
          providers: [
            {
              id: "dup-claude",
              name: "OpenAI (Claude Code)",
              type: "default",
              endpoint: "https://api.example.com",
              apiKey: "sk-dup-claude",
              isDefault: true,
              target: "claude",
            },
            {
              id: mod.PASEO_MANAGED_CLAUDE_PROVIDER_ID,
              name: "Anthropic",
              type: "default",
              endpoint: "https://api.example.com",
              apiKey: "sk-main-claude",
              isDefault: true,
              target: "claude",
            },
            {
              id: "dup-codex",
              name: "OpenAI",
              type: "default",
              endpoint: "https://api.example.com",
              apiKey: "sk-dup-codex",
              isDefault: true,
              target: "codex",
            },
            {
              id: mod.PASEO_MANAGED_CODEX_PROVIDER_ID,
              name: "OpenAI",
              type: "default",
              endpoint: "https://api.example.com",
              apiKey: "sk-main-codex",
              isDefault: true,
              target: "codex",
            },
          ],
          activeProviderId: null,
          activeClaudeProviderId: "dup-claude",
          activeCodexProviderId: "dup-codex",
        },
        null,
        2,
      ),
      "utf8",
    );

    const providers = await mod.getProviders();
    const claudeRows = providers.providers.filter(
      (provider) => provider.isDefault && provider.target === "claude",
    );
    const codexRows = providers.providers.filter(
      (provider) => provider.isDefault && provider.target === "codex",
    );

    expect(claudeRows).toHaveLength(1);
    expect(codexRows).toHaveLength(1);
    expect(claudeRows[0]?.id).toBe(mod.PASEO_MANAGED_CLAUDE_PROVIDER_ID);
    expect(codexRows[0]?.id).toBe(mod.PASEO_MANAGED_CODEX_PROVIDER_ID);
    expect(providers.activeClaudeProviderId).toBe(mod.PASEO_MANAGED_CLAUDE_PROVIDER_ID);
    expect(providers.activeCodexProviderId).toBe(mod.PASEO_MANAGED_CODEX_PROVIDER_ID);
  });
});

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
    const i18n = await import("../i18n/desktop-i18n");
    i18n.__setDesktopLocaleForTests(null);
    if (mockedHome.dir) {
      await rm(mockedHome.dir, { recursive: true, force: true });
    }
  });

  it("localizes missing provider errors", async () => {
    const i18n = await import("../i18n/desktop-i18n");
    i18n.__setDesktopLocaleForTests("zh");
    const mod = await import("./provider-switch");

    await expect(mod.switchProvider("missing-provider")).rejects.toThrow(
      "未找到提供商：missing-provider",
    );
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
    const storePath = join(mockedHome.dir, ".agent-client", "providers.json");

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

  it("merges existing Claude settings without changing model or permissions", async () => {
    const mod = await import("./provider-switch");
    const claudePath = join(mockedHome.dir, ".claude", "settings.json");
    await mkdir(join(mockedHome.dir, ".claude"), { recursive: true });
    await writeFile(
      claudePath,
      JSON.stringify(
        {
          env: {
            ANTHROPIC_MODEL: "claude-sonnet-4-5",
            SOME_OTHER_KEY: "keep-me",
          },
          permissions: {
            allow: ["Bash(git status)"],
          },
        },
        null,
        2,
      ),
      "utf8",
    );

    await mod.setupDefaultProvider({
      endpoint: "https://api.example.com",
      apiKey: "sk-new-claude",
      name: "Claude switch",
      scope: "claude",
    });

    const claudeSettings = JSON.parse(await readFile(claudePath, "utf8")) as {
      env?: Record<string, unknown>;
      permissions?: Record<string, unknown>;
    };
    expect(claudeSettings.env).toEqual({
      ANTHROPIC_MODEL: "claude-sonnet-4-5",
      SOME_OTHER_KEY: "keep-me",
      ANTHROPIC_BASE_URL: "https://api.example.com",
      ANTHROPIC_AUTH_TOKEN: "sk-new-claude",
      CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
      CLAUDE_CODE_ATTRIBUTION_HEADER: "0",
    });
    expect(claudeSettings.permissions).toEqual({
      allow: ["Bash(git status)"],
    });
  });

  it("does not overwrite non-empty invalid Claude settings JSON", async () => {
    const mod = await import("./provider-switch");
    const claudePath = join(mockedHome.dir, ".claude", "settings.json");
    await mkdir(join(mockedHome.dir, ".claude"), { recursive: true });
    await writeFile(claudePath, "{ invalid json", "utf8");

    await expect(
      mod.setupDefaultProvider({
        endpoint: "https://api.example.com",
        apiKey: "sk-new-claude",
        name: "Claude switch",
        scope: "claude",
      }),
    ).rejects.toThrow("Claude Code settings.json contains invalid JSON");

    expect(await readFile(claudePath, "utf8")).toBe("{ invalid json");
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
    const storePath = join(mockedHome.dir, ".agent-client", "providers.json");

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

  it("merges existing Codex auth and config without changing model preferences", async () => {
    const mod = await import("./provider-switch");
    const codexDir = join(mockedHome.dir, ".codex");
    const codexAuthPath = join(codexDir, "auth.json");
    const codexConfigPath = join(codexDir, "config.toml");
    await mkdir(codexDir, { recursive: true });
    await writeFile(
      codexAuthPath,
      JSON.stringify(
        {
          OPENAI_API_KEY: "old-key",
          tokens: {
            refresh: "keep-me",
          },
        },
        null,
        2,
      ),
      "utf8",
    );
    await writeFile(
      codexConfigPath,
      `# preserve this comment
model_provider = "Local"
model = "gpt-5.2-codex"
review_model = "gpt-5.2-codex"
model_reasoning_effort = "medium"

[model_providers.OpenAI]
name = "Old"
base_url = "https://old.example.com/v1"
wire_api = "chat"
requires_openai_auth = false

[profiles.work]
model = "profile-model"
`,
      "utf8",
    );

    await mod.setupDefaultProvider({
      endpoint: "https://api.example.com",
      apiKey: "sk-new-codex",
      name: "Codex switch",
      scope: "codex",
    });

    const codexAuth = JSON.parse(await readFile(codexAuthPath, "utf8")) as {
      OPENAI_API_KEY?: string;
      tokens?: Record<string, unknown>;
    };
    const codexConfig = await readFile(codexConfigPath, "utf8");

    expect(codexAuth).toEqual({
      OPENAI_API_KEY: "sk-new-codex",
      tokens: {
        refresh: "keep-me",
      },
    });
    expect(codexConfig).toContain("# preserve this comment");
    expect(codexConfig).toContain('model_provider = "OpenAI"');
    expect(codexConfig).toContain('model = "gpt-5.2-codex"');
    expect(codexConfig).toContain('review_model = "gpt-5.2-codex"');
    expect(codexConfig).toContain('model_reasoning_effort = "medium"');
    expect(codexConfig).toContain('base_url = "https://api.example.com/v1"');
    expect(codexConfig).toContain('wire_api = "responses"');
    expect(codexConfig).toContain("requires_openai_auth = true");
    expect(codexConfig).toContain("[profiles.work]");
    expect(codexConfig).toContain('model = "profile-model"');
    expect(codexConfig).not.toContain("https://old.example.com/v1");
  });

  it("does not overwrite Codex config when auth JSON is invalid", async () => {
    const mod = await import("./provider-switch");
    const codexDir = join(mockedHome.dir, ".codex");
    const codexAuthPath = join(codexDir, "auth.json");
    const codexConfigPath = join(codexDir, "config.toml");
    await mkdir(codexDir, { recursive: true });
    await writeFile(codexAuthPath, "{ invalid json", "utf8");
    await writeFile(codexConfigPath, 'model = "keep-me"\n', "utf8");

    await expect(
      mod.setupDefaultProvider({
        endpoint: "https://api.example.com",
        apiKey: "sk-new-codex",
        name: "Codex switch",
        scope: "codex",
      }),
    ).rejects.toThrow("Codex auth.json contains invalid JSON");

    expect(await readFile(codexAuthPath, "utf8")).toBe("{ invalid json");
    expect(await readFile(codexConfigPath, "utf8")).toBe('model = "keep-me"\n');
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
    const storePath = join(mockedHome.dir, ".agent-client", "providers.json");
    await mkdir(join(mockedHome.dir, ".agent-client"), { recursive: true });
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
    const storePath = join(mockedHome.dir, ".agent-client", "providers.json");
    await mkdir(join(mockedHome.dir, ".agent-client"), { recursive: true });
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

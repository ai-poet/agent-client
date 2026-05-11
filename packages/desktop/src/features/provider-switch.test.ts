import { describe, expect, it } from "vitest";
import {
  buildClaudeSettings,
  buildCodexConfig,
  DEFAULT_PROVIDER_ID,
  DEFAULT_PROVIDER_NAME,
  PASEO_MANAGED_CLAUDE_PROVIDER_ID,
  PASEO_MANAGED_CODEX_PROVIDER_ID,
  type Provider,
} from "./provider-switch";

function createProvider(overrides: Partial<Provider> = {}): Provider {
  return {
    id: DEFAULT_PROVIDER_ID,
    name: DEFAULT_PROVIDER_NAME,
    type: "default",
    endpoint: "https://api.example.com/v1",
    apiKey: "sk-live-example",
    isDefault: true,
    ...overrides,
  };
}

describe("provider-switch", () => {
  it("writes default Claude rows with only minimal integration-guide env keys", () => {
    const settings = buildClaudeSettings(createProvider(), {});

    expect(settings).toEqual({
      env: {
        ANTHROPIC_BASE_URL: "https://api.example.com",
        ANTHROPIC_AUTH_TOKEN: "sk-live-example",
        CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
        CLAUDE_CODE_ATTRIBUTION_HEADER: "0",
      },
    });
  });

  it("writes managed Claude settings using only the minimal integration-guide env keys", () => {
    const settings = buildClaudeSettings(
      createProvider({
        id: PASEO_MANAGED_CLAUDE_PROVIDER_ID,
        target: "claude",
        claudeConfig: {
          env: {
            ANTHROPIC_MODEL: "claude-opus-4-7",
          },
        },
      }),
      {
        env: {
          SOME_OTHER_KEY: "keep-me",
        },
        random: true,
      },
    );

    expect(settings).toEqual({
      env: {
        ANTHROPIC_BASE_URL: "https://api.example.com",
        ANTHROPIC_AUTH_TOKEN: "sk-live-example",
        CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
        CLAUDE_CODE_ATTRIBUTION_HEADER: "0",
      },
    });
  });

  it("preserves Claude Code Git Bash path on Windows when switching managed Claude keys", () => {
    const settings = buildClaudeSettings(
      createProvider({
        id: PASEO_MANAGED_CLAUDE_PROVIDER_ID,
        target: "claude",
      }),
      {
        env: {
          CLAUDE_CODE_GIT_BASH_PATH: "C:\\Program Files\\Git\\bin\\bash.exe",
          SOME_OTHER_KEY: "drop-me",
        },
      },
      { platform: "win32" },
    );

    expect(settings).toEqual({
      env: {
        ANTHROPIC_BASE_URL: "https://api.example.com",
        ANTHROPIC_AUTH_TOKEN: "sk-live-example",
        CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
        CLAUDE_CODE_ATTRIBUTION_HEADER: "0",
        CLAUDE_CODE_GIT_BASH_PATH: "C:\\Program Files\\Git\\bin\\bash.exe",
      },
    });
  });

  it("does not add or preserve Claude Code Git Bash path outside Windows", () => {
    const settings = buildClaudeSettings(
      createProvider({
        id: PASEO_MANAGED_CLAUDE_PROVIDER_ID,
        target: "claude",
      }),
      {
        env: {
          CLAUDE_CODE_GIT_BASH_PATH: "C:\\Program Files\\Git\\bin\\bash.exe",
        },
      },
      { platform: "darwin" },
    );

    expect(settings).toEqual({
      env: {
        ANTHROPIC_BASE_URL: "https://api.example.com",
        ANTHROPIC_AUTH_TOKEN: "sk-live-example",
        CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
        CLAUDE_CODE_ATTRIBUTION_HEADER: "0",
      },
    });
  });

  it("uses a newly detected Windows Git Bash path over a stale existing value", () => {
    const settings = buildClaudeSettings(
      createProvider({
        id: PASEO_MANAGED_CLAUDE_PROVIDER_ID,
        target: "claude",
      }),
      {
        env: {
          CLAUDE_CODE_GIT_BASH_PATH: "C:\\Old\\Git\\bin\\bash.exe",
        },
      },
      {
        platform: "win32",
        gitBashPath: "C:\\Program Files\\Git\\bin\\bash.exe",
      },
    );

    expect(settings).toMatchObject({
      env: {
        CLAUDE_CODE_GIT_BASH_PATH: "C:\\Program Files\\Git\\bin\\bash.exe",
      },
    });
  });

  it("preserves Claude Code Git Bash path for custom Claude rows on Windows", () => {
    const settings = buildClaudeSettings(
      createProvider({
        id: "custom-claude",
        type: "custom",
        isDefault: false,
        target: "claude",
        claudeConfig: {
          env: {
            ANTHROPIC_MODEL: "claude-sonnet-4-5",
          },
        },
      }),
      {
        env: {
          CLAUDE_CODE_GIT_BASH_PATH: "C:\\Program Files\\Git\\bin\\bash.exe",
        },
      },
      { platform: "win32" },
    );

    expect(settings).toMatchObject({
      env: {
        ANTHROPIC_MODEL: "claude-sonnet-4-5",
        CLAUDE_CODE_GIT_BASH_PATH: "C:\\Program Files\\Git\\bin\\bash.exe",
      },
    });
  });

  it("keeps explicit model env overrides for custom Claude rows", () => {
    const settings = buildClaudeSettings(
      createProvider({
        id: "custom-claude",
        type: "custom",
        isDefault: false,
        target: "claude",
        claudeConfig: {
          env: {
            ANTHROPIC_MODEL: "claude-sonnet-4-5",
            ANTHROPIC_DEFAULT_OPUS_MODEL: "claude-opus-4-7",
          },
        },
      }),
      {},
    );

    expect(settings).toMatchObject({
      env: {
        ANTHROPIC_BASE_URL: "https://api.example.com",
        ANTHROPIC_AUTH_TOKEN: "sk-live-example",
        ANTHROPIC_MODEL: "claude-sonnet-4-5",
        ANTHROPIC_DEFAULT_OPUS_MODEL: "claude-opus-4-7",
      },
    });
  });

  it("writes Codex config in the integration-guide format", () => {
    const config = buildCodexConfig(createProvider());

    expect(config).toContain('model_provider = "OpenAI"');
    expect(config).toContain('model = "gpt-5.4"');
    expect(config).toContain('review_model = "gpt-5.4"');
    expect(config).toContain('model_reasoning_effort = "xhigh"');
    expect(config).toContain("disable_response_storage = true");
    expect(config).toContain(`[model_providers.OpenAI]`);
    expect(config).toContain('name = "OpenAI"');
    expect(config).toContain('base_url = "https://api.example.com/v1"');
    expect(config).not.toContain("/v1/v1");
  });

  it("forces managed Codex rows to use gpt-5.4 template even if stale custom config exists", () => {
    const config = buildCodexConfig(
      createProvider({
        id: PASEO_MANAGED_CODEX_PROVIDER_ID,
        target: "codex",
        codexConfig: 'model = "claude-opus-4-7"\n',
      }),
    );

    expect(config).toContain('model = "gpt-5.4"');
    expect(config).toContain('review_model = "gpt-5.4"');
    expect(config).not.toContain('model = "claude-opus-4-7"');
  });
});

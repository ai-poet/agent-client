import { describe, expect, it } from "vitest";
import {
  buildByokClaudeProviderFromDisk,
  buildByokCodexProviderFromDisk,
  buildClaudeSettings,
  buildCodexAuth,
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

  it("merges managed Claude settings while only changing the managed env keys", () => {
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
          ANTHROPIC_MODEL: "claude-sonnet-4-5",
        },
        permissions: {
          allow: ["Bash(git status)"],
        },
        random: true,
      },
    );

    expect(settings).toEqual({
      env: {
        SOME_OTHER_KEY: "keep-me",
        ANTHROPIC_MODEL: "claude-sonnet-4-5",
        ANTHROPIC_BASE_URL: "https://api.example.com",
        ANTHROPIC_AUTH_TOKEN: "sk-live-example",
        CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
        CLAUDE_CODE_ATTRIBUTION_HEADER: "0",
      },
      permissions: {
        allow: ["Bash(git status)"],
      },
      random: true,
    });
  });

  it("preserves existing Claude env keys when switching managed Claude keys", () => {
    const settings = buildClaudeSettings(
      createProvider({
        id: PASEO_MANAGED_CLAUDE_PROVIDER_ID,
        target: "claude",
      }),
      {
        env: {
          CLAUDE_CODE_GIT_BASH_PATH: "C:\\Program Files\\Git\\bin\\bash.exe",
          SOME_OTHER_KEY: "keep-me",
        },
      },
    );

    expect(settings).toEqual({
      env: {
        CLAUDE_CODE_GIT_BASH_PATH: "C:\\Program Files\\Git\\bin\\bash.exe",
        SOME_OTHER_KEY: "keep-me",
        ANTHROPIC_BASE_URL: "https://api.example.com",
        ANTHROPIC_AUTH_TOKEN: "sk-live-example",
        CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
        CLAUDE_CODE_ATTRIBUTION_HEADER: "0",
      },
    });
  });

  it("preserves Claude Code Git Bash path outside Windows because provider switching only patches managed keys", () => {
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
    );

    expect(settings).toEqual({
      env: {
        CLAUDE_CODE_GIT_BASH_PATH: "C:\\Program Files\\Git\\bin\\bash.exe",
        ANTHROPIC_BASE_URL: "https://api.example.com",
        ANTHROPIC_AUTH_TOKEN: "sk-live-example",
        CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
        CLAUDE_CODE_ATTRIBUTION_HEADER: "0",
      },
    });
  });

  it("does not update Claude Code Git Bash path during provider switching", () => {
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
    );

    expect(settings).toMatchObject({
      env: {
        CLAUDE_CODE_GIT_BASH_PATH: "C:\\Old\\Git\\bin\\bash.exe",
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

  it("uses the default Codex template for empty config even if managed row has stale custom config", () => {
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

  it("merges Codex auth by updating the API key and preserving other fields", () => {
    expect(
      buildCodexAuth(createProvider(), {
        OPENAI_API_KEY: "old-key",
        tokens: {
          refresh: "keep-me",
        },
      }),
    ).toEqual({
      OPENAI_API_KEY: "sk-live-example",
      tokens: {
        refresh: "keep-me",
      },
    });
  });

  it("patches existing Codex config without changing user model preferences", () => {
    const config = buildCodexConfig(
      createProvider({
        endpoint: "https://gateway.example.com",
      }),
      `# user choices
model_provider = "Local"
model = "gpt-5.2-codex"
review_model = "gpt-5.2-codex"
model_reasoning_effort = "medium"

[model_providers.Local]
name = "Local"
base_url = "http://localhost:11434/v1"
wire_api = "responses"

[model_providers.OpenAI]
name = "Old OpenAI"
base_url = "https://old.example.com/v1"
wire_api = "chat"
requires_openai_auth = false

[profiles.work]
model = "custom-profile-model"
`,
    );

    expect(config).toContain("# user choices");
    expect(config).toContain('model_provider = "OpenAI"');
    expect(config).toContain('model = "gpt-5.2-codex"');
    expect(config).toContain('review_model = "gpt-5.2-codex"');
    expect(config).toContain('model_reasoning_effort = "medium"');
    expect(config).toContain('[model_providers.Local]');
    expect(config).toContain('[profiles.work]');
    expect(config).toContain('model = "custom-profile-model"');
    expect(config).toContain('name = "OpenAI"');
    expect(config).toContain('base_url = "https://gateway.example.com/v1"');
    expect(config).toContain('wire_api = "responses"');
    expect(config).toContain("requires_openai_auth = true");
    expect(config).not.toContain("https://old.example.com/v1");
  });

  it("builds BYOK Claude provider from disk config with correct target and format", () => {
    const provider = buildByokClaudeProviderFromDisk({
      baseUrl: "https://api.anthropic.com",
      apiKey: "sk-ant-test",
    });

    expect(provider).toEqual({
      id: "byok-local-claude",
      name: "Local Claude Code",
      type: "custom",
      endpoint: "https://api.anthropic.com",
      apiKey: "sk-ant-test",
      isDefault: false,
      target: "claude",
      claudeApiFormat: "anthropic",
    });
  });

  it("builds BYOK Codex provider from disk config with resolved base_url", () => {
    const provider = buildByokCodexProviderFromDisk({
      apiKey: "sk-openai-test",
      configToml: `model_provider = "OpenAI"
[model_providers.OpenAI]
base_url = "https://api.openai.com/v1"
wire_api = "responses"
`,
    });

    expect(provider).toMatchObject({
      id: "byok-local-codex",
      name: "Local Codex",
      type: "custom",
      endpoint: "https://api.openai.com",
      apiKey: "sk-openai-test",
      isDefault: false,
      target: "codex",
      codexWireApi: "responses",
    });
  });

  it("builds BYOK Codex provider with empty endpoint when base_url is missing", () => {
    const provider = buildByokCodexProviderFromDisk({
      apiKey: "sk-openai-test",
      configToml: 'model_provider = "OpenAI"\n',
    });

    expect(provider.id).toBe("byok-local-codex");
    expect(provider.endpoint).toBe("");
    expect(provider.target).toBe("codex");
  });
});

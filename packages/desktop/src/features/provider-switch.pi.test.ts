import { describe, expect, it } from "vitest";
import { PI_MANAGED_PROVIDER_NAME, buildPiAuthJson, buildPiModelsJson } from "./provider-switch";

const ENDPOINT = "https://gateway.example.com/v1";

function providersOf(config: Record<string, unknown>): Record<string, unknown> {
  return config.providers as Record<string, unknown>;
}

function managedProvider(config: Record<string, unknown>): Record<string, unknown> {
  return providersOf(config)[PI_MANAGED_PROVIDER_NAME] as Record<string, unknown>;
}

describe("buildPiModelsJson", () => {
  it("registers the gateway as an OpenAI-compatible provider", () => {
    const config = buildPiModelsJson({ endpoint: ENDPOINT, models: ["gpt-5.4"] });

    expect(managedProvider(config)).toMatchObject({
      baseUrl: ENDPOINT,
      api: "openai-completions",
    });
  });

  it("lists every gateway model, not just one family", () => {
    const config = buildPiModelsJson({
      endpoint: ENDPOINT,
      models: ["gpt-5.4", "claude-opus-5", "grok-4.5"],
    });

    expect(managedProvider(config).models).toEqual([
      { id: "gpt-5.4", name: "gpt-5.4" },
      { id: "claude-opus-5", name: "claude-opus-5" },
      { id: "grok-4.5", name: "grok-4.5" },
    ]);
  });

  it("preserves other providers the user configured", () => {
    const config = buildPiModelsJson({
      endpoint: ENDPOINT,
      models: ["gpt-5.4"],
      existing: {
        providers: { ollama: { baseUrl: "http://localhost:11434/v1" } },
      },
    });

    expect(providersOf(config).ollama).toEqual({ baseUrl: "http://localhost:11434/v1" });
    expect(managedProvider(config)).toBeDefined();
  });

  it("preserves unrelated top-level keys", () => {
    const config = buildPiModelsJson({
      endpoint: ENDPOINT,
      models: [],
      existing: { $schema: "https://pi.dev/models.schema.json" },
    });

    expect(config.$schema).toBe("https://pi.dev/models.schema.json");
  });

  it("replaces our own previous entry rather than accumulating", () => {
    const first = buildPiModelsJson({ endpoint: ENDPOINT, models: ["old-model"] });
    const second = buildPiModelsJson({
      endpoint: "https://other.example.com/v1",
      models: ["new-model"],
      existing: first,
    });

    expect(managedProvider(second)).toMatchObject({ baseUrl: "https://other.example.com/v1" });
    expect(managedProvider(second).models).toEqual([{ id: "new-model", name: "new-model" }]);
  });

  it("never writes the key into models.json", () => {
    const config = buildPiModelsJson({ endpoint: ENDPOINT, models: ["gpt-5.4"] });

    expect(JSON.stringify(config)).not.toContain("apiKey");
  });

  it("survives a corrupt providers value instead of throwing", () => {
    const config = buildPiModelsJson({
      endpoint: ENDPOINT,
      models: [],
      existing: { providers: "not-an-object" },
    });

    expect(managedProvider(config)).toBeDefined();
  });
});

describe("buildPiAuthJson", () => {
  it("stores the key under the managed provider name", () => {
    expect(buildPiAuthJson({ apiKey: "sk-test" })).toEqual({
      [PI_MANAGED_PROVIDER_NAME]: { type: "api_key", key: "sk-test" },
    });
  });

  it("leaves credentials for other providers untouched", () => {
    const auth = buildPiAuthJson({
      apiKey: "sk-test",
      existing: { anthropic: { type: "api_key", key: "sk-user-owned" } },
    });

    expect(auth.anthropic).toEqual({ type: "api_key", key: "sk-user-owned" });
  });
});

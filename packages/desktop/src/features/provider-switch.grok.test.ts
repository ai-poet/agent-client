import { describe, expect, it } from "vitest";
import { GROK_GATEWAY_KEY_ENV, buildGrokConfigToml } from "./provider-switch";

const SECRET = "sk-super-secret-value";

describe("buildGrokConfigToml", () => {
  it("emits one model block per gateway model", () => {
    const toml = buildGrokConfigToml({
      endpoint: "https://gateway.example.com/v1",
      models: ["grok-4.5", "grok-build"],
    });

    expect(toml).toContain('[model."grok-4.5"]');
    expect(toml).toContain('[model."grok-build"]');
    expect(toml).toContain('base_url = "https://gateway.example.com/v1"');
  });

  it("pins the Responses wire format the gateway serves", () => {
    const toml = buildGrokConfigToml({
      endpoint: "https://gateway.example.com/v1",
      models: ["grok-4.5"],
    });

    expect(toml).toContain('api_backend = "responses"');
  });

  it("references the key by env var and never writes it to disk", () => {
    const toml = buildGrokConfigToml({
      endpoint: "https://gateway.example.com/v1",
      models: ["grok-4.5"],
    });

    // This is the point of the whole indirection.
    expect(toml).not.toContain(SECRET);
    expect(toml).not.toContain("api_key");
    expect(toml).toContain(`env_key = "${GROK_GATEWAY_KEY_ENV}"`);
  });

  it("sets a default model when one is supplied", () => {
    const toml = buildGrokConfigToml({
      endpoint: "https://gateway.example.com/v1",
      models: ["grok-4.5", "grok-build"],
      defaultModel: "grok-4.5",
    });

    expect(toml).toContain("[models]");
    expect(toml).toContain('default = "grok-4.5"');
  });

  it("omits the default block when no model is supplied", () => {
    const toml = buildGrokConfigToml({ endpoint: "https://gateway.example.com/v1", models: [] });

    expect(toml).not.toContain("[models]");
    expect(toml).not.toContain("[model.");
  });

  it("escapes quotes so a hostile model id cannot break out of the string", () => {
    const toml = buildGrokConfigToml({
      endpoint: 'https://example.com/"injected',
      models: ['grok-"evil'],
    });

    expect(toml).toContain('[model."grok-\\"evil"]');
    expect(toml).toContain('base_url = "https://example.com/\\"injected"');
  });

  it("marks the file as managed so hand edits are not silently lost", () => {
    const toml = buildGrokConfigToml({ endpoint: "https://example.com/v1", models: ["grok-4.5"] });

    expect(toml.startsWith("# Managed by Agent Client")).toBe(true);
  });
});

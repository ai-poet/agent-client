import { describe, expect, it } from "vitest";
import { buildGrokConfigToml } from "./provider-switch";

const ENDPOINT = "https://cheaprouter.org/v1";
const KEY = "sk-gateway-key";

function build(models: string[], existingToml?: string | null): string {
  return buildGrokConfigToml({ endpoint: ENDPOINT, apiKey: KEY, models, existingToml });
}

describe("buildGrokConfigToml", () => {
  it("emits one block per gateway model, keyed by the real model id", () => {
    const toml = build(["grok-4.6", "grok-4.5"]);

    expect(toml).toContain('[model."grok-4.6"]');
    expect(toml).toContain('[model."grok-4.5"]');
  });

  it("quotes model ids so a dotted version cannot become a nested table", () => {
    // `[model.grok-4.6]` would parse as model -> grok-4 -> 6.
    expect(build(["grok-4.6"])).not.toContain("[model.grok-4.6]");
  });

  it("writes the routing fields the CLI needs", () => {
    const toml = build(["grok-4.6"]);

    expect(toml).toContain('model = "grok-4.6"');
    expect(toml).toContain(`base_url = "${ENDPOINT}"`);
    expect(toml).toContain('api_backend = "responses"');
    expect(toml).toContain(`api_key = "${KEY}"`);
    expect(toml).toContain("context_window = 500000");
    expect(toml).toContain("supports_reasoning_effort = true");
    expect(toml).toContain('reasoning_efforts = ["low", "medium", "high"]');
  });

  it("defaults to the first model", () => {
    const toml = build(["grok-4.6", "grok-4.5"]);

    expect(toml).toContain("[models]");
    expect(toml).toContain('default = "grok-4.6"');
    expect(toml).toContain('default_reasoning_effort = "high"');
  });

  it("marks the installer as internal", () => {
    expect(build(["grok-4.6"])).toContain('installer = "internal"');
  });

  it("seeds preference sections when creating the file", () => {
    const toml = build(["grok-4.6"]);

    expect(toml).toContain("[session]");
    expect(toml).toContain("auto_compact_threshold_percent = 85");
    expect(toml).toContain("[memory.session]");
    expect(toml).toContain('permission_mode = "always-approve"');
    expect(toml).toContain("[subagents]");
  });

  it("never touches preferences that already exist on disk", () => {
    const existing = `[ui]
yolo = true
permission_mode = "ask"

[session]
auto_compact_threshold_percent = 50
`;
    const toml = build(["grok-4.6"], existing);

    // The user's choices survive; only routing is rewritten.
    expect(toml).toContain("yolo = true");
    expect(toml).toContain('permission_mode = "ask"');
    expect(toml).toContain("auto_compact_threshold_percent = 50");
    expect(toml).toContain('[model."grok-4.6"]');
  });

  it("preserves unrelated sections when merging", () => {
    const existing = `[mcp.servers.local]
command = "my-server"
`;
    const toml = build(["grok-4.6"], existing);

    expect(toml).toContain("[mcp.servers.local]");
    expect(toml).toContain('command = "my-server"');
  });

  it("updates an existing model block in place instead of duplicating it", () => {
    const first = build(["grok-4.6"]);
    const second = buildGrokConfigToml({
      endpoint: "https://other.example.com/v1",
      apiKey: "sk-rotated",
      models: ["grok-4.6"],
      existingToml: first,
    });

    expect(second.match(/\[model\."grok-4\.6"\]/gu)).toHaveLength(1);
    expect(second).toContain('base_url = "https://other.example.com/v1"');
    expect(second).toContain('api_key = "sk-rotated"');
    expect(second).not.toContain(KEY);
  });

  it("leaves the default block out when there are no models", () => {
    const toml = build([]);

    expect(toml).not.toContain("[model.");
    expect(toml).not.toContain("[models]");
  });

  it("escapes quotes so a hostile model id cannot break out of the header", () => {
    const toml = build(['grok-"evil']);

    expect(toml).toContain('[model."grok-\\"evil"]');
  });

  it("always ends with a newline", () => {
    expect(build(["grok-4.6"]).endsWith("\n")).toBe(true);
  });
});

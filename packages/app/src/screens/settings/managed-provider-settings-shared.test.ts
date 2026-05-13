import { describe, expect, it } from "vitest";
import type { DesktopProviderPayload } from "@/screens/settings/sub2api-provider-types";
import {
  getCustomTargetSegmentOptions,
  providerTargetHint,
  providerWritesClaude,
  providerWritesCodex,
} from "./managed-provider-settings-shared";

function createProvider(overrides: Partial<DesktopProviderPayload> = {}): DesktopProviderPayload {
  return {
    id: "provider",
    name: "Provider",
    type: "default",
    endpoint: "https://api.example.com",
    apiKey: "sk-test",
    isDefault: true,
    ...overrides,
  };
}

describe("providerTargetHint", () => {
  it("describes managed Claude rows as Claude-only", () => {
    expect(providerTargetHint(createProvider({ target: "claude" }))).toBe(
      "Claude Code · Anthropic",
    );
  });

  it("describes managed Codex rows as Codex-only", () => {
    expect(providerTargetHint(createProvider({ target: "codex" }))).toBe("Codex · Responses");
  });

  it("flags legacy unscoped rows", () => {
    expect(providerTargetHint(createProvider({ target: undefined }))).toBe(
      "Legacy unscoped endpoint",
    );
  });
});

describe("provider write scopes", () => {
  it("writes Claude only when target is claude", () => {
    expect(providerWritesClaude(createProvider({ target: "claude" }))).toBe(true);
    expect(providerWritesCodex(createProvider({ target: "claude" }))).toBe(false);
  });

  it("writes Codex only when target is codex", () => {
    expect(providerWritesClaude(createProvider({ target: "codex" }))).toBe(false);
    expect(providerWritesCodex(createProvider({ target: "codex" }))).toBe(true);
  });

  it("does not expose legacy unscoped rows as usable write targets", () => {
    expect(providerWritesClaude(createProvider({ target: undefined }))).toBe(false);
    expect(providerWritesCodex(createProvider({ target: undefined }))).toBe(false);
  });
});

describe("getCustomTargetSegmentOptions", () => {
  it("builds localized segment labels", () => {
    expect(getCustomTargetSegmentOptions({ claude: "Claude Code", codex: "Codex" })).toEqual([
      { value: "claude", label: "Claude Code" },
      { value: "codex", label: "Codex" },
    ]);
  });
});

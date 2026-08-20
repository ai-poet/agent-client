import { describe, expect, it } from "vitest";
import type { DesktopProviderPayload } from "@/screens/settings/sub2api-provider-types";
import {
  getCustomTargetSegmentOptions,
  MANAGED_PROVIDER_TARGETS,
  providerTargetHint,
  providerWritesTarget,
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

  it("describes Grok rows as model-restricted", () => {
    expect(providerTargetHint(createProvider({ target: "grok" }))).toBe(
      "Grok · Responses · grok- models only",
    );
  });

  it("describes Pi rows as unrestricted", () => {
    expect(providerTargetHint(createProvider({ target: "pi" }))).toBe("Pi · all gateway models");
  });

  it("flags legacy unscoped rows", () => {
    expect(providerTargetHint(createProvider({ target: undefined }))).toBe(
      "Legacy unscoped endpoint",
    );
  });
});

describe("providerWritesTarget", () => {
  it("matches only the row's own target", () => {
    for (const target of MANAGED_PROVIDER_TARGETS) {
      const provider = createProvider({ target });
      for (const candidate of MANAGED_PROVIDER_TARGETS) {
        expect(providerWritesTarget(provider, candidate)).toBe(candidate === target);
      }
    }
  });

  it("does not expose legacy unscoped rows as usable write targets", () => {
    const legacy = createProvider({ target: undefined });
    for (const target of MANAGED_PROVIDER_TARGETS) {
      expect(providerWritesTarget(legacy, target)).toBe(false);
    }
  });
});

describe("getCustomTargetSegmentOptions", () => {
  const labels = { claude: "Claude Code", codex: "Codex", grok: "Grok", pi: "Pi" };

  it("offers every managed target with its localized label", () => {
    expect(
      getCustomTargetSegmentOptions(labels).map(({ value, label }) => ({ value, label })),
    ).toEqual([
      { value: "claude", label: "Claude Code" },
      { value: "codex", label: "Codex" },
      { value: "grok", label: "Grok" },
    ]);
  });

  it("does not offer Pi while it is hidden", () => {
    expect(getCustomTargetSegmentOptions(labels).map((option) => option.value)).not.toContain(
      "pi",
    );
  });

  it("stays free of component imports so it can run as a plain unit test", () => {
    // Icons are the panel's job; this module must not pull React Native into node tests.
    for (const option of getCustomTargetSegmentOptions(labels)) {
      expect(option.icon).toBeUndefined();
    }
  });
});

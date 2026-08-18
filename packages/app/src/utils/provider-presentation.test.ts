import { describe, expect, it } from "vitest";
import type { ProviderSnapshotEntry } from "@server/server/agent/agent-sdk-types";

import {
  canOfferInstall,
  classifyInstallOutcome,
  formatProviderMeta,
  resolveProviderStatusTone,
} from "./provider-presentation";

function makeEntry(overrides: Partial<ProviderSnapshotEntry> = {}): ProviderSnapshotEntry {
  return { provider: "grok", status: "ready", ...overrides };
}

const modelCountLabel = (count: number) => `${count} models`;

describe("resolveProviderStatusTone", () => {
  it("gives loading and unavailable distinct tones", () => {
    expect(resolveProviderStatusTone("loading")).toBe("pending");
    expect(resolveProviderStatusTone("unavailable")).toBe("warning");
    expect(resolveProviderStatusTone("loading")).not.toBe(resolveProviderStatusTone("unavailable"));
  });

  it("maps ready and error to their semantic tones", () => {
    expect(resolveProviderStatusTone("ready")).toBe("success");
    expect(resolveProviderStatusTone("error")).toBe("error");
  });
});

describe("formatProviderMeta", () => {
  it("combines version and model count", () => {
    const entry = makeEntry({
      version: "1.0.4",
      models: [
        { provider: "grok", id: "a", label: "A" },
        { provider: "grok", id: "b", label: "B" },
      ],
    });

    expect(formatProviderMeta(entry, modelCountLabel)).toBe("1.0.4 · 2 models");
  });

  it("omits the model count when the provider is not ready", () => {
    const entry = makeEntry({
      status: "unavailable",
      version: "1.0.4",
      models: [{ provider: "grok", id: "a", label: "A" }],
    });

    expect(formatProviderMeta(entry, modelCountLabel)).toBe("1.0.4");
  });

  it("returns null when there is nothing to show", () => {
    expect(formatProviderMeta(makeEntry(), modelCountLabel)).toBeNull();
    expect(formatProviderMeta(undefined, modelCountLabel)).toBeNull();
  });
});

describe("canOfferInstall", () => {
  it("offers install only for an unavailable provider that reported a command", () => {
    expect(
      canOfferInstall(makeEntry({ status: "unavailable", installCommand: "npm i -g x" })),
    ).toBe(true);
  });

  it("does not offer install without a command or when already working", () => {
    expect(canOfferInstall(makeEntry({ status: "unavailable" }))).toBe(false);
    expect(canOfferInstall(makeEntry({ status: "ready", installCommand: "npm i -g x" }))).toBe(
      false,
    );
    expect(canOfferInstall(undefined)).toBe(false);
  });
});

describe("classifyInstallOutcome", () => {
  it("reports a real install", () => {
    expect(
      classifyInstallOutcome({
        nextEntry: makeEntry({ status: "ready", version: "1.0.4" }),
      }),
    ).toEqual({ kind: "installed", version: "1.0.4" });
  });

  it("reports when the command finished but the binary is still undetected", () => {
    expect(classifyInstallOutcome({ nextEntry: makeEntry({ status: "unavailable" }) })).toEqual({
      kind: "not-detected",
    });
  });

  it("reports when an update left the version unchanged", () => {
    expect(
      classifyInstallOutcome({
        previousVersion: "1.0.4",
        nextEntry: makeEntry({ status: "ready", version: "1.0.4" }),
      }),
    ).toEqual({ kind: "unchanged", version: "1.0.4" });
  });

  it("treats a moved version as a successful update", () => {
    expect(
      classifyInstallOutcome({
        previousVersion: "1.0.3",
        nextEntry: makeEntry({ status: "ready", version: "1.0.4" }),
      }),
    ).toEqual({ kind: "installed", version: "1.0.4" });
  });
});

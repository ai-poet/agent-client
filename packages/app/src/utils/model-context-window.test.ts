import { describe, expect, it } from "vitest";
import {
  collapseModelContextVariants,
  getAvailableModelContextWindows,
  getBaseModelId,
  getModelContextWindow,
  resolveModelIdForContext,
} from "./model-context-window";

const claudeModels = [
  { id: "claude-opus-5[1m]", label: "Opus 5 1M", description: "Extended" },
  { id: "claude-opus-5", label: "Opus 5", description: "Standard", isDefault: true },
  { id: "claude-haiku-4-5", label: "Haiku 4.5" },
];

describe("model context windows", () => {
  it("separates a Claude model ID into its base model and context window", () => {
    expect(getBaseModelId("claude", "claude-opus-5[1m]")).toBe("claude-opus-5");
    expect(getModelContextWindow("claude", "claude-opus-5[1m]")).toBe("1m");
    expect(getModelContextWindow("claude", "claude-opus-5")).toBe("256k");
  });

  it("collapses Claude context variants into one model row using standard metadata", () => {
    expect(collapseModelContextVariants("claude", claudeModels)).toEqual([
      {
        id: "claude-opus-5",
        label: "Opus 5",
        description: "Standard",
        isDefault: true,
      },
      { id: "claude-haiku-4-5", label: "Haiku 4.5" },
    ]);
  });

  it("reports only context windows backed by real model variants", () => {
    expect(
      getAvailableModelContextWindows({
        provider: "claude",
        modelId: "claude-opus-5",
        models: claudeModels,
      }),
    ).toEqual(["256k", "1m"]);
    expect(
      getAvailableModelContextWindows({
        provider: "claude",
        modelId: "claude-haiku-4-5",
        models: claudeModels,
      }),
    ).toEqual(["256k"]);
  });

  it("maps a context choice back to the existing runtime model ID", () => {
    expect(
      resolveModelIdForContext({
        provider: "claude",
        modelId: "claude-opus-5",
        contextWindow: "1m",
        models: claudeModels,
      }),
    ).toBe("claude-opus-5[1m]");
    expect(
      resolveModelIdForContext({
        provider: "claude",
        modelId: "claude-haiku-4-5",
        contextWindow: "1m",
        models: claudeModels,
      }),
    ).toBe("claude-haiku-4-5");
  });

  it("leaves providers without context variants unchanged", () => {
    const codexModels = [{ id: "gpt-5.4", label: "GPT-5.4" }];
    expect(collapseModelContextVariants("codex", codexModels)).toBe(codexModels);
    expect(
      resolveModelIdForContext({
        provider: "codex",
        modelId: "gpt-5.4",
        contextWindow: "1m",
        models: codexModels,
      }),
    ).toBe("gpt-5.4");
  });
});

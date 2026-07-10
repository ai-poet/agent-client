import { describe, expect, it } from "vitest";

import { getCodexModels } from "./codex-models.js";

describe("getCodexModels", () => {
  it("matches the backend OpenAI default model catalog", () => {
    expect(getCodexModels().map((model) => model.id)).toEqual([
      "gpt-5.6-sol",
      "gpt-5.6-terra",
      "gpt-5.6-luna",
      "gpt-5.5",
      "gpt-5.4",
      "gpt-5.4-mini",
      "gpt-5.4-nano",
      "gpt-5.3-codex",
      "gpt-5.3-codex-spark",
      "gpt-5.2",
      "gpt-5.2-codex",
      "gpt-5.1-codex-max",
      "gpt-5.1-codex",
      "gpt-5.1",
      "gpt-5.1-codex-mini",
      "gpt-5",
    ]);
  });

  it("keeps GPT-5.4 as the single hardcoded default", () => {
    expect(
      getCodexModels()
        .filter((model) => model.isDefault)
        .map((model) => model.id),
    ).toEqual(["gpt-5.4"]);
  });

  it.each([
    ["gpt-5.6-sol", "low", ["low", "medium", "high", "xhigh", "max", "ultra"]],
    ["gpt-5.6-terra", "medium", ["low", "medium", "high", "xhigh", "max", "ultra"]],
    ["gpt-5.6-luna", "medium", ["low", "medium", "high", "xhigh", "max"]],
  ])("provides the complete reasoning metadata for %s", (id, defaultId, optionIds) => {
    const model = getCodexModels().find((candidate) => candidate.id === id);

    expect(model?.defaultThinkingOptionId).toBe(defaultId);
    expect(model?.thinkingOptions?.map((option) => option.id)).toEqual(optionIds);
    expect(
      model?.thinkingOptions?.filter((option) => option.isDefault).map((option) => option.id),
    ).toEqual([defaultId]);
  });

  it("does not include an unconfirmed GPT-5.6 Pro model", () => {
    expect(getCodexModels().some((model) => model.id === "gpt-5.6-pro")).toBe(false);
  });

  it("returns independent model and thinking option objects", () => {
    const first = getCodexModels();
    const second = getCodexModels();

    expect(first).not.toBe(second);
    expect(first[0]).not.toBe(second[0]);
    expect(first[0]!.thinkingOptions).not.toBe(second[0]!.thinkingOptions);
  });
});

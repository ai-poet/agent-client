import { describe, expect, it } from "vitest";

import { getCodexModels } from "./codex-models.js";

describe("getCodexModels", () => {
  it("returns the supported hardcoded Codex model catalog", () => {
    expect(getCodexModels().map((model) => model.id)).toEqual([
      "gpt-5.6-sol",
      "gpt-5.6-terra",
      "gpt-5.6-luna",
      "gpt-5.5",
      "gpt-5.4",
      "gpt-5.4-mini",
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

  it("excludes GPT-5.4 Nano and all models below GPT-5.4", () => {
    const modelIds = getCodexModels().map((model) => model.id);

    expect(modelIds).not.toContain("gpt-5.4-nano");
    expect(modelIds.some((id) => /^(?:gpt-5(?:$|\.[0-3](?:$|-)))/.test(id))).toBe(false);
  });

  it("returns independent model and thinking option objects", () => {
    const first = getCodexModels();
    const second = getCodexModels();

    expect(first).not.toBe(second);
    expect(first[0]).not.toBe(second[0]);
    expect(first[0]!.thinkingOptions).not.toBe(second[0]!.thinkingOptions);
  });
});

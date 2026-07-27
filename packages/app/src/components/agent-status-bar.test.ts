import { describe, expect, it } from "vitest";
import type { AgentModelDefinition } from "@server/server/agent/agent-sdk-types";
import {
  cloudGroupsForStatusProvider,
  filterSelectableProviderDefinitions,
  getFeatureHighlightColor,
  getFeatureTooltip,
  getStatusSelectorHint,
  resolveCloudGroupDisplayLabel,
  normalizeModelId,
  resolveAgentModelSelection,
  scopeModelsToProvider,
} from "./agent-status-bar.utils";

describe("getStatusSelectorHint", () => {
  it("explains what each editable status control does", () => {
    expect(getStatusSelectorHint("thinking")).toBe("Thinking mode");
    expect(getStatusSelectorHint("model")).toBe("Change model");
    expect(getStatusSelectorHint("context")).toBe("Change context window");
    expect(getStatusSelectorHint("mode")).toBe("Change permission mode");
    expect(getStatusSelectorHint("cloud-group")).toBe("Cloud group");
  });
});

describe("cloud group status helpers", () => {
  const groups = [
    {
      provider: "claude",
      groupId: 10,
      groupLabel: "Claude Fast",
      platform: "anthropic",
      description: "Claude · 0.8x · up",
      isActiveForWorkspace: false,
      models: [],
    },
    {
      provider: "claude",
      groupId: 12,
      groupLabel: "GLM Pay As You Go",
      platform: "anthropic",
      description: "Claude Code · global CLI key",
      isActiveForGlobalKey: true,
      models: [],
    },
  ];

  it("resolves the active Cloud group as read-only status", () => {
    expect(resolveCloudGroupDisplayLabel(groups, "claude")).toBe("GLM Pay As You Go");
    expect(resolveCloudGroupDisplayLabel(groups, "codex")).toBeNull();
  });

  it("keeps only Cloud groups for the selected provider", () => {
    expect(
      cloudGroupsForStatusProvider(
        [...groups, { ...groups[0]!, provider: "codex", groupId: 20 }],
        "claude",
      ).map((group) => group.groupId),
    ).toEqual([10, 12]);
  });
});

describe("draft provider and model controls", () => {
  const providerDefinitions = [
    {
      id: "claude",
      label: "Claude",
      description: "Claude provider",
      defaultModeId: "default",
      modes: [],
    },
    {
      id: "codex",
      label: "Codex",
      description: "Codex provider",
      defaultModeId: "auto",
      modes: [],
    },
  ];
  const claudeModels = [{ provider: "claude" as const, id: "claude-opus-5", label: "Opus 5" }];
  const codexModels = [{ provider: "codex" as const, id: "gpt-5.4", label: "GPT-5.4" }];

  it("offers only provider snapshots that are ready", () => {
    expect(
      filterSelectableProviderDefinitions(providerDefinitions, ["claude"]).map(
        (definition) => definition.id,
      ),
    ).toEqual(["claude"]);
  });

  it("keeps the model selector scoped to the selected provider", () => {
    const scoped = scopeModelsToProvider(
      "claude",
      new Map<string, AgentModelDefinition[]>([
        ["claude", claudeModels],
        ["codex", codexModels],
      ]),
      [],
    );

    expect(Array.from(scoped.keys())).toEqual(["claude"]);
    expect(scoped.get("claude")?.map((model) => model.id)).toEqual(["claude-opus-5"]);
    expect(scoped.has("codex")).toBe(false);
  });

  it("uses selected-provider fallback models while its snapshot is absent", () => {
    const scoped = scopeModelsToProvider("codex", new Map(), codexModels);

    expect(scoped.get("codex")).toEqual(codexModels);
  });
});

describe("feature metadata helpers", () => {
  it("prefers explicit feature tooltip copy", () => {
    expect(
      getFeatureTooltip({
        label: "Plan",
        tooltip: "Toggle plan mode",
      }),
    ).toBe("Toggle plan mode");
  });

  it("falls back to the feature label when no tooltip is provided", () => {
    expect(
      getFeatureTooltip({
        label: "Custom",
      }),
    ).toBe("Custom");
  });

  it("maps feature highlight colors by feature id", () => {
    expect(getFeatureHighlightColor("fast_mode")).toBe("yellow");
    expect(getFeatureHighlightColor("plan_mode")).toBe("blue");
    expect(getFeatureHighlightColor("other")).toBe("default");
  });
});

describe("normalizeModelId", () => {
  it("treats empty values as unset", () => {
    expect(normalizeModelId("")).toBeNull();
    expect(normalizeModelId(undefined)).toBeNull();
  });

  it("returns trimmed model ids", () => {
    expect(normalizeModelId(" gpt-5.1-codex ")).toBe("gpt-5.1-codex");
    expect(normalizeModelId(" default ")).toBe("default");
  });
});

describe("resolveAgentModelSelection", () => {
  it("prefers runtime model over configured model", () => {
    const selection = resolveAgentModelSelection({
      models: [
        {
          id: "a",
          provider: "codex",
          label: "Model A",
          thinkingOptions: [{ id: "low", label: "Low" }],
          defaultThinkingOptionId: "low",
        },
      ],
      runtimeModelId: "a",
      configuredModelId: "b",
      explicitThinkingOptionId: null,
    });

    expect(selection.activeModelId).toBe("a");
    expect(selection.displayModel).toBe("Model A");
    expect(selection.selectedThinkingId).toBe("low");
  });

  it("uses explicit thinking option when provided", () => {
    const selection = resolveAgentModelSelection({
      models: [
        {
          id: "a",
          provider: "codex",
          label: "Model A",
          thinkingOptions: [
            { id: "low", label: "Low" },
            { id: "high", label: "High" },
          ],
          defaultThinkingOptionId: "low",
        },
      ],
      runtimeModelId: "a",
      configuredModelId: null,
      explicitThinkingOptionId: "high",
    });

    expect(selection.selectedThinkingId).toBe("high");
    expect(selection.displayThinking).toBe("High");
  });

  it("falls back to the provider default model label instead of Auto", () => {
    const selection = resolveAgentModelSelection({
      models: [
        {
          id: "a",
          provider: "codex",
          label: "Model A",
          isDefault: true,
          thinkingOptions: [{ id: "low", label: "Low" }],
          defaultThinkingOptionId: "low",
        },
      ],
      runtimeModelId: null,
      configuredModelId: null,
      explicitThinkingOptionId: null,
    });

    expect(selection.displayModel).toBe("Model A");
    expect(selection.displayThinking).toBe("Low");
  });

  it("prefers the configured model when runtime model is not in the model list", () => {
    const selection = resolveAgentModelSelection({
      models: [
        {
          id: "default",
          provider: "claude",
          label: "Default (Sonnet 4.6)",
          isDefault: true,
          thinkingOptions: [
            { id: "low", label: "Low" },
            { id: "medium", label: "Medium" },
          ],
        },
      ],
      runtimeModelId: "claude-sonnet-4-6-20260101",
      configuredModelId: "default",
      explicitThinkingOptionId: null,
    });

    expect(selection.activeModelId).toBe("default");
    expect(selection.displayModel).toBe("Default (Sonnet 4.6)");
    expect(selection.selectedThinkingId).toBe("low");
    expect(selection.displayThinking).toBe("Low");
  });
});

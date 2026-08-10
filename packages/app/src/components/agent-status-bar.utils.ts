import type {
  AgentFeature,
  AgentModelDefinition,
  AgentProvider,
} from "@server/server/agent/agent-sdk-types";
import type { AgentProviderDefinition } from "@server/server/agent/provider-manifest";

export type ExplainedStatusSelector = "mode" | "model" | "context" | "thinking" | "cloud-group";
export type FeatureHighlightColor = "blue" | "default" | "yellow";

export type CloudGroupSelectorSummary = {
  provider: string;
  groupId: number;
  groupLabel: string;
  description?: string;
  isActiveForWorkspace?: boolean;
  isActiveForGlobalKey?: boolean;
};

export function getStatusSelectorHint(selector: ExplainedStatusSelector): string {
  switch (selector) {
    case "cloud-group":
      return "Cloud group";
    case "thinking":
      return "Thinking mode";
    case "model":
      return "Change model";
    case "context":
      return "Change context window";
    case "mode":
      return "Change permission mode";
  }
}

export function cloudGroupsForStatusProvider<T extends CloudGroupSelectorSummary>(
  cloudGroups: T[] | undefined,
  provider: string,
): T[] {
  return (cloudGroups ?? []).filter((group) => group.provider === provider);
}

export function resolveActiveCloudGroup<T extends CloudGroupSelectorSummary>(
  cloudGroups: T[] | undefined,
  provider: string,
): T | null {
  return (
    cloudGroupsForStatusProvider(cloudGroups, provider).find(
      (group) => group.isActiveForGlobalKey || group.isActiveForWorkspace,
    ) ?? null
  );
}

export function resolveCloudGroupDisplayLabel(
  cloudGroups: CloudGroupSelectorSummary[] | undefined,
  provider: string,
): string | null {
  return resolveActiveCloudGroup(cloudGroups, provider)?.groupLabel ?? null;
}

export function filterSelectableProviderDefinitions(
  providerDefinitions: AgentProviderDefinition[],
  selectableProviderIds: AgentProvider[] | undefined,
): AgentProviderDefinition[] {
  if (!selectableProviderIds) {
    return providerDefinitions;
  }
  const selectableIds = new Set(selectableProviderIds);
  return providerDefinitions.filter((definition) => selectableIds.has(definition.id));
}

export function scopeModelsToProvider(
  provider: AgentProvider | null,
  allProviderModels: Map<string, AgentModelDefinition[]>,
  fallbackModels: AgentModelDefinition[],
): Map<string, AgentModelDefinition[]> {
  const scopedModels = new Map<string, AgentModelDefinition[]>();
  if (provider) {
    scopedModels.set(provider, allProviderModels.get(provider) ?? fallbackModels);
  }
  return scopedModels;
}

export function normalizeModelId(modelId: string | null | undefined): string | null {
  const normalized = typeof modelId === "string" ? modelId.trim() : "";
  if (!normalized) {
    return null;
  }
  return normalized;
}

export function getFeatureTooltip(feature: Pick<AgentFeature, "label" | "tooltip">): string {
  return feature.tooltip ?? feature.label;
}

export function getFeatureHighlightColor(featureId: string): FeatureHighlightColor {
  switch (featureId) {
    case "fast_mode":
      return "yellow";
    case "plan_mode":
      return "blue";
    default:
      return "default";
  }
}

export function resolveAgentModelSelection(input: {
  models: AgentModelDefinition[] | null;
  runtimeModelId: string | null | undefined;
  configuredModelId: string | null | undefined;
  explicitThinkingOptionId: string | null | undefined;
}) {
  const { models, runtimeModelId, configuredModelId, explicitThinkingOptionId } = input;
  const normalizedRuntimeModelId = normalizeModelId(runtimeModelId);
  const normalizedConfiguredModelId = normalizeModelId(configuredModelId);
  const runtimeSelectedModel =
    models && normalizedRuntimeModelId
      ? (models.find((model) => model.id === normalizedRuntimeModelId) ?? null)
      : null;
  const preferredModelId =
    runtimeSelectedModel?.id ?? normalizedConfiguredModelId ?? normalizedRuntimeModelId;
  const fallbackModel = models?.find((model) => model.isDefault) ?? models?.[0] ?? null;
  const selectedModel =
    models && preferredModelId
      ? (models.find((model) => model.id === preferredModelId) ?? fallbackModel ?? null)
      : fallbackModel;

  const activeModelId = selectedModel?.id ?? preferredModelId ?? null;
  const displayModel =
    selectedModel?.label ?? preferredModelId ?? fallbackModel?.label ?? "Unknown model";

  const thinkingOptions = selectedModel?.thinkingOptions ?? null;
  const resolvedThinkingId =
    explicitThinkingOptionId && explicitThinkingOptionId !== "default"
      ? explicitThinkingOptionId
      : (selectedModel?.defaultThinkingOptionId ?? null);
  const selectedThinking =
    thinkingOptions?.find((option) => option.id === resolvedThinkingId) ?? null;
  const effectiveThinking = selectedThinking ?? thinkingOptions?.[0] ?? null;
  const selectedThinkingId = effectiveThinking?.id ?? null;
  const displayThinking = effectiveThinking?.label ?? selectedThinkingId ?? "Unknown";

  return {
    selectedModel,
    activeModelId,
    displayModel,
    thinkingOptions,
    selectedThinkingId,
    displayThinking,
  };
}

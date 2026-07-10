import type { AgentModelDefinition } from "@server/server/agent/agent-sdk-types";
import type { AgentProviderDefinition } from "@server/server/agent/provider-manifest";
import { buildFavoriteModelKey, type FavoriteModelRow } from "@/hooks/use-form-preferences";

export type SelectorModelRow = FavoriteModelRow;

export interface SelectorCloudModel {
  id: string;
  label: string;
  description?: string;
}

export interface SelectorCloudGroup {
  provider: string;
  groupId: number;
  groupLabel: string;
  platform: string;
  description?: string;
  isActiveForWorkspace?: boolean;
  isActiveForGlobalKey?: boolean;
  models: SelectorCloudModel[];
}

export function resolveProviderLabel(
  providerDefinitions: AgentProviderDefinition[],
  providerId: string,
): string {
  return (
    providerDefinitions.find((definition) => definition.id === providerId)?.label ?? providerId
  );
}

export function buildSelectedTriggerLabel(modelLabel: string): string {
  return modelLabel;
}

export function buildModelRows(
  providerDefinitions: AgentProviderDefinition[],
  allProviderModels: Map<string, AgentModelDefinition[]>,
): SelectorModelRow[] {
  const providerLabelMap = new Map(
    providerDefinitions.map((definition) => [definition.id, definition.label]),
  );
  const rows: SelectorModelRow[] = [];

  for (const definition of providerDefinitions) {
    const providerLabel = providerLabelMap.get(definition.id) ?? definition.label;
    for (const model of allProviderModels.get(definition.id) ?? []) {
      rows.push({
        favoriteKey: buildFavoriteModelKey({
          provider: definition.id,
          modelId: model.id,
        }),
        provider: definition.id,
        providerLabel,
        modelId: model.id,
        modelLabel: model.label,
        description: model.description,
      });
    }
  }

  return rows;
}

export function cloudGroupsForProvider(
  cloudGroups: SelectorCloudGroup[] | undefined,
  providerId: string,
): SelectorCloudGroup[] {
  return (cloudGroups ?? []).filter((group) => group.provider === providerId);
}

export function buildCloudGroupModelRows(input: {
  providerLabel: string;
  group: SelectorCloudGroup;
}): SelectorModelRow[] {
  return input.group.models.map((model) => ({
    favoriteKey: buildFavoriteModelKey({
      provider: input.group.provider,
      modelId: model.id,
    }),
    provider: input.group.provider,
    providerLabel: input.providerLabel,
    modelId: model.id,
    modelLabel: model.label,
    description: model.description,
  }));
}

export function buildOtherAvailableModelRows(
  providerRows: SelectorModelRow[],
  cloudGroups: SelectorCloudGroup[] | undefined,
): SelectorModelRow[] {
  const cloudModelKeys = new Set(
    (cloudGroups ?? []).flatMap((group) =>
      group.models.map((model) => `${group.provider}\0${model.id}`),
    ),
  );
  return providerRows.filter((row) => !cloudModelKeys.has(`${row.provider}\0${row.modelId}`));
}

export function matchesSearch(row: SelectorModelRow, normalizedQuery: string): boolean {
  if (!normalizedQuery) {
    return true;
  }

  return [row.modelLabel, row.modelId, row.providerLabel].some((value) =>
    value.toLowerCase().includes(normalizedQuery),
  );
}

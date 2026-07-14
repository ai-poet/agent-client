import type { AgentProviderDefinition } from "@server/server/agent/provider-manifest";
import type { AgentModelDefinition } from "@server/server/agent/agent-sdk-types";
import type {
  Sub2APIGroup,
  Sub2APIGroupStatusItem,
  Sub2APIKey,
  Sub2APIModelCatalog,
} from "@/lib/sub2api-client";
import type {
  DesktopProviderPayload,
  ProviderStore,
} from "@/screens/settings/sub2api-provider-types";
import type { SelectorCloudGroup } from "@/components/combined-model-selector.utils";
import { buildGroupFirstModelCatalog } from "@/screens/settings/paseo-cloud-catalog-utils";
import {
  resolveManagedCloudRouteForKey,
  resolveManagedCloudRouteFromPlatform,
  type ManagedCloudDesktopScope,
} from "@/screens/settings/managed-cloud-scope";

export type ActiveGlobalCloudProvider = {
  provider: ManagedCloudDesktopScope;
  apiKey: string;
  endpoint: string;
  providerName?: string;
};

function normalizeApiKey(value: string | null | undefined): string {
  return value?.trim() ?? "";
}

function normalizeProviderEndpoint(value: string | null | undefined): string {
  const trimmed = value?.trim().replace(/\/+$/, "") ?? "";
  return trimmed.toLowerCase().endsWith("/v1") ? trimmed.slice(0, -3) : trimmed;
}

function providerSupportsScope(
  provider: DesktopProviderPayload | undefined,
  scope: ManagedCloudDesktopScope,
): provider is DesktopProviderPayload {
  return Boolean(provider && (!provider.target || provider.target === scope));
}

function resolveScopedActiveProviderIds(
  store: ProviderStore,
): Record<ManagedCloudDesktopScope, string | null> {
  const hasScopedIds =
    store.activeClaudeProviderId !== null || store.activeCodexProviderId !== null;
  const legacyFallback = hasScopedIds ? null : (store.activeProviderId ?? null);
  return {
    claude: store.activeClaudeProviderId ?? legacyFallback,
    codex: store.activeCodexProviderId ?? legacyFallback,
  };
}

export function resolveActiveGlobalCloudProviders(
  store: ProviderStore | null | undefined,
): ActiveGlobalCloudProvider[] {
  if (!store) {
    return [];
  }

  const activeIds = resolveScopedActiveProviderIds(store);
  const providersById = new Map(store.providers.map((provider) => [provider.id, provider]));
  const result: ActiveGlobalCloudProvider[] = [];

  for (const scope of ["claude", "codex"] as const) {
    const id = activeIds[scope];
    const provider = id ? providersById.get(id) : undefined;
    if (!providerSupportsScope(provider, scope)) {
      continue;
    }

    const apiKey = normalizeApiKey(provider.apiKey);
    const endpoint = normalizeProviderEndpoint(provider.endpoint);
    if (!apiKey || !endpoint) {
      continue;
    }

    result.push({
      provider: scope,
      apiKey,
      endpoint,
      providerName: provider.name,
    });
  }

  return result;
}

export function buildCloudModelRoutingGroups(input: {
  catalog: Sub2APIModelCatalog | null | undefined;
  statuses?: Sub2APIGroupStatusItem[] | null;
  providerDefinitions: AgentProviderDefinition[];
  activeGroupIdsByProvider?: Partial<Record<ManagedCloudDesktopScope, number | null | undefined>>;
}): SelectorCloudGroup[] {
  const supportedProviderIds = new Set(
    input.providerDefinitions.map((definition) => definition.id),
  );
  const groups: SelectorCloudGroup[] = [];

  for (const platform of ["anthropic", "openai"]) {
    const route = resolveManagedCloudRouteFromPlatform(platform);
    if (!route.ok || !supportedProviderIds.has(route.scope)) {
      continue;
    }
    const catalog = buildGroupFirstModelCatalog({
      catalog: input.catalog,
      statuses: input.statuses,
      platform,
    });

    for (const group of catalog.groups) {
      const seenModelIds = new Set<string>();
      const models = group.models
        .filter((entry) => {
          if (seenModelIds.has(entry.item.model)) {
            return false;
          }
          seenModelIds.add(entry.item.model);
          return true;
        })
        .map((entry) => ({
          id: entry.item.model,
          label: entry.item.display_name || entry.item.model,
          description: `${group.group.rate_multiplier}x · ${entry.item.billing_mode}`,
        }));
      if (models.length === 0) {
        continue;
      }

      const status = group.status?.stable_status || group.status?.latest_status;
      const isActiveForGlobalKey = input.activeGroupIdsByProvider?.[route.scope] === group.group.id;
      groups.push({
        provider: route.scope,
        groupId: group.group.id,
        groupLabel: group.group.name,
        platform,
        isActiveForGlobalKey,
        description: `${isActiveForGlobalKey ? "Current global key · " : ""}${route.cliLabel} · ${
          group.group.rate_multiplier
        }x${status ? ` · ${status}` : ""}`,
        models,
      });
    }
  }

  return groups.sort((a, b) => {
    const aActive = a.isActiveForGlobalKey ? 1 : 0;
    const bActive = b.isActiveForGlobalKey ? 1 : 0;
    return bActive - aActive;
  });
}

export function buildGlobalCloudRouteGroups(input: {
  activeProviders: ActiveGlobalCloudProvider[];
  cloudEndpoint?: string | null;
  keys?: Sub2APIKey[] | null;
  groups?: Sub2APIGroup[] | null;
  providerDefinitions: AgentProviderDefinition[];
  allProviderModels?: Map<string, AgentModelDefinition[]>;
}): SelectorCloudGroup[] {
  const keys = input.keys ?? [];
  const groups = input.groups ?? [];
  const groupsById = new Map(groups.map((group) => [group.id, group]));
  const supportedProviderIds = new Set(
    input.providerDefinitions.map((definition) => definition.id),
  );
  const cloudEndpoint = normalizeProviderEndpoint(input.cloudEndpoint);
  const result: SelectorCloudGroup[] = [];

  for (const activeProvider of input.activeProviders) {
    if (!supportedProviderIds.has(activeProvider.provider)) {
      continue;
    }
    if (cloudEndpoint && activeProvider.endpoint && activeProvider.endpoint !== cloudEndpoint) {
      continue;
    }

    const activeKey = normalizeApiKey(activeProvider.apiKey);
    const key = keys.find((candidate) => normalizeApiKey(candidate.key) === activeKey);
    if (!key) {
      continue;
    }

    const route = resolveManagedCloudRouteForKey(key, groups);
    if (!route.ok || route.scope !== activeProvider.provider) {
      continue;
    }

    const groupId = key.group_id ?? key.group?.id ?? null;
    if (typeof groupId !== "number") {
      continue;
    }

    const group = key.group ?? groupsById.get(groupId) ?? null;
    const groupLabel = group?.name ?? `Group #${groupId}`;
    const models = (input.allProviderModels?.get(activeProvider.provider) ?? []).map((model) => ({
      id: model.id,
      label: model.label,
      description: model.description,
    }));
    if (models.length === 0) {
      continue;
    }
    result.push({
      provider: activeProvider.provider,
      groupId,
      groupLabel,
      platform: group?.platform ?? "",
      description: `${route.cliLabel} · global CLI key`,
      isActiveForGlobalKey: true,
      models,
    });
  }

  return result;
}

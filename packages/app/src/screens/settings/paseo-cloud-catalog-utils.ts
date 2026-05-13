import type {
  Sub2APIGroupStatusItem,
  Sub2APIModelCatalog,
  Sub2APIModelCatalogComparison,
  Sub2APIModelCatalogGroupRef,
  Sub2APIModelCatalogItem,
  Sub2APIModelCatalogPrice,
} from "@/lib/sub2api-client";

export interface GroupFirstCatalogModel {
  item: Sub2APIModelCatalogItem;
  group: Sub2APIModelCatalogGroupRef;
  effectivePricing: Sub2APIModelCatalogPrice;
  comparison: Sub2APIModelCatalogComparison;
  isBestGroup: boolean;
}

export interface GroupFirstCatalogGroup {
  platform: string;
  group: Sub2APIModelCatalogGroupRef;
  status: Sub2APIGroupStatusItem | null;
  models: GroupFirstCatalogModel[];
}

export interface GroupFirstCatalogResult {
  groups: GroupFirstCatalogGroup[];
  allModels: Sub2APIModelCatalogItem[];
}

export function buildCatalogModelCardItem(model: GroupFirstCatalogModel): Sub2APIModelCatalogItem {
  return {
    ...model.item,
    best_group: model.group,
    effective_pricing_usd: model.effectivePricing,
    comparison: model.comparison,
    other_groups: (model.item.other_groups ?? []).filter(
      (entry) => entry.group.id !== model.group.id,
    ),
  };
}

function normalizePlatform(value: string | null | undefined): string {
  return value?.trim().toLowerCase() ?? "";
}

function getRuntimeStatusRank(status: Sub2APIGroupStatusItem | null): number {
  switch (status?.stable_status || status?.latest_status) {
    case "up":
      return 3;
    case "degraded":
      return 2;
    case "down":
      return 1;
    default:
      return 0;
  }
}

function getGroupSortScore(group: GroupFirstCatalogGroup): number {
  const statusScore = getRuntimeStatusRank(group.status) * 100;
  const availabilityScore =
    typeof group.status?.availability_24h === "number" ? group.status.availability_24h : 0;
  const priceScore = Math.max(0, 10 - group.group.rate_multiplier);
  return statusScore + availabilityScore + priceScore;
}

export function buildGroupFirstModelCatalog(input: {
  catalog: Sub2APIModelCatalog | null | undefined;
  statuses?: Sub2APIGroupStatusItem[] | null;
  platform?: string | null;
}): GroupFirstCatalogResult {
  const items = input.catalog?.items ?? [];
  const platformFilter = normalizePlatform(input.platform);
  const filteredItems = platformFilter
    ? items.filter((item) => normalizePlatform(item.platform) === platformFilter)
    : items;
  const statusByGroupId = new Map(
    (input.statuses ?? []).map((status) => [status.group_id, status] as const),
  );
  const groups = new Map<number, GroupFirstCatalogGroup>();

  for (const item of filteredItems) {
    const refs: GroupFirstCatalogModel[] = [
      {
        item,
        group: item.best_group,
        effectivePricing: item.effective_pricing_usd,
        comparison: item.comparison,
        isBestGroup: true,
      },
      ...(item.other_groups ?? []).map((entry) => ({
        item,
        group: entry.group,
        effectivePricing: entry.effective_pricing_usd,
        comparison: entry.comparison,
        isBestGroup: false,
      })),
    ];

    for (const model of refs) {
      const existing = groups.get(model.group.id);
      if (existing) {
        existing.models.push(model);
        continue;
      }
      groups.set(model.group.id, {
        platform: normalizePlatform(item.platform),
        group: model.group,
        status: statusByGroupId.get(model.group.id) ?? null,
        models: [model],
      });
    }
  }

  return {
    groups: Array.from(groups.values()).sort((a, b) => {
      const scoreDelta = getGroupSortScore(b) - getGroupSortScore(a);
      if (scoreDelta !== 0) {
        return scoreDelta;
      }
      const rateDelta = a.group.rate_multiplier - b.group.rate_multiplier;
      if (rateDelta !== 0) {
        return rateDelta;
      }
      return a.group.name.localeCompare(b.group.name);
    }),
    allModels: filteredItems,
  };
}

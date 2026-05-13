import { useEffect, useMemo, useState } from "react";
import { Text, TextInput, View } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { ComboSelect } from "@/components/agent-form/agent-form-dropdowns";
import { ModelCard } from "@/components/model-square/group-card";
import { SegmentedControl } from "@/components/ui/segmented-control";
import { SettingsSection } from "@/screens/settings/settings-section";
import { settingsStyles } from "@/styles/settings";
import { useSub2APIAuth } from "@/hooks/use-sub2api-auth";
import {
  useSub2APIGroupStatuses,
  useSub2APIModelCatalog,
  useSub2APIPaymentConfig,
} from "@/hooks/use-sub2api-api";
import { CLOUD_NAME } from "@/config/branding";
import {
  buildCatalogModelCardItem,
  buildGroupFirstModelCatalog,
  type GroupFirstCatalogGroup,
  type GroupFirstCatalogModel,
} from "@/screens/settings/paseo-cloud-catalog-utils";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";

type PlatformFilter = "anthropic" | "openai";
type SelectedGroup = number | "all" | null;

const PLATFORM_OPTIONS = [
  {
    value: "anthropic" as const,
    label: "Claude",
    testID: "paseo-cloud-catalog-tab-claude",
  },
  {
    value: "openai" as const,
    label: "Codex",
    testID: "paseo-cloud-catalog-tab-codex",
  },
];

function getErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function formatAvailability(value: number | null | undefined): string {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return "-";
  }
  return `${value.toFixed(value >= 99 ? 2 : 1)}%`;
}

function normalizeSearch(value: string): string {
  return value.trim().toLowerCase();
}

function matchesModelSearch(
  input: { model: string; display_name: string; platform: string },
  query: string,
): boolean {
  if (!query) {
    return true;
  }
  return [input.model, input.display_name, input.platform].some((value) =>
    value.toLowerCase().includes(query),
  );
}

function isGroupCatalogModel(
  value: GroupFirstCatalogModel | unknown,
): value is GroupFirstCatalogModel {
  return (
    typeof value === "object" &&
    value !== null &&
    "item" in value &&
    "group" in value &&
    "effectivePricing" in value
  );
}

function GroupStatusLine({
  group,
  text,
}: {
  group: GroupFirstCatalogGroup;
  text: ReturnType<typeof getSub2APIMessages>["modelCatalog"];
}) {
  const status = group.status;
  if (!status) {
    return <Text style={styles.usageHint}>{text.runtimeUnknown}</Text>;
  }
  return (
    <View style={styles.statusMetricsRow}>
      <Text style={styles.usageHint}>
        {text.status} {(status.stable_status || status.latest_status || "unknown").toUpperCase()}
      </Text>
      <Text style={styles.usageHint}>24h {formatAvailability(status.availability_24h)}</Text>
      <Text style={styles.usageHint}>7d {formatAvailability(status.availability_7d)}</Text>
    </View>
  );
}

export function Sub2APIModelsSection() {
  const { theme } = useUnistyles();
  const { isLoggedIn } = useSub2APIAuth();
  const statusesQuery = useSub2APIGroupStatuses();
  const catalogQuery = useSub2APIModelCatalog();
  const paymentConfigQuery = useSub2APIPaymentConfig();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).modelCatalog, [locale]);
  const [platform, setPlatform] = useState<PlatformFilter>("anthropic");
  const [selectedGroup, setSelectedGroup] = useState<SelectedGroup>(null);
  const [search, setSearch] = useState("");

  const catalogItems = catalogQuery.data?.items ?? [];
  const summary = catalogQuery.data?.summary;

  const groupCatalog = useMemo(
    () =>
      buildGroupFirstModelCatalog({
        catalog: catalogQuery.data,
        statuses: statusesQuery.data,
        platform,
      }),
    [catalogQuery.data, platform, statusesQuery.data],
  );
  const statusByGroupId = useMemo(
    () => new Map((statusesQuery.data ?? []).map((status) => [status.group_id, status] as const)),
    [statusesQuery.data],
  );
  const actualPaidPricing = useMemo(
    () => ({
      balanceCreditCnyPerUsd: paymentConfigQuery.data?.balanceCreditCnyPerUsd ?? null,
      usdExchangeRate: paymentConfigQuery.data?.usdExchangeRate ?? null,
      locale,
    }),
    [
      locale,
      paymentConfigQuery.data?.balanceCreditCnyPerUsd,
      paymentConfigQuery.data?.usdExchangeRate,
    ],
  );

  useEffect(() => {
    if (selectedGroup === "all") {
      return;
    }
    if (
      typeof selectedGroup === "number" &&
      groupCatalog.groups.some((group) => group.group.id === selectedGroup)
    ) {
      return;
    }
    setSelectedGroup(groupCatalog.groups[0]?.group.id ?? "all");
  }, [groupCatalog.groups, selectedGroup]);

  const selectedGroupRecord =
    typeof selectedGroup === "number"
      ? (groupCatalog.groups.find((group) => group.group.id === selectedGroup) ?? null)
      : null;
  const groupOptions = useMemo(
    () => [
      ...groupCatalog.groups.map((group) => {
        const statusLabel = group.status?.stable_status || group.status?.latest_status;
        return {
          id: String(group.group.id),
          label: group.group.name,
          description: text.groupDescription(
            group.models.length,
            group.group.rate_multiplier,
            statusLabel,
          ),
        };
      }),
      {
        id: "all",
        label: text.allModels,
        description: text.platformModels(groupCatalog.allModels.length),
      },
    ],
    [groupCatalog.allModels.length, groupCatalog.groups, text],
  );
  const selectedGroupValue = selectedGroup === null ? "" : String(selectedGroup);
  const normalizedSearch = useMemo(() => normalizeSearch(search), [search]);
  const selectedModels = useMemo(() => {
    if (selectedGroup === "all") {
      return groupCatalog.allModels.filter((item) => matchesModelSearch(item, normalizedSearch));
    }
    return (selectedGroupRecord?.models ?? []).filter((entry) =>
      matchesModelSearch(entry.item, normalizedSearch),
    );
  }, [groupCatalog.allModels, normalizedSearch, selectedGroup, selectedGroupRecord?.models]);
  const isLoading = catalogQuery.isLoading || statusesQuery.isLoading;
  const error = catalogQuery.error || statusesQuery.error;

  return (
    <SettingsSection title={text.title}>
      {!isLoggedIn ? (
        <View style={[settingsStyles.card, styles.cardBody]}>
          <Text style={styles.hintText}>{text.signInHint(CLOUD_NAME)}</Text>
        </View>
      ) : null}

      {isLoggedIn ? (
        <View style={[settingsStyles.card, styles.cardBody]}>
          {isLoading ? (
            <Text style={styles.hintText}>{text.loading}</Text>
          ) : error ? (
            <View style={styles.errorBlock}>
              <Text style={styles.errorText}>{getErrorMessage(error)}</Text>
            </View>
          ) : catalogItems.length === 0 ? (
            <Text style={styles.hintText}>{text.noModels}</Text>
          ) : (
            <>
              {summary ? (
                <View style={styles.summaryGrid}>
                  <Text style={styles.summaryCell}>
                    {text.models}: {summary.total_models}
                  </Text>
                  <Text style={styles.summaryCell}>
                    {text.token}: {summary.token_models}
                  </Text>
                  <Text style={styles.summaryCell}>
                    {text.nonToken}: {summary.non_token_models}
                  </Text>
                  <Text style={styles.summaryCell}>
                    {text.bestSavings}: {summary.max_savings_percent.toFixed(1)}%
                  </Text>
                </View>
              ) : null}

              <SegmentedControl
                options={PLATFORM_OPTIONS}
                value={platform}
                onValueChange={(value) => setPlatform(value)}
                size="sm"
                testID="paseo-cloud-catalog-platform-tabs"
              />

              <View style={styles.catalogControlsRow}>
                <View style={styles.groupSelectWrap}>
                  <Text style={styles.fieldLabel}>{text.group}</Text>
                  <ComboSelect
                    label={text.group}
                    title={text.selectGroup(platform)}
                    value={selectedGroupValue}
                    options={groupOptions}
                    placeholder={text.selectGroupPlaceholder}
                    isLoading={statusesQuery.isFetching}
                    onSelect={(id) => setSelectedGroup(id === "all" ? "all" : Number(id))}
                    showLabel={false}
                    testID="paseo-cloud-catalog-group-select"
                  />
                </View>
                <View style={styles.searchWrap}>
                  <Text style={styles.fieldLabel}>{text.search}</Text>
                  <TextInput
                    value={search}
                    onChangeText={setSearch}
                    placeholder={
                      selectedGroup === "all" ? text.searchAllModels : text.searchModelsInGroup
                    }
                    placeholderTextColor={theme.colors.foregroundMuted}
                    autoCapitalize="none"
                    autoCorrect={false}
                    style={styles.textInput}
                  />
                </View>
              </View>

              <View style={styles.modelList}>
                {selectedGroupRecord ? (
                  <View style={styles.selectedGroupHeader}>
                    <Text style={styles.selectedGroupTitle}>{selectedGroupRecord.group.name}</Text>
                    <Text style={styles.usageHint}>
                      {platform} · {selectedGroupRecord.models.length} {text.models} ·{" "}
                      {selectedGroupRecord.group.rate_multiplier}x
                    </Text>
                    <GroupStatusLine group={selectedGroupRecord} text={text} />
                  </View>
                ) : (
                  <View style={styles.selectedGroupHeader}>
                    <Text style={styles.selectedGroupTitle}>{text.allModels}</Text>
                    <Text style={styles.usageHint}>{text.fallbackAcrossGroups(platform)}</Text>
                  </View>
                )}

                {selectedModels.length === 0 ? (
                  <Text style={styles.hintText}>{text.noSearchMatch}</Text>
                ) : (
                  selectedModels.map((entry) => {
                    const cardItem = isGroupCatalogModel(entry)
                      ? buildCatalogModelCardItem(entry)
                      : entry;
                    const statusGroupId = isGroupCatalogModel(entry)
                      ? entry.group.id
                      : cardItem.best_group.id;
                    return (
                      <ModelCard
                        key={`${cardItem.model}-${statusGroupId}`}
                        item={cardItem}
                        status={statusByGroupId.get(statusGroupId) ?? null}
                        actualPaidPricing={actualPaidPricing}
                        locale={locale}
                      />
                    );
                  })
                )}
              </View>
            </>
          )}
        </View>
      ) : null}
    </SettingsSection>
  );
}

const styles = StyleSheet.create((theme) => ({
  cardBody: {
    padding: theme.spacing[4],
    gap: theme.spacing[3],
  },
  hintText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  usageHint: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  errorBlock: {
    padding: theme.spacing[3],
    borderWidth: 1,
    borderColor: theme.colors.destructive,
    borderRadius: theme.borderRadius.md,
  },
  errorText: {
    color: theme.colors.destructive,
    fontSize: theme.fontSize.xs,
  },
  summaryGrid: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: theme.spacing[2],
  },
  summaryCell: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  textInput: {
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.md,
    paddingHorizontal: theme.spacing[3],
    paddingVertical: theme.spacing[2],
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
  },
  catalogControlsRow: {
    flexDirection: "row",
    gap: theme.spacing[3],
    alignItems: "stretch",
    flexWrap: "wrap",
  },
  groupSelectWrap: {
    flex: 1,
    minWidth: 240,
    gap: theme.spacing[1],
  },
  searchWrap: {
    flex: 2,
    minWidth: 260,
    gap: theme.spacing[1],
  },
  fieldLabel: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.medium,
  },
  modelList: {
    gap: theme.spacing[2],
  },
  selectedGroupHeader: {
    gap: theme.spacing[1],
    paddingBottom: theme.spacing[2],
  },
  selectedGroupTitle: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.base,
    fontWeight: theme.fontWeight.medium,
  },
  statusMetricsRow: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: theme.spacing[2],
  },
}));

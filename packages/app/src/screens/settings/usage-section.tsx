import { useCallback, useMemo, useState } from "react";
import { Pressable, ScrollView, Text, View } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import {
  BarChart3,
  Clock3,
  Coins,
  Database,
  MessageSquareText,
  RotateCw,
} from "lucide-react-native";

import { SettingsSection } from "@/screens/settings/settings-section";
import { SegmentedControl } from "@/components/ui/segmented-control";
import { StatCard } from "@/components/ui/stat-card";
import { StatusPanel } from "@/components/ui/status-panel";
import { LoadingSpinner } from "@/components/ui/loading-spinner";
import { settingsStyles } from "@/styles/settings";
import { getProviderIcon } from "@/components/provider-icons";
import { useLocalDaemonServerId } from "@/hooks/use-is-local-daemon";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";
import { useUsageStats, type UsageBucket } from "@/hooks/use-usage-stats";
import { formatCost, formatDuration, formatTokenValue } from "@/utils/usage-format";
import {
  buildTrendPoints,
  findLatestPopulatedIndex,
  resolveBarHeightPercent,
  shouldShowTrendLabel,
  type TrendPoint,
  type UsageRangeKey,
} from "@/utils/usage-trend";

type TrendMetric = "tokens" | "cost" | "duration";

const RANGE_OPTIONS: UsageRangeKey[] = ["1", "7", "30", "90", "all"];
/** A 4% floor keeps a tiny breakdown row visible and tappable. */
const MIN_TRACK_PERCENT = 4;

export function UsageSection() {
  const { theme } = useUnistyles();
  // Usage is recorded by the daemon that runs the agents, which is the local one.
  const serverId = useLocalDaemonServerId() ?? "";
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).settings.usage, [locale]);

  const [range, setRange] = useState<UsageRangeKey>("7");
  const [metric, setMetric] = useState<TrendMetric>("tokens");
  const [selectedIndex, setSelectedIndex] = useState<number | null>(null);

  const daily = useUsageStats({ serverId, range, groupBy: "day" });
  const byProvider = useUsageStats({ serverId, range, groupBy: "provider" });
  const byProject = useUsageStats({ serverId, range, groupBy: "project" });

  const isLoading = daily.isLoading || byProvider.isLoading || byProject.isLoading;
  const isFetching = daily.isFetching || byProvider.isFetching || byProject.isFetching;

  const handleRefresh = useCallback(() => {
    void daily.refetch();
    void byProvider.refetch();
    void byProject.refetch();
  }, [daily, byProvider, byProject]);

  const { points, unit } = useMemo(
    () =>
      buildTrendPoints({
        buckets: daily.stats?.buckets ?? [],
        range,
        now: new Date(),
      }),
    [daily.stats, range],
  );

  const metricValue = useCallback(
    (point: TrendPoint) =>
      metric === "tokens"
        ? point.totalTokens
        : metric === "cost"
          ? point.costUsd
          : point.durationMs,
    [metric],
  );

  const maxValue = useMemo(
    () => points.reduce((max, point) => Math.max(max, metricValue(point)), 0),
    [points, metricValue],
  );

  // Touch has no hover, so default to the newest bucket that actually has data.
  const activeIndex = selectedIndex ?? findLatestPopulatedIndex(points);
  const activePoint = activeIndex >= 0 ? points[activeIndex] : undefined;

  const totals = daily.stats?.totals;
  const rangeLabel = text.ranges[range];

  const trailing = (
    <Pressable
      onPress={handleRefresh}
      disabled={isFetching}
      hitSlop={8}
      style={settingsStyles.sectionHeaderLink}
      accessibilityRole="button"
      accessibilityLabel={text.refresh}
    >
      {isFetching ? (
        <LoadingSpinner size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
      ) : (
        <RotateCw size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
      )}
    </Pressable>
  );

  if (!daily.supportsUsageStats) {
    return (
      <SettingsSection title={text.title}>
        <View style={settingsStyles.card}>
          <StatusPanel title={text.unsupported} />
        </View>
      </SettingsSection>
    );
  }

  const hasAnyUsage = (totals?.turns ?? 0) > 0;

  return (
    <SettingsSection title={text.title} trailing={trailing}>
      <Text style={styles.subtitle}>{text.subtitle}</Text>

      <SegmentedControl<UsageRangeKey>
        value={range}
        onValueChange={(next) => {
          setRange(next);
          setSelectedIndex(null);
        }}
        options={RANGE_OPTIONS.map((option) => ({
          value: option,
          label: text.ranges[option],
        }))}
      />

      <Text style={styles.scope}>{text.scope(rangeLabel, text.allProjects)}</Text>

      {daily.error ? (
        <View style={settingsStyles.card}>
          <StatusPanel title={text.loadFailed} description={daily.error} error />
        </View>
      ) : isLoading ? (
        <View style={settingsStyles.card}>
          <StatusPanel title={text.title} loading />
        </View>
      ) : !hasAnyUsage ? (
        <View style={settingsStyles.card}>
          <StatusPanel
            title={range === "all" ? text.empty : text.noMatching}
            description={range === "all" ? text.emptyHint : undefined}
          />
        </View>
      ) : (
        <>
          <View style={styles.cardsGrid}>
            <StatCard
              icon={
                <MessageSquareText size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
              }
              label={text.cards.turns}
              value={String(totals?.turns ?? 0)}
              hint={text.cards.turnsHint(totals?.turns ?? 0)}
            />
            <StatCard
              icon={<BarChart3 size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />}
              label={text.cards.tokens}
              value={formatTokenValue(totals?.totalTokens ?? 0)}
              hint={text.cards.tokensHint(
                formatTokenValue(totals?.inputTokens ?? 0),
                formatTokenValue(totals?.outputTokens ?? 0),
              )}
            />
            <StatCard
              icon={<Database size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />}
              label={text.cards.cached}
              value={formatTokenValue(totals?.cachedInputTokens ?? 0)}
              hint={text.cards.cachedHint}
            />
            <StatCard
              icon={<Clock3 size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />}
              label={text.cards.duration}
              value={formatDuration(totals?.durationMs ?? 0)}
              hint={text.cards.durationHint}
            />
            <StatCard
              icon={<Coins size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />}
              label={text.cards.cost}
              value={formatCost(totals?.costUsd ?? 0)}
              hint={text.cards.costHint}
            />
          </View>

          <View style={settingsStyles.card}>
            <View style={styles.trendHeader}>
              <View style={styles.trendTitleBlock}>
                <Text style={settingsStyles.rowTitle}>{text.trend}</Text>
                <Text style={settingsStyles.rowHint}>{text.trendHint}</Text>
              </View>
            </View>

            <View style={styles.metricRow}>
              <SegmentedControl<TrendMetric>
                value={metric}
                onValueChange={setMetric}
                size="sm"
                options={[
                  { value: "tokens" as const, label: text.metrics.tokens },
                  { value: "cost" as const, label: text.metrics.cost },
                  { value: "duration" as const, label: text.metrics.duration },
                ]}
              />
            </View>

            {activePoint ? (
              <View style={styles.activeSummary}>
                <Text style={styles.activeLabel}>{formatBucketLabel(activePoint, unit)}</Text>
                <Text style={styles.activeValue}>
                  {metric === "tokens"
                    ? formatTokenValue(activePoint.totalTokens)
                    : metric === "cost"
                      ? formatCost(activePoint.costUsd)
                      : formatDuration(activePoint.durationMs)}
                </Text>
                <Text style={styles.activeMeta}>{text.bucketSummary(activePoint.turns)}</Text>
              </View>
            ) : null}

            <ScrollView horizontal showsHorizontalScrollIndicator={false}>
              <View style={styles.chart}>
                {points.map((point, index) => {
                  const height = resolveBarHeightPercent(metricValue(point), maxValue);
                  const isActive = index === activeIndex;
                  return (
                    <Pressable
                      key={point.start}
                      onPress={() => setSelectedIndex(index)}
                      style={styles.barColumn}
                      accessibilityRole="button"
                      accessibilityLabel={formatBucketLabel(point, unit)}
                    >
                      <View style={styles.barTrack}>
                        <View
                          style={[
                            styles.bar,
                            { height: `${height}%` },
                            isActive && styles.barActive,
                          ]}
                        />
                      </View>
                      <Text style={styles.barLabel} numberOfLines={1}>
                        {shouldShowTrendLabel({ index, total: points.length, unit })
                          ? formatAxisLabel(point, unit)
                          : ""}
                      </Text>
                    </Pressable>
                  );
                })}
              </View>
            </ScrollView>
          </View>

          <BreakdownCard
            title={text.byProvider}
            emptyLabel={text.noProviderRecords}
            buckets={byProvider.stats?.buckets ?? []}
            renderIcon={(key) => {
              const Icon = getProviderIcon(key);
              return <Icon size={theme.iconSize.sm} color={theme.colors.foreground} />;
            }}
          />
          <BreakdownCard
            title={text.byProject}
            emptyLabel={text.noProjectRecords}
            buckets={byProject.stats?.buckets ?? []}
            formatKey={(key) => key.split(/[\\/]/).filter(Boolean).pop() ?? key}
          />
        </>
      )}
    </SettingsSection>
  );
}

interface BreakdownCardProps {
  title: string;
  emptyLabel: string;
  buckets: UsageBucket[];
  renderIcon?: (key: string) => React.ReactNode;
  formatKey?: (key: string) => string;
}

function BreakdownCard({ title, emptyLabel, buckets, renderIcon, formatKey }: BreakdownCardProps) {
  const max = buckets.reduce((value, bucket) => Math.max(value, bucket.totalTokens), 0);

  return (
    <View style={settingsStyles.card}>
      <View style={styles.breakdownHeader}>
        <Text style={settingsStyles.rowTitle}>{title}</Text>
      </View>
      {buckets.length === 0 ? (
        <StatusPanel title={emptyLabel} />
      ) : (
        buckets.map((bucket) => {
          const percent =
            max <= 0 || bucket.totalTokens <= 0
              ? 0
              : Math.max(MIN_TRACK_PERCENT, Math.round((bucket.totalTokens / max) * 100));
          return (
            <View key={bucket.key} style={styles.breakdownRow}>
              <View style={styles.breakdownTitleRow}>
                {renderIcon?.(bucket.key) ?? null}
                <Text style={styles.breakdownTitle} numberOfLines={1}>
                  {formatKey?.(bucket.key) ?? bucket.key}
                </Text>
                <Text style={styles.breakdownTokens}>{formatTokenValue(bucket.totalTokens)}</Text>
              </View>
              <View style={styles.track}>
                <View style={[styles.trackFill, { width: `${percent}%` }]} />
              </View>
              <Text style={styles.breakdownMeta}>
                {`${bucket.turns} · ${formatDuration(bucket.durationMs)} · ${formatCost(bucket.costUsd)}`}
              </Text>
            </View>
          );
        })
      )}
    </View>
  );
}

function formatAxisLabel(point: TrendPoint, unit: "day" | "week" | "month"): string {
  const [year, month, day] = point.start.split("-");
  return unit === "month" ? `${year}/${Number(month)}` : `${Number(month)}/${Number(day)}`;
}

function formatBucketLabel(point: TrendPoint, unit: "day" | "week" | "month"): string {
  if (unit === "week" && point.start !== point.end) {
    return `${formatAxisLabel(point, "day")} – ${formatAxisLabel({ ...point, start: point.end }, "day")}`;
  }
  return formatAxisLabel(point, unit);
}

const styles = StyleSheet.create((theme) => ({
  subtitle: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
    marginBottom: theme.spacing[2],
  },
  scope: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
    marginTop: theme.spacing[2],
    marginBottom: theme.spacing[2],
  },
  cardsGrid: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: theme.spacing[2],
    marginBottom: theme.spacing[3],
  },
  trendHeader: {
    paddingHorizontal: theme.spacing[4],
    paddingTop: theme.spacing[3],
  },
  trendTitleBlock: {
    gap: 2,
  },
  metricRow: {
    paddingHorizontal: theme.spacing[4],
    paddingTop: theme.spacing[2],
  },
  activeSummary: {
    paddingHorizontal: theme.spacing[4],
    paddingTop: theme.spacing[3],
    gap: 2,
  },
  activeLabel: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
  },
  activeValue: {
    fontSize: theme.fontSize.xl,
    fontWeight: theme.fontWeight.semibold,
    color: theme.colors.foreground,
  },
  activeMeta: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
  },
  chart: {
    flexDirection: "row",
    alignItems: "flex-end",
    gap: 6,
    height: 168,
    paddingHorizontal: theme.spacing[4],
    paddingVertical: theme.spacing[3],
  },
  barColumn: {
    width: 26,
    height: "100%",
    justifyContent: "flex-end",
    gap: 4,
  },
  barTrack: {
    flex: 1,
    justifyContent: "flex-end",
  },
  bar: {
    width: "100%",
    minHeight: 2,
    borderRadius: theme.borderRadius.base,
    backgroundColor: theme.colors.surface3,
  },
  barActive: {
    backgroundColor: theme.colors.accent,
  },
  barLabel: {
    fontSize: 9,
    color: theme.colors.foregroundMuted,
    textAlign: "center",
  },
  breakdownHeader: {
    paddingHorizontal: theme.spacing[4],
    paddingTop: theme.spacing[3],
    paddingBottom: theme.spacing[1],
  },
  breakdownRow: {
    paddingHorizontal: theme.spacing[4],
    paddingVertical: theme.spacing[2],
    gap: 4,
  },
  breakdownTitleRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
  },
  breakdownTitle: {
    flex: 1,
    fontSize: theme.fontSize.sm,
    color: theme.colors.foreground,
  },
  breakdownTokens: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
  },
  track: {
    height: 6,
    borderRadius: theme.borderRadius.full,
    backgroundColor: theme.colors.surface3,
    overflow: "hidden",
  },
  trackFill: {
    height: "100%",
    borderRadius: theme.borderRadius.full,
    backgroundColor: theme.colors.accent,
  },
  breakdownMeta: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
  },
}));

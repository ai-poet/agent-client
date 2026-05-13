import React from "react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Pressable, Text, View } from "react-native";
import type { DimensionValue } from "react-native";
import { StyleSheet } from "react-native-unistyles";
import { CLOUD_NAME } from "@/config/branding";
import { SegmentedControl } from "@/components/ui/segmented-control";
import { useSub2APIAuth } from "@/hooks/use-sub2api-auth";
import {
  useSub2APIAvailableGroups,
  useSub2APIGroupStatusEvents,
  useSub2APIGroupStatusHistory,
  useSub2APIGroupStatusRecords,
  useSub2APIGroupStatuses,
} from "@/hooks/use-sub2api-api";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";
import type {
  Sub2APIGroupStatusEvent,
  Sub2APIGroupStatusHistoryBucket,
  Sub2APIGroupStatusHistoryPeriod,
  Sub2APIGroupStatusItem,
  Sub2APIGroupStatusRecord,
} from "@/lib/sub2api-client";
import { SettingsSection } from "@/screens/settings/settings-section";
import { getErrorMessage } from "@/screens/settings/managed-provider-settings-shared";
import { managedProviderSettingsStyles as sharedStyles } from "@/screens/settings/managed-provider-settings-styles";
import { resolveGroupStatusDisplayName } from "@/screens/settings/paseo-cloud-model-status-utils";
import { settingsStyles } from "@/styles/settings";

type NormalizedStatus = "up" | "degraded" | "down" | "unknown";
type ModelStatusText = ReturnType<typeof getSub2APIMessages>["paseoCloudModelStatus"];

const POLL_INTERVAL_MS = 30_000;
const DETAIL_RECORD_LIMIT = 24;
const DETAIL_EVENT_LIMIT = 20;

const HISTORY_PERIOD_OPTIONS: Array<{
  value: Sub2APIGroupStatusHistoryPeriod;
  label: string;
}> = [
  { value: "24h", label: "24h" },
  { value: "7d", label: "7d" },
];

function normalizeStatus(status: string | null | undefined): NormalizedStatus {
  if (status === "up" || status === "degraded" || status === "down") {
    return status;
  }
  return "unknown";
}

function getItemStatus(item: Sub2APIGroupStatusItem): NormalizedStatus {
  return normalizeStatus(item.stable_status || item.latest_status);
}

function getStatusLabel(status: NormalizedStatus, text: ModelStatusText): string {
  return text.statusLabels[status];
}

function getStatusColor(status: NormalizedStatus): string {
  switch (status) {
    case "up":
      return "#16a34a";
    case "degraded":
      return "#d97706";
    case "down":
      return "#dc2626";
    default:
      return "#6b7280";
  }
}

function formatLatency(value: number | null | undefined): string {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return "--";
  }
  if (value < 1000) {
    return `${Math.round(value)} ms`;
  }
  const seconds = value / 1000;
  return `${seconds >= 10 ? seconds.toFixed(0) : seconds.toFixed(1)} s`;
}

function formatAvailability(value: number | null | undefined): string {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return "--";
  }
  const digits = value >= 99 ? 2 : 1;
  return `${value.toFixed(digits)}%`;
}

function availabilityWidth(value: number | null | undefined): DimensionValue {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return "0%";
  }
  return `${Math.max(0, Math.min(value, 100))}%` as DimensionValue;
}

function formatDateTime(value: string | null | undefined): string {
  if (!value) {
    return "--";
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleString();
}

function shorten(value: string | null | undefined, maxLength = 160): string {
  const trimmed = value?.trim() ?? "";
  if (!trimmed) {
    return "";
  }
  if (trimmed.length <= maxLength) {
    return trimmed;
  }
  return `${trimmed.slice(0, maxLength).trimEnd()}...`;
}

function SummaryCard({
  label,
  value,
  status,
}: {
  label: string;
  value: number;
  status: NormalizedStatus;
}) {
  return (
    <View style={styles.summaryCard}>
      <View style={[styles.statusDot, { backgroundColor: getStatusColor(status) }]} />
      <Text style={styles.summaryValue}>{value}</Text>
      <Text style={styles.summaryLabel}>{label}</Text>
    </View>
  );
}

function StatusMetric({ label, value }: { label: string; value: string }) {
  return (
    <View style={styles.metricCell}>
      <Text style={styles.metricLabel}>{label}</Text>
      <Text style={styles.metricValue}>{value}</Text>
    </View>
  );
}

function StatusRow({
  item,
  selected,
  groupNameById,
  onPress,
  text,
}: {
  item: Sub2APIGroupStatusItem;
  selected: boolean;
  groupNameById: ReadonlyMap<number, string>;
  onPress: () => void;
  text: ModelStatusText;
}) {
  const status = getItemStatus(item);
  const displayName = resolveGroupStatusDisplayName(item, groupNameById);
  return (
    <Pressable
      onPress={onPress}
      style={({ pressed }) => [
        styles.statusCard,
        selected && styles.statusCardSelected,
        pressed && sharedStyles.buttonPressed,
      ]}
    >
      <View style={styles.statusHeader}>
        <View style={styles.statusTitleBlock}>
          <View style={styles.statusTitleRow}>
            <View style={[styles.statusDot, { backgroundColor: getStatusColor(status) }]} />
            <Text style={styles.statusTitle} numberOfLines={1}>
              {displayName}
            </Text>
          </View>
          <Text style={styles.statusMeta}>
            {getStatusLabel(status, text)} - {text.observed} {formatDateTime(item.observed_at)}
          </Text>
        </View>
        <Text
          style={[
            sharedStyles.infoBadge,
            status === "up"
              ? sharedStyles.infoBadgeSuccess
              : status === "degraded"
                ? sharedStyles.infoBadgeWarning
                : status === "down"
                  ? sharedStyles.infoBadgeDanger
                  : sharedStyles.infoBadgeNeutral,
          ]}
        >
          {getStatusLabel(status, text)}
        </Text>
      </View>

      <View style={styles.metricsGrid}>
        <StatusMetric label={text.latency} value={formatLatency(item.latency_ms)} />
        <StatusMetric label="24h" value={formatAvailability(item.availability_24h)} />
        <StatusMetric label="7d" value={formatAvailability(item.availability_7d)} />
      </View>

      <View style={styles.availabilityTrack}>
        <View
          style={[
            styles.availabilityFill,
            {
              width: availabilityWidth(item.availability_24h),
              backgroundColor: getStatusColor(status),
            },
          ]}
        />
      </View>
    </Pressable>
  );
}

function HistoryRow({
  bucket,
  text,
}: {
  bucket: Sub2APIGroupStatusHistoryBucket;
  text: ModelStatusText;
}) {
  const status = normalizeStatus(bucket.latest_status);
  return (
    <View style={styles.detailRow}>
      <View style={styles.detailRowMain}>
        <Text style={styles.detailRowTitle}>{formatDateTime(bucket.bucket_start)}</Text>
        <Text style={styles.detailRowMeta}>
          {text.availability} {formatAvailability(bucket.availability)} - {text.average}{" "}
          {formatLatency(bucket.avg_latency_ms)}
        </Text>
      </View>
      <Text style={[sharedStyles.infoBadge, sharedStyles.infoBadgeNeutral]}>
        {bucket.total_count ?? 0} {text.probes}
      </Text>
      <View style={styles.historyBar}>
        <View
          style={[
            styles.historyBarFill,
            {
              width: availabilityWidth(bucket.availability),
              backgroundColor: getStatusColor(status === "unknown" ? "up" : status),
            },
          ]}
        />
      </View>
    </View>
  );
}

function RecordRow({ record, text }: { record: Sub2APIGroupStatusRecord; text: ModelStatusText }) {
  const status = normalizeStatus(record.status);
  const excerpt = shorten(record.error_detail || record.response_excerpt, 140);
  return (
    <View style={styles.detailRow}>
      <View style={styles.detailRowMain}>
        <View style={styles.statusTitleRow}>
          <View style={[styles.statusDotSmall, { backgroundColor: getStatusColor(status) }]} />
          <Text style={styles.detailRowTitle}>{getStatusLabel(status, text)}</Text>
        </View>
        <Text style={styles.detailRowMeta}>
          {formatDateTime(record.observed_at)} - {formatLatency(record.latency_ms)} - {text.http}{" "}
          {record.http_code ?? "--"}
        </Text>
        {excerpt ? <Text style={styles.detailRowMeta}>{excerpt}</Text> : null}
      </View>
    </View>
  );
}

function EventRow({ event, text }: { event: Sub2APIGroupStatusEvent; text: ModelStatusText }) {
  const status = normalizeStatus(event.to_status);
  const excerpt = shorten(event.error_detail, 140);
  return (
    <View style={styles.detailRow}>
      <View style={styles.detailRowMain}>
        <View style={styles.statusTitleRow}>
          <View style={[styles.statusDotSmall, { backgroundColor: getStatusColor(status) }]} />
          <Text style={styles.detailRowTitle}>
            {event.event_type || text.statusFallback}: {event.from_status || text.unknownStatus}{" "}
            {"->"} {event.to_status || text.unknownStatus}
          </Text>
        </View>
        <Text style={styles.detailRowMeta}>
          {formatDateTime(event.observed_at)} - {formatLatency(event.latency_ms)} - {text.http}{" "}
          {event.http_code ?? "--"}
        </Text>
        {excerpt ? <Text style={styles.detailRowMeta}>{excerpt}</Text> : null}
      </View>
    </View>
  );
}

function DetailPanel({
  selected,
  groupNameById,
  period,
  onPeriodChange,
  history,
  records,
  events,
  isLoading,
  error,
  text,
}: {
  selected: Sub2APIGroupStatusItem;
  groupNameById: ReadonlyMap<number, string>;
  period: Sub2APIGroupStatusHistoryPeriod;
  onPeriodChange: (period: Sub2APIGroupStatusHistoryPeriod) => void;
  history: Sub2APIGroupStatusHistoryBucket[];
  records: Sub2APIGroupStatusRecord[];
  events: Sub2APIGroupStatusEvent[];
  isLoading: boolean;
  error: unknown;
  text: ModelStatusText;
}) {
  const displayName = resolveGroupStatusDisplayName(selected, groupNameById);
  return (
    <View style={[settingsStyles.card, styles.cardBody]}>
      <View style={styles.detailHeader}>
        <View>
          <Text style={styles.cardTitle}>{displayName}</Text>
          <Text style={styles.hintText}>{text.detailHint}</Text>
        </View>
        <SegmentedControl
          options={HISTORY_PERIOD_OPTIONS}
          value={period}
          onValueChange={onPeriodChange}
          size="sm"
        />
      </View>

      {error ? <Text style={styles.errorText}>{getErrorMessage(error)}</Text> : null}
      {isLoading ? <Text style={styles.hintText}>{text.loadingDetails}</Text> : null}

      {!isLoading && !error ? (
        <View style={styles.detailGrid}>
          <View style={styles.detailColumn}>
            <Text style={styles.detailSectionTitle}>{text.availabilityHistory}</Text>
            {history.length === 0 ? (
              <Text style={styles.hintText}>{text.noHistory}</Text>
            ) : (
              <View style={styles.detailList}>
                {history.slice(-8).map((bucket) => (
                  <HistoryRow key={bucket.bucket_start} bucket={bucket} text={text} />
                ))}
              </View>
            )}
          </View>

          <View style={styles.detailColumn}>
            <Text style={styles.detailSectionTitle}>{text.recentProbes}</Text>
            {records.length === 0 ? (
              <Text style={styles.hintText}>{text.noRecords}</Text>
            ) : (
              <View style={styles.detailList}>
                {records.slice(0, 8).map((record) => (
                  <RecordRow key={record.id} record={record} text={text} />
                ))}
              </View>
            )}
          </View>

          <View style={styles.detailColumn}>
            <Text style={styles.detailSectionTitle}>{text.statusEvents}</Text>
            {events.length === 0 ? (
              <Text style={styles.hintText}>{text.noEvents}</Text>
            ) : (
              <View style={styles.detailList}>
                {events.slice(0, 8).map((event) => (
                  <EventRow key={event.id} event={event} text={text} />
                ))}
              </View>
            )}
          </View>
        </View>
      ) : null}
    </View>
  );
}

export function PaseoCloudModelStatusSection() {
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).paseoCloudModelStatus, [locale]);
  const { isLoggedIn } = useSub2APIAuth();
  const [selectedGroupId, setSelectedGroupId] = useState<number | null>(null);
  const [detailPeriod, setDetailPeriod] = useState<Sub2APIGroupStatusHistoryPeriod>("24h");
  const groupsQuery = useSub2APIAvailableGroups();
  const statusesQuery = useSub2APIGroupStatuses();
  const statuses = statusesQuery.data ?? [];
  const groupNameById = useMemo(
    () => new Map((groupsQuery.data ?? []).map((group) => [group.id, group.name] as const)),
    [groupsQuery.data],
  );
  const selectedItem = useMemo(
    () => statuses.find((item) => item.group_id === selectedGroupId) ?? statuses[0] ?? null,
    [selectedGroupId, statuses],
  );
  const activeGroupId = selectedItem?.group_id ?? null;
  const historyQuery = useSub2APIGroupStatusHistory(activeGroupId, detailPeriod, {
    enabled: isLoggedIn && activeGroupId !== null,
  });
  const recordsQuery = useSub2APIGroupStatusRecords(activeGroupId, DETAIL_RECORD_LIMIT, {
    enabled: isLoggedIn && activeGroupId !== null,
  });
  const eventsQuery = useSub2APIGroupStatusEvents(activeGroupId, DETAIL_EVENT_LIMIT, {
    enabled: isLoggedIn && activeGroupId !== null,
  });

  useEffect(() => {
    if (selectedGroupId !== null || statuses.length === 0) {
      return;
    }
    setSelectedGroupId(statuses[0].group_id);
  }, [selectedGroupId, statuses]);

  useEffect(() => {
    if (!isLoggedIn) {
      return;
    }
    const timer = setInterval(() => {
      void statusesQuery.refetch();
    }, POLL_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [isLoggedIn, statusesQuery]);

  const counts = useMemo(() => {
    const next = { up: 0, degraded: 0, down: 0, unknown: 0 };
    for (const item of statuses) {
      next[getItemStatus(item)] += 1;
    }
    return next;
  }, [statuses]);

  const handleRefresh = useCallback(() => {
    void Promise.all([
      groupsQuery.refetch(),
      statusesQuery.refetch(),
      historyQuery.refetch(),
      recordsQuery.refetch(),
      eventsQuery.refetch(),
    ]);
  }, [eventsQuery, groupsQuery, historyQuery, recordsQuery, statusesQuery]);

  const detailError = historyQuery.error || recordsQuery.error || eventsQuery.error;
  const detailLoading = historyQuery.isLoading || recordsQuery.isLoading || eventsQuery.isLoading;

  return (
    <SettingsSection title={text.title}>
      {!isLoggedIn ? (
        <View style={[settingsStyles.card, styles.cardBody]}>
          <Text style={styles.hintText}>{text.signInHint(CLOUD_NAME)}</Text>
        </View>
      ) : null}

      {isLoggedIn ? (
        <View style={styles.sectionStack}>
          <View style={[settingsStyles.card, styles.cardBody]}>
            <View style={styles.toolbarHeader}>
              <View>
                <Text style={styles.cardTitle}>{text.runtimeHealth}</Text>
                <Text style={styles.hintText}>{text.refreshInterval}</Text>
              </View>
              <Pressable
                onPress={handleRefresh}
                style={({ pressed }) => [
                  sharedStyles.secondaryButton,
                  pressed && sharedStyles.buttonPressed,
                ]}
              >
                <Text style={sharedStyles.secondaryButtonText}>{text.refresh}</Text>
              </Pressable>
            </View>

            <View style={styles.summaryGrid}>
              <SummaryCard label={text.statusLabels.up} value={counts.up} status="up" />
              <SummaryCard
                label={text.statusLabels.degraded}
                value={counts.degraded}
                status="degraded"
              />
              <SummaryCard label={text.statusLabels.down} value={counts.down} status="down" />
              <SummaryCard
                label={text.statusLabels.unknown}
                value={counts.unknown}
                status="unknown"
              />
            </View>
          </View>

          {statusesQuery.error ? (
            <View style={[settingsStyles.card, styles.cardBody]}>
              <Text style={styles.errorText}>{getErrorMessage(statusesQuery.error)}</Text>
            </View>
          ) : null}

          <View style={[settingsStyles.card, styles.cardBody]}>
            <Text style={styles.cardTitle}>{text.groups}</Text>
            {statusesQuery.isLoading ? (
              <Text style={styles.hintText}>{text.loadingGroups}</Text>
            ) : statuses.length === 0 ? (
              <Text style={styles.hintText}>{text.noGroups(CLOUD_NAME)}</Text>
            ) : (
              <View style={styles.statusList}>
                {statuses.map((item) => (
                  <StatusRow
                    key={item.group_id}
                    item={item}
                    selected={item.group_id === activeGroupId}
                    groupNameById={groupNameById}
                    onPress={() => setSelectedGroupId(item.group_id)}
                    text={text}
                  />
                ))}
              </View>
            )}
          </View>

          {selectedItem ? (
            <DetailPanel
              selected={selectedItem}
              groupNameById={groupNameById}
              period={detailPeriod}
              onPeriodChange={setDetailPeriod}
              history={historyQuery.data ?? []}
              records={recordsQuery.data ?? []}
              events={eventsQuery.data ?? []}
              isLoading={detailLoading}
              error={detailError}
              text={text}
            />
          ) : null}
        </View>
      ) : null}
    </SettingsSection>
  );
}

const styles = StyleSheet.create((theme) => ({
  sectionStack: {
    gap: theme.spacing[4],
  },
  cardBody: {
    padding: theme.spacing[4],
    gap: theme.spacing[3],
  },
  toolbarHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: theme.spacing[3],
  },
  cardTitle: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.medium,
  },
  hintText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  errorText: {
    color: theme.colors.destructive,
    fontSize: theme.fontSize.xs,
  },
  summaryGrid: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: theme.spacing[3],
  },
  summaryCard: {
    flexGrow: 1,
    minWidth: 120,
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.md,
    backgroundColor: theme.colors.surface0,
    padding: theme.spacing[3],
    gap: theme.spacing[1],
  },
  summaryValue: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.lg,
    fontWeight: theme.fontWeight.medium,
  },
  summaryLabel: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  statusList: {
    gap: theme.spacing[3],
  },
  statusCard: {
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.md,
    backgroundColor: theme.colors.surface0,
    padding: theme.spacing[3],
    gap: theme.spacing[3],
  },
  statusCardSelected: {
    borderColor: theme.colors.accent,
    backgroundColor: "rgba(59, 130, 246, 0.08)",
  },
  statusHeader: {
    flexDirection: "row",
    justifyContent: "space-between",
    gap: theme.spacing[3],
  },
  statusTitleBlock: {
    flex: 1,
    minWidth: 0,
    gap: theme.spacing[1],
  },
  statusTitleRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
  },
  statusDot: {
    width: 10,
    height: 10,
    borderRadius: 999,
  },
  statusDotSmall: {
    width: 8,
    height: 8,
    borderRadius: 999,
  },
  statusTitle: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.medium,
    flexShrink: 1,
  },
  statusMeta: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  metricsGrid: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: theme.spacing[3],
  },
  metricCell: {
    flexGrow: 1,
    minWidth: 110,
    gap: theme.spacing[1],
  },
  metricLabel: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  metricValue: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.medium,
  },
  availabilityTrack: {
    height: 6,
    borderRadius: 999,
    backgroundColor: theme.colors.surface2,
    overflow: "hidden",
  },
  availabilityFill: {
    height: "100%",
    borderRadius: 999,
  },
  detailHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: theme.spacing[3],
  },
  detailGrid: {
    gap: theme.spacing[4],
  },
  detailColumn: {
    gap: theme.spacing[2],
  },
  detailSectionTitle: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.medium,
  },
  detailList: {
    gap: theme.spacing[2],
  },
  detailRow: {
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.md,
    backgroundColor: theme.colors.surface0,
    padding: theme.spacing[3],
    gap: theme.spacing[2],
  },
  detailRowMain: {
    gap: theme.spacing[1],
  },
  detailRowTitle: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.medium,
  },
  detailRowMeta: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  historyBar: {
    height: 5,
    borderRadius: 999,
    backgroundColor: theme.colors.surface2,
    overflow: "hidden",
  },
  historyBarFill: {
    height: "100%",
    borderRadius: 999,
  },
}));

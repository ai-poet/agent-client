import { memo } from "react";
import { Text, View } from "react-native";
import { StyleSheet } from "react-native-unistyles";
import { getPlatformColors, getStatusColor, getStatusLabel } from "@/utils/platform-colors";
import type { Sub2APIModelCatalogItem, Sub2APIGroupStatusItem } from "@/lib/sub2api-client";
import {
  formatActualPaidPrice,
  formatUsdPrice,
  getActualPaidSectionLabel,
  type ActualPaidPricingContext,
} from "@/components/model-square/model-card-pricing";
import { getSub2APIMessages, type Sub2APILocale } from "@/i18n/sub2api";

function fmtPct(value: number | null): string {
  if (value == null || !Number.isFinite(value)) return "--";
  return `${value.toFixed(1)}%`;
}

export interface ModelCardProps {
  item: Sub2APIModelCatalogItem;
  status?: Sub2APIGroupStatusItem | null;
  actualPaidPricing?: ActualPaidPricingContext | null;
  locale?: Sub2APILocale | null;
}

export const ModelCard = memo(function ModelCard({
  item,
  status,
  actualPaidPricing,
  locale,
}: ModelCardProps) {
  const text = getSub2APIMessages(locale).modelCard;
  const platform = getPlatformColors(item.platform);
  const stableStatus = status?.stable_status ?? "unknown";
  const statusColor = getStatusColor(stableStatus);
  const statusLabel = getStatusLabel(stableStatus);
  const availability = status?.availability_24h;

  const isToken = item.billing_mode === "token" || item.billing_mode === "";
  const isPerRequest = item.billing_mode === "per_request";
  const isImage = item.billing_mode === "image";

  // Official prices
  const offInput = item.official_pricing.input_per_mtok_usd;
  const offOutput = item.official_pricing.output_per_mtok_usd;
  const offReq = item.official_pricing.per_request_usd;
  const offImg = item.official_pricing.per_image_usd;

  // Actual paid (effective) prices
  const effInput = item.effective_pricing_usd.input_per_mtok_usd;
  const effOutput = item.effective_pricing_usd.output_per_mtok_usd;
  const effReq = item.effective_pricing_usd.per_request_usd;
  const effImg = item.effective_pricing_usd.per_image_usd;

  const savings = item.comparison.savings_percent;
  const isCheaper = item.comparison.is_cheaper_than_official;

  const details = item.pricing_details;
  const caching = details?.supports_prompt_caching;

  return (
    <View style={styles.card}>
      {/* Header: Platform badge + Model name */}
      <View style={styles.header}>
        <View
          style={[
            styles.platformBadge,
            { backgroundColor: platform.badge.bg, borderColor: platform.badge.border },
          ]}
        >
          <Text style={[styles.platformBadgeText, { color: platform.badge.text }]}>
            {platform.label}
          </Text>
        </View>
        <Text style={styles.modelName} numberOfLines={1}>
          {item.display_name || item.model}
        </Text>
      </View>

      {/* Best group + rate */}
      <Text style={styles.groupHint}>
        {item.best_group.name} · {item.best_group.rate_multiplier}x
        {item.available_group_count > 1 ? ` · ${text.groups(item.available_group_count)}` : ""}
      </Text>

      {/* Status indicator */}
      {status ? (
        <View style={styles.statusRow}>
          <View style={[styles.statusDot, { backgroundColor: statusColor }]} />
          <Text style={[styles.statusText, { color: statusColor }]}>{statusLabel}</Text>
          {availability != null ? (
            <Text style={styles.mutedText}>24h: {availability.toFixed(1)}%</Text>
          ) : null}
        </View>
      ) : null}

      {/* Pricing — actual paid (effective) is primary */}
      <View style={styles.pricingSection}>
        <Text style={styles.sectionLabel}>
          {getActualPaidSectionLabel(actualPaidPricing, locale)}
        </Text>
        {isToken ? (
          <>
            <PriceRow
              label={text.input}
              official={offInput}
              effective={effInput}
              cheaper={isCheaper}
              actualPaidPricing={actualPaidPricing}
              locale={locale}
            />
            <PriceRow
              label={text.output}
              official={offOutput}
              effective={effOutput}
              cheaper={isCheaper}
              actualPaidPricing={actualPaidPricing}
              locale={locale}
            />
          </>
        ) : isPerRequest ? (
          <PriceRow
            label={text.perRequest}
            official={offReq}
            effective={effReq}
            cheaper={isCheaper}
            actualPaidPricing={actualPaidPricing}
            locale={locale}
          />
        ) : isImage ? (
          <PriceRow
            label={text.perImage}
            official={offImg}
            effective={effImg}
            cheaper={isCheaper}
            actualPaidPricing={actualPaidPricing}
            locale={locale}
          />
        ) : null}
        {savings != null ? (
          <View style={styles.priceRow}>
            <Text style={styles.mutedText}>{text.savings}</Text>
            <Text style={[styles.savingsValue, isCheaper && styles.green]}>{fmtPct(savings)}</Text>
          </View>
        ) : null}
      </View>

      {/* Cache write/read if token mode and available */}
      {isToken &&
      (item.effective_pricing_usd.cache_write_per_mtok_usd != null ||
        item.effective_pricing_usd.cache_read_per_mtok_usd != null) ? (
        <View style={styles.pricingSection}>
          <Text style={styles.sectionLabel}>{text.cachePricing}</Text>
          {item.effective_pricing_usd.cache_write_per_mtok_usd != null ? (
            <PriceRow
              label={text.write}
              official={item.official_pricing.cache_write_per_mtok_usd}
              effective={item.effective_pricing_usd.cache_write_per_mtok_usd}
              cheaper={isCheaper}
              actualPaidPricing={actualPaidPricing}
              locale={locale}
            />
          ) : null}
          {item.effective_pricing_usd.cache_read_per_mtok_usd != null ? (
            <PriceRow
              label={text.read}
              official={item.official_pricing.cache_read_per_mtok_usd}
              effective={item.effective_pricing_usd.cache_read_per_mtok_usd}
              cheaper={isCheaper}
              actualPaidPricing={actualPaidPricing}
              locale={locale}
            />
          ) : null}
        </View>
      ) : null}

      {/* Tags: billing mode + caching */}
      <View style={styles.tagWrap}>
        <View style={[styles.tag, { borderColor: platform.badge.border }]}>
          <View style={[styles.tagDot, { backgroundColor: platform.dot }]} />
          <Text style={styles.tagText}>{item.billing_mode || "token"}</Text>
        </View>
        {caching ? (
          <View style={[styles.tag, { borderColor: "rgba(34,197,94,0.3)" }]}>
            <View style={[styles.tagDot, { backgroundColor: "#22c55e" }]} />
            <Text style={styles.tagText}>{text.promptCaching}</Text>
          </View>
        ) : null}
      </View>

      {/* Other groups */}
      {item.other_groups && item.other_groups.length > 0 ? (
        <View style={styles.otherGroupsSection}>
          <Text style={styles.sectionLabel}>{text.alsoAvailableIn}</Text>
          {item.other_groups.map((og) => (
            <Text key={og.group.id} style={styles.mutedText}>
              {og.group.name} ({og.group.rate_multiplier}x)
              {og.comparison.savings_percent != null
                ? ` · ${fmtPct(og.comparison.savings_percent)} ${text.savingsSuffix}`
                : ""}
            </Text>
          ))}
        </View>
      ) : null}
    </View>
  );
});

function PriceRow({
  label,
  official,
  effective,
  cheaper,
  actualPaidPricing,
  locale,
}: {
  label: string;
  official: number | null;
  effective: number | null;
  cheaper: boolean;
  actualPaidPricing?: ActualPaidPricingContext | null;
  locale?: Sub2APILocale | null;
}) {
  return (
    <View style={styles.priceRow}>
      <Text style={styles.mutedText}>{label}</Text>
      <View style={styles.priceValues}>
        <Text style={styles.officialPrice}>{formatUsdPrice(official)}</Text>
        <Text style={styles.arrow}>→</Text>
        <Text style={[styles.effectivePrice, cheaper && styles.green]}>
          {formatActualPaidPrice(
            effective,
            actualPaidPricing
              ? { ...actualPaidPricing, locale: locale ?? actualPaidPricing.locale }
              : actualPaidPricing,
          )}
        </Text>
      </View>
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  card: {
    borderRadius: theme.borderRadius.xl,
    borderWidth: 1,
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.surface1,
    padding: theme.spacing[4],
    gap: theme.spacing[3],
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
  },
  platformBadge: {
    borderRadius: theme.borderRadius.md,
    borderWidth: 1,
    paddingHorizontal: theme.spacing[2],
    paddingVertical: 2,
  },
  platformBadgeText: {
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.medium,
  },
  modelName: {
    fontSize: theme.fontSize.base,
    fontWeight: theme.fontWeight.bold,
    color: theme.colors.foreground,
    flexShrink: 1,
  },
  groupHint: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
  },
  statusRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
  },
  statusDot: {
    width: 8,
    height: 8,
    borderRadius: theme.borderRadius.full,
  },
  statusText: {
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.medium,
  },
  mutedText: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
  },
  sectionLabel: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
    fontWeight: theme.fontWeight.medium,
    marginBottom: theme.spacing[1],
  },
  pricingSection: {
    gap: theme.spacing[1],
  },
  priceRow: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "center",
  },
  priceValues: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[1],
  },
  officialPrice: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
    textDecorationLine: "line-through",
  },
  arrow: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
  },
  effectivePrice: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foreground,
    fontWeight: theme.fontWeight.medium,
  },
  green: {
    color: "#22c55e",
  },
  savingsValue: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
    fontWeight: theme.fontWeight.medium,
  },
  tagWrap: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: theme.spacing[1],
  },
  tag: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[1],
    borderRadius: theme.borderRadius.lg,
    borderWidth: 1,
    paddingHorizontal: theme.spacing[2],
    paddingVertical: theme.spacing[1],
  },
  tagDot: {
    width: 6,
    height: 6,
    borderRadius: theme.borderRadius.full,
  },
  tagText: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foreground,
  },
  otherGroupsSection: {
    gap: theme.spacing[1],
  },
}));

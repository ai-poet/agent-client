import { useMemo, useState } from "react";
import { Pressable, Text, View } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { settingsStyles } from "@/styles/settings";
import { useHostRuntimeIsConnected } from "@/runtime/host-runtime";
import { useProvidersSnapshot } from "@/hooks/use-providers-snapshot";
import { buildProviderDefinitions } from "@/utils/provider-definitions";
import { getProviderIcon } from "@/components/provider-icons";
import { ProviderDiagnosticSheet } from "@/components/provider-diagnostic-sheet";
import { LoadingSpinner } from "@/components/ui/loading-spinner";
import { StatusBadge } from "@/components/ui/status-badge";
import { SettingsSection } from "@/screens/settings/settings-section";
import { RotateCw } from "lucide-react-native";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";

export interface ProvidersSectionProps {
  serverId: string;
}

export function ProvidersSection({ serverId }: ProvidersSectionProps) {
  const { theme } = useUnistyles();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).settings.providers, [locale]);
  const isConnected = useHostRuntimeIsConnected(serverId);
  const { entries, isLoading, isRefreshing, refresh } = useProvidersSnapshot(serverId);
  const [diagnosticProvider, setDiagnosticProvider] = useState<string | null>(null);
  const providerDefinitions = buildProviderDefinitions(entries);
  const providerRefreshInFlight =
    isRefreshing || (entries?.some((entry) => entry.status === "loading") ?? false);
  const hasServer = serverId.length > 0;

  const refreshAction =
    hasServer && isConnected ? (
      <Pressable
        onPress={() => void refresh()}
        disabled={providerRefreshInFlight}
        hitSlop={8}
        style={settingsStyles.sectionHeaderLink}
        accessibilityRole="button"
        accessibilityLabel={providerRefreshInFlight ? text.refreshing : text.refresh}
      >
        {providerRefreshInFlight ? (
          <LoadingSpinner size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
        ) : (
          <RotateCw size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
        )}
      </Pressable>
    ) : undefined;

  return (
    <>
      <SettingsSection
        title={text.title}
        trailing={refreshAction}
        testID="host-page-providers-card"
        style={styles.sectionSpacing}
      >
        {!hasServer || !isConnected ? (
          <View style={[settingsStyles.card, styles.emptyCard]}>
            <Text style={styles.emptyText}>{text.connectToSee}</Text>
          </View>
        ) : isLoading ? (
          <View style={[settingsStyles.card, styles.emptyCard]}>
            <Text style={styles.emptyText}>{text.loading}</Text>
          </View>
        ) : (
          <View style={settingsStyles.card}>
            {providerDefinitions.map((def, index) => {
              const entry = entries?.find((e) => e.provider === def.id);
              const status = entry?.status ?? "unavailable";
              const ProviderIcon = getProviderIcon(def.id);
              const providerError =
                status === "error" &&
                typeof entry?.error === "string" &&
                entry.error.trim().length > 0
                  ? entry.error.trim()
                  : null;
              const modelCount = entry?.models?.length ?? 0;

              return (
                <Pressable
                  key={def.id}
                  style={[settingsStyles.row, index > 0 && settingsStyles.rowBorder]}
                  onPress={() => setDiagnosticProvider(def.id)}
                  accessibilityRole="button"
                >
                  <View style={settingsStyles.rowContent}>
                    <View style={styles.titleRow}>
                      <ProviderIcon size={theme.iconSize.sm} color={theme.colors.foreground} />
                      <Text style={settingsStyles.rowTitle}>{def.label}</Text>
                    </View>
                    {providerError ? (
                      <Text style={styles.errorText} numberOfLines={3}>
                        {providerError}
                      </Text>
                    ) : null}
                    {status === "ready" && modelCount > 0 ? (
                      <Text style={settingsStyles.rowHint}>{text.modelCount(modelCount)}</Text>
                    ) : null}
                  </View>
                  <StatusBadge
                    label={
                      status === "ready"
                        ? text.statuses.available
                        : status === "error"
                          ? text.statuses.error
                          : status === "loading"
                            ? text.statuses.loading
                            : text.statuses.notInstalled
                    }
                    variant={
                      status === "ready" ? "success" : status === "error" ? "error" : "muted"
                    }
                  />
                </Pressable>
              );
            })}
          </View>
        )}
      </SettingsSection>

      {diagnosticProvider ? (
        <ProviderDiagnosticSheet
          provider={diagnosticProvider}
          visible
          onClose={() => setDiagnosticProvider(null)}
          serverId={serverId}
        />
      ) : null}
    </>
  );
}

const styles = StyleSheet.create((theme) => ({
  sectionSpacing: {
    marginBottom: theme.spacing[4],
  },
  emptyCard: {
    padding: theme.spacing[4],
    alignItems: "center",
  },
  emptyText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
  },
  titleRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
  },
  errorText: {
    color: theme.colors.palette.red[300],
    fontSize: theme.fontSize.xs,
    marginTop: theme.spacing[1],
  },
}));

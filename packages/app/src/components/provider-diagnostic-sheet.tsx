import {
  AlertCircle,
  Check,
  Download,
  ExternalLink,
  RotateCw,
  Search,
  X,
} from "lucide-react-native";
import { useCallback, useEffect, useMemo, useState } from "react";
import { ActivityIndicator, Alert, Pressable, ScrollView, Text, View } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { AdaptiveModalSheet, AdaptiveTextInput } from "@/components/adaptive-modal-sheet";
import { LoadingSpinner } from "@/components/ui/loading-spinner";
import { Button } from "@/components/ui/button";
import { FactRow } from "@/components/ui/fact-row";
import { StatusPanel } from "@/components/ui/status-panel";
import { isWeb } from "@/constants/platform";
import { Fonts } from "@/constants/theme";
import { useProvidersSnapshot } from "@/hooks/use-providers-snapshot";
import { useHostRuntimeClient } from "@/runtime/host-runtime";
import { resolveProviderLabel } from "@/utils/provider-definitions";
import { classifyInstallOutcome } from "@/utils/provider-presentation";
import { formatTimeAgo } from "@/utils/time";
import { openExternalUrl } from "@/utils/open-external-url";
import { shouldUseDesktopDaemon, installModelCli } from "@/desktop/daemon/desktop-daemon";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";
import type {
  AgentCapabilityFlags,
  AgentModelDefinition,
  AgentProvider,
} from "@server/server/agent/agent-sdk-types";

const CAPABILITY_ORDER: (keyof AgentCapabilityFlags)[] = [
  "supportsToolInvocations",
  "supportsSessionPersistence",
  "supportsMcpServers",
  "supportsReasoningStream",
];

interface ProviderDiagnosticSheetProps {
  provider: string;
  visible: boolean;
  onClose: () => void;
  serverId: string;
}

export function ProviderDiagnosticSheet({
  provider,
  visible,
  onClose,
  serverId,
}: ProviderDiagnosticSheetProps) {
  const { theme } = useUnistyles();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).settings.providers, [locale]);
  const client = useHostRuntimeClient(serverId);
  const { entries: snapshotEntries, refresh, isRefreshing } = useProvidersSnapshot(serverId);
  const [diagnostic, setDiagnostic] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [query, setQuery] = useState("");

  const providerLabel = resolveProviderLabel(provider, snapshotEntries);
  const providerEntry = useMemo(
    () => snapshotEntries?.find((entry) => entry.provider === provider),
    [snapshotEntries, provider],
  );
  const models = providerEntry?.models ?? [];
  const isReady = providerEntry?.status === "ready";
  const providerSnapshotRefreshing = providerEntry?.status === "loading";
  const providerErrorMessage =
    providerEntry?.status === "error" ? (providerEntry.error ?? "Unknown error") : null;
  const refreshInFlight = isRefreshing || providerSnapshotRefreshing || loading;

  const [clockTick, setClockTick] = useState(0);
  useEffect(() => {
    if (!visible) return;
    const id = setInterval(() => setClockTick((t) => t + 1), 10_000);
    return () => clearInterval(id);
  }, [visible]);
  const fetchedAtLabel = useMemo(() => {
    if (!providerEntry?.fetchedAt) return null;
    return formatTimeAgo(new Date(providerEntry.fetchedAt));
    // clockTick triggers re-computation on timer
  }, [providerEntry?.fetchedAt, clockTick]);

  const q = query.trim().toLowerCase();
  const filteredModels = q
    ? models.filter((m) => m.label.toLowerCase().includes(q) || m.id.toLowerCase().includes(q))
    : models;

  const fetchDiagnostic = useCallback(
    async (options?: { keepCurrent?: boolean }) => {
      if (!client || !provider) return;

      setLoading(true);
      if (!options?.keepCurrent) {
        setDiagnostic(null);
      }

      try {
        const result = await client.getProviderDiagnostic(provider as AgentProvider);
        setDiagnostic(result.diagnostic);
      } catch (err) {
        setDiagnostic(err instanceof Error ? err.message : "Failed to fetch diagnostic");
      } finally {
        setLoading(false);
      }
    },
    [client, provider],
  );

  const handleRefresh = useCallback(() => {
    if (!provider) {
      return;
    }
    void Promise.all([refresh([provider as AgentProvider]), fetchDiagnostic()]).catch((err) => {
      setDiagnostic(err instanceof Error ? err.message : "Failed to refresh provider");
    });
  }, [fetchDiagnostic, provider, refresh]);

  useEffect(() => {
    if (visible) {
      fetchDiagnostic();
    } else {
      setDiagnostic(null);
      setQuery("");
    }
  }, [visible, fetchDiagnostic]);

  const canInstallHere = shouldUseDesktopDaemon();
  const showInstallAction = Boolean(providerEntry?.installCommand) && !isReady;

  const handleInstall = useCallback(() => {
    if (installing) {
      return;
    }
    const previousVersion = providerEntry?.version;
    setInstalling(true);
    void installModelCli(provider)
      .then(async () => {
        const refreshed = await refresh([provider as AgentProvider]);
        const nextEntry = refreshed?.find((entry) => entry.provider === provider);
        const outcome = classifyInstallOutcome({ previousVersion, nextEntry });
        // Report what actually happened rather than assuming a zero exit means success.
        if (outcome.kind === "not-detected") {
          Alert.alert(text.installFailed, text.installNotDetected(providerLabel));
        } else if (outcome.kind === "unchanged") {
          Alert.alert(providerLabel, text.installUnchanged(providerLabel, outcome.version));
        } else {
          Alert.alert(
            providerLabel,
            outcome.version
              ? text.installSucceeded(providerLabel, outcome.version)
              : text.installSucceededUnknownVersion(providerLabel),
          );
        }
        void fetchDiagnostic({ keepCurrent: true });
      })
      .catch((error) => {
        console.error("[ProviderDiagnostic] Install failed", error);
        Alert.alert(text.installFailed, error instanceof Error ? error.message : String(error));
      })
      .finally(() => {
        setInstalling(false);
      });
  }, [installing, provider, providerEntry?.version, providerLabel, refresh, fetchDiagnostic, text]);

  function renderModelsBody() {
    if (models.length === 0 && providerSnapshotRefreshing) {
      return <StatusPanel title={text.loadingModels} loading />;
    }
    if (models.length === 0 && providerErrorMessage) {
      return (
        <StatusPanel
          title={text.statuses.error}
          description={providerErrorMessage}
          error
          icon={<AlertCircle size={theme.iconSize.md} color={theme.colors.foregroundMuted} />}
        />
      );
    }
    if (models.length === 0) {
      return <StatusPanel title={text.noModels} />;
    }
    if (filteredModels.length === 0) {
      return (
        <StatusPanel
          title={text.noMatchingModels}
          icon={<Search size={theme.iconSize.md} color={theme.colors.foregroundMuted} />}
        />
      );
    }
    return filteredModels.map((model: AgentModelDefinition, index) => (
      <View key={model.id} style={[sheetStyles.modelRow, index > 0 && sheetStyles.modelRowBorder]}>
        <Text style={sheetStyles.modelLabel} numberOfLines={1}>
          {model.label}
        </Text>
        <Text style={sheetStyles.modelId} numberOfLines={1} selectable>
          {model.id}
        </Text>
      </View>
    ));
  }

  return (
    <AdaptiveModalSheet
      title={providerLabel}
      visible={visible}
      onClose={onClose}
      snapPoints={["50%", "85%"]}
      scrollable={false}
      headerActions={
        <Pressable
          onPress={handleRefresh}
          disabled={refreshInFlight}
          hitSlop={8}
          style={({ hovered, pressed }) => [
            sheetStyles.iconButton,
            (hovered || pressed) && sheetStyles.iconButtonHovered,
            refreshInFlight ? sheetStyles.disabled : null,
          ]}
          accessibilityRole="button"
          accessibilityLabel={refreshInFlight ? text.refreshing : text.refresh}
        >
          {refreshInFlight ? (
            <LoadingSpinner size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
          ) : (
            <RotateCw size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
          )}
        </Pressable>
      }
    >
      <View style={sheetStyles.section}>
        <View style={sheetStyles.factsCard}>
          <FactRow
            label={text.facts.version}
            value={providerEntry?.version ?? text.facts.notDetected}
            mono={Boolean(providerEntry?.version)}
          />
          {providerEntry?.configDir ? (
            <FactRow label={text.facts.configDir} value={providerEntry.configDir} mono copyable />
          ) : null}
          {providerEntry?.installCommand ? (
            <FactRow
              label={text.facts.installCommand}
              value={providerEntry.installCommand}
              mono
              copyable
            />
          ) : null}
        </View>

        <View style={sheetStyles.actionRow}>
          {showInstallAction && canInstallHere ? (
            <Button
              variant="outline"
              size="sm"
              busy={installing}
              leftIcon={Download}
              onPress={handleInstall}
            >
              {installing ? text.installing : text.install}
            </Button>
          ) : null}
          {providerEntry?.docsUrl ? (
            <Button
              variant="ghost"
              size="sm"
              leftIcon={ExternalLink}
              onPress={() => void openExternalUrl(providerEntry.docsUrl ?? "")}
            >
              {text.docs}
            </Button>
          ) : null}
        </View>
        {showInstallAction && !canInstallHere ? (
          <Text style={sheetStyles.mutedText}>{text.installOnDesktopOnly}</Text>
        ) : null}
      </View>

      {providerEntry?.capabilities ? (
        <View style={sheetStyles.section}>
          <Text style={sheetStyles.sectionTitle}>{text.capabilities}</Text>
          <View style={sheetStyles.capabilityRow}>
            {CAPABILITY_ORDER.map((key) => {
              const supported = providerEntry.capabilities?.[key] ?? false;
              return (
                <View key={key} style={sheetStyles.capabilityChip}>
                  {supported ? (
                    <Check size={theme.iconSize.xs} color={theme.colors.statusSuccess} />
                  ) : (
                    <X size={theme.iconSize.xs} color={theme.colors.foregroundMuted} />
                  )}
                  <Text
                    style={[
                      sheetStyles.capabilityText,
                      !supported && sheetStyles.capabilityTextMuted,
                    ]}
                  >
                    {text.capabilityLabels[key]}
                  </Text>
                </View>
              );
            })}
          </View>
        </View>
      ) : null}

      <View style={sheetStyles.section}>
        <Text style={sheetStyles.sectionTitle}>{text.diagnostic}</Text>
        <View style={sheetStyles.codeBlock}>
          {loading && !diagnostic ? (
            <View style={sheetStyles.codeBlockLoading}>
              <ActivityIndicator size="small" color={theme.colors.foregroundMuted} />
              <Text style={sheetStyles.mutedText}>{text.runningDiagnostic}</Text>
            </View>
          ) : diagnostic ? (
            <ScrollView
              style={sheetStyles.codeScroll}
              contentContainerStyle={sheetStyles.codeContent}
              showsVerticalScrollIndicator={false}
            >
              <ScrollView horizontal showsHorizontalScrollIndicator={false}>
                <Text style={sheetStyles.codeText} selectable>
                  {diagnostic}
                </Text>
              </ScrollView>
            </ScrollView>
          ) : (
            <View style={sheetStyles.codeBlockLoading}>
              <Text style={sheetStyles.mutedText}>{text.noDiagnostic}</Text>
            </View>
          )}
        </View>
      </View>

      <View style={sheetStyles.modelsSection}>
        <View style={sheetStyles.modelsHeader}>
          <Text style={sheetStyles.sectionTitle}>{text.models}</Text>
          <View style={sheetStyles.modelsHeaderMeta}>
            <Text style={sheetStyles.countText}>{models.length}</Text>
            {fetchedAtLabel ? (
              <>
                <Text style={sheetStyles.metaDot}>·</Text>
                <Text style={sheetStyles.countText}>{text.updatedAgo(fetchedAtLabel)}</Text>
              </>
            ) : null}
          </View>
        </View>
        {models.length > 0 ? (
          <View style={sheetStyles.searchContainer}>
            <Search size={theme.iconSize.md} color={theme.colors.foregroundMuted} />
            <AdaptiveTextInput
              value={query}
              onChangeText={setQuery}
              placeholder={text.searchModels}
              placeholderTextColor={theme.colors.foregroundMuted}
              autoCapitalize="none"
              autoCorrect={false}
              // @ts-expect-error - outlineStyle is web-only
              style={[sheetStyles.searchInput, isWeb && { outlineStyle: "none" }]}
            />
          </View>
        ) : null}
        <ScrollView
          style={sheetStyles.modelsScroll}
          contentContainerStyle={sheetStyles.modelsScrollContent}
          keyboardShouldPersistTaps="handled"
          showsVerticalScrollIndicator={false}
        >
          {renderModelsBody()}
        </ScrollView>
      </View>
    </AdaptiveModalSheet>
  );
}

const sheetStyles = StyleSheet.create((theme) => ({
  section: {
    gap: theme.spacing[2],
  },
  factsCard: {
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.lg,
    backgroundColor: theme.colors.surface2,
    paddingHorizontal: theme.spacing[3],
    paddingVertical: theme.spacing[1],
  },
  actionRow: {
    flexDirection: "row",
    alignItems: "center",
    flexWrap: "wrap",
    gap: theme.spacing[2],
  },
  capabilityRow: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: theme.spacing[2],
  },
  capabilityChip: {
    flexDirection: "row",
    alignItems: "center",
    gap: 4,
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.full,
    paddingHorizontal: theme.spacing[2],
    paddingVertical: 3,
  },
  capabilityText: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foreground,
  },
  capabilityTextMuted: {
    color: theme.colors.foregroundMuted,
  },
  sectionTitle: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.normal,
  },
  mutedText: {
    fontSize: theme.fontSize.sm,
    color: theme.colors.foregroundMuted,
  },
  iconButton: {
    width: 30,
    height: 30,
    borderRadius: theme.borderRadius.full,
    alignItems: "center",
    justifyContent: "center",
  },
  iconButtonHovered: {
    backgroundColor: theme.colors.surface2,
  },
  disabled: {
    opacity: 0.5,
  },
  codeBlock: {
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.base,
    backgroundColor: theme.colors.surface2,
    overflow: "hidden",
    maxHeight: 180,
  },
  codeScroll: {
    maxHeight: 180,
  },
  codeContent: {
    paddingVertical: theme.spacing[3],
    paddingHorizontal: theme.spacing[3],
  },
  codeText: {
    fontFamily: Fonts.mono,
    fontSize: theme.fontSize.xs,
    color: theme.colors.foreground,
    lineHeight: 18,
  },
  codeBlockLoading: {
    paddingVertical: theme.spacing[4],
    paddingHorizontal: theme.spacing[3],
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
  },
  modelsSection: {
    flex: 1,
    minHeight: 0,
    gap: theme.spacing[2],
  },
  modelsHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  modelsHeaderMeta: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[1],
  },
  metaDot: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
  },
  countText: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
  },
  searchContainer: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    backgroundColor: theme.colors.surface2,
    borderRadius: theme.borderRadius.md,
    paddingHorizontal: theme.spacing[3],
  },
  searchInput: {
    flex: 1,
    paddingVertical: theme.spacing[2],
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
  },
  modelsScroll: {
    flex: 1,
    minHeight: 0,
  },
  modelsScrollContent: {
    paddingBottom: theme.spacing[2],
  },
  modelRow: {
    paddingVertical: theme.spacing[3],
  },
  modelRowBorder: {
    borderTopWidth: 1,
    borderTopColor: theme.colors.border,
  },
  modelLabel: {
    fontSize: theme.fontSize.sm,
    color: theme.colors.foreground,
  },
  modelId: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
    fontFamily: Fonts.mono,
    marginTop: 2,
  },
  emptyState: {
    paddingVertical: theme.spacing[6],
    alignItems: "center",
    gap: theme.spacing[2],
  },
}));

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Alert, Pressable, Text, TextInput, View } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { ChevronRight, Globe, Monitor, Pencil, RotateCw, Trash2 } from "lucide-react-native";
import type { HostConnection, HostProfile } from "@/types/host-connection";
import {
  getHostRuntimeStore,
  isHostRuntimeConnected,
  useHostRuntimeClient,
  useHostRuntimeIsConnected,
  useHostRuntimeSnapshot,
  useHostMutations,
  useHosts,
} from "@/runtime/host-runtime";
import { useSessionStore } from "@/stores/session-store";
import { getConnectionStatusTone } from "@/utils/daemons";
import { confirmDialog } from "@/utils/confirm-dialog";
import { settingsStyles } from "@/styles/settings";
import { Button } from "@/components/ui/button";
import { SegmentedControl } from "@/components/ui/segmented-control";
import { AdaptiveModalSheet } from "@/components/adaptive-modal-sheet";
import { useDaemonConfig } from "@/hooks/use-daemon-config";
import { useIsLocalDaemon } from "@/hooks/use-is-local-daemon";
import { SettingsSection } from "@/screens/settings/settings-section";
import { ProvidersSection } from "@/screens/settings/providers-section";
import { PairDeviceModal } from "@/desktop/components/pair-device-modal";
import { LocalDaemonSection } from "@/desktop/components/desktop-updates-section";
import { APP_NAME } from "@/config/branding";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";

type SettingsText = ReturnType<typeof getSub2APIMessages>["settings"];
type HostText = SettingsText["host"];

function formatHostConnectionLabel(connection: HostConnection, text: HostText): string {
  if (connection.type === "relay") {
    return text.connectionLabels.relay(connection.relayEndpoint);
  }
  if (connection.type === "directSocket" || connection.type === "directPipe") {
    return text.connectionLabels.local(connection.path);
  }
  return text.connectionLabels.tcp(connection.endpoint);
}

function formatActiveConnectionBadge(
  activeConnection: { type: HostConnection["type"]; display: string } | null,
  theme: ReturnType<typeof useUnistyles>["theme"],
  text: HostText,
): { icon: React.ReactNode; text: string } | null {
  if (!activeConnection) return null;
  if (activeConnection.type === "relay") {
    return {
      icon: <Globe size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />,
      text: text.activeConnections.relay,
    };
  }
  if (activeConnection.type === "directSocket" || activeConnection.type === "directPipe") {
    return {
      icon: <Monitor size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />,
      text: text.activeConnections.local,
    };
  }
  return {
    icon: <Monitor size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />,
    text: activeConnection.display,
  };
}

function formatDaemonVersionBadge(version: string | null): string | null {
  const trimmed = version?.trim();
  if (!trimmed) return null;
  return trimmed.startsWith("v") ? trimmed : `v${trimmed}`;
}

export interface HostPageProps {
  serverId: string;
  onHostRemoved?: () => void;
}

export function HostPage({ serverId, onHostRemoved }: HostPageProps) {
  const daemons = useHosts();
  const host = daemons.find((entry) => entry.serverId === serverId) ?? null;
  const { theme } = useUnistyles();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).settings.host, [locale]);
  const snapshot = useHostRuntimeSnapshot(serverId);
  const isLocalDaemon = useIsLocalDaemon(serverId);

  const daemonVersion = useSessionStore(
    (state) => state.sessions[serverId]?.serverInfo?.version ?? null,
  );

  const connectionStatus = snapshot?.connectionStatus ?? "connecting";
  const activeConnection = snapshot?.activeConnection ?? null;
  const lastError = snapshot?.lastError ?? null;
  const statusLabel = text.statuses[connectionStatus];
  const statusTone = getConnectionStatusTone(connectionStatus);
  const statusColor =
    statusTone === "success"
      ? theme.colors.palette.green[400]
      : statusTone === "warning"
        ? theme.colors.palette.amber[500]
        : statusTone === "error"
          ? theme.colors.destructive
          : theme.colors.foregroundMuted;
  const statusPillBg =
    statusTone === "success"
      ? "rgba(74, 222, 128, 0.1)"
      : statusTone === "warning"
        ? "rgba(245, 158, 11, 0.1)"
        : statusTone === "error"
          ? "rgba(248, 113, 113, 0.1)"
          : "rgba(161, 161, 170, 0.1)";
  const connectionBadge = formatActiveConnectionBadge(activeConnection, theme, text);
  const versionBadgeText = formatDaemonVersionBadge(daemonVersion);
  const connectionError =
    typeof lastError === "string" && lastError.trim().length > 0 ? lastError.trim() : null;

  if (!host) {
    return (
      <View testID={`settings-host-page-${serverId}`}>
        <View style={[settingsStyles.card, styles.emptyCard]}>
          <Text style={styles.emptyText}>{text.notFound}</Text>
        </View>
      </View>
    );
  }

  return (
    <View testID={`settings-host-page-${serverId}`}>
      <View style={styles.identityBadges} testID="host-page-identity">
        <View style={[styles.statusPill, { backgroundColor: statusPillBg }]}>
          <View style={[styles.statusDot, { backgroundColor: statusColor }]} />
          <Text style={[styles.statusText, { color: statusColor }]}>{statusLabel}</Text>
        </View>
        {connectionBadge ? (
          <View style={styles.badgePill}>
            {connectionBadge.icon}
            <Text style={styles.badgeText} numberOfLines={1}>
              {connectionBadge.text}
            </Text>
          </View>
        ) : null}
        {versionBadgeText ? (
          <View style={styles.badgePill}>
            <Text style={styles.badgeText} numberOfLines={1}>
              {versionBadgeText}
            </Text>
          </View>
        ) : null}
      </View>
      {connectionError ? <Text style={styles.errorText}>{connectionError}</Text> : null}

      <ConnectionsSection host={host} />

      <DaemonSection host={host} isLocalDaemon={isLocalDaemon} />

      <ProvidersSection serverId={serverId} />

      <RemoveHostSection host={host} onRemoved={onHostRemoved} />
    </View>
  );
}

export function HostRenameButton({ host }: { host: HostProfile }) {
  const { theme } = useUnistyles();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).settings.host, [locale]);
  const { renameHost } = useHostMutations();
  const [isEditing, setIsEditing] = useState(false);
  const [draftLabel, setDraftLabel] = useState(host.label ?? "");
  const [isSaving, setIsSaving] = useState(false);
  const inputRef = useRef<TextInput>(null);

  useEffect(() => {
    setDraftLabel(host.label ?? "");
  }, [host.serverId, host.label]);

  useEffect(() => {
    if (isEditing) {
      const timeout = setTimeout(() => inputRef.current?.focus(), 50);
      return () => clearTimeout(timeout);
    }
    return undefined;
  }, [isEditing]);

  const handleSave = useCallback(async () => {
    const nextLabel = draftLabel.trim();
    if (!nextLabel) {
      Alert.alert(text.labelRequiredTitle, text.labelRequiredMessage);
      return;
    }
    if (isSaving) return;
    if (nextLabel === host.label.trim()) {
      setIsEditing(false);
      return;
    }
    try {
      setIsSaving(true);
      await renameHost(host.serverId, nextLabel);
      setIsEditing(false);
    } catch (error) {
      console.error("[HostPage] Failed to rename host", error);
      Alert.alert(text.error, text.unableSaveHost);
    } finally {
      setIsSaving(false);
    }
  }, [draftLabel, host.label, host.serverId, isSaving, renameHost]);

  const handleCancel = useCallback(() => {
    if (isSaving) return;
    setDraftLabel(host.label ?? "");
    setIsEditing(false);
  }, [host.label, isSaving]);

  return (
    <>
      <Pressable
        onPress={() => {
          setDraftLabel(host.label ?? "");
          setIsEditing(true);
        }}
        hitSlop={8}
        style={styles.identityEditButton}
        accessibilityRole="button"
        accessibilityLabel={text.editLabel}
        testID="host-page-label-edit-button"
      >
        <Pencil size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
      </Pressable>

      <AdaptiveModalSheet
        visible={isEditing}
        onClose={handleCancel}
        title={text.renameTitle}
        testID="host-page-rename-modal"
      >
        <View style={styles.renameBody}>
          <TextInput
            ref={inputRef}
            value={draftLabel}
            onChangeText={setDraftLabel}
            placeholder={text.labelPlaceholder}
            placeholderTextColor={theme.colors.foregroundMuted}
            autoCapitalize="none"
            autoCorrect={false}
            editable={!isSaving}
            onSubmitEditing={() => void handleSave()}
            style={styles.renameInput}
            testID="host-page-label-input"
          />
          <View style={styles.renameActions}>
            <Button
              variant="secondary"
              size="sm"
              style={{ flex: 1 }}
              onPress={handleCancel}
              disabled={isSaving}
            >
              {text.cancel}
            </Button>
            <Button
              size="sm"
              style={{ flex: 1 }}
              onPress={() => void handleSave()}
              disabled={isSaving}
              testID="host-page-label-save"
            >
              {isSaving ? text.saving : text.save}
            </Button>
          </View>
        </View>
      </AdaptiveModalSheet>
    </>
  );
}

function ConnectionsSection({ host }: { host: HostProfile }) {
  const locale = useSub2APILocale();
  const messages = useMemo(() => getSub2APIMessages(locale).settings, [locale]);
  const hostText = messages.host;
  const text = messages.connections;
  const { removeConnection } = useHostMutations();
  const snapshot = useHostRuntimeSnapshot(host.serverId);
  const probeByConnectionId = snapshot?.probeByConnectionId ?? new Map();
  const [pendingRemoveConnection, setPendingRemoveConnection] = useState<{
    connectionId: string;
    title: string;
  } | null>(null);
  const [isRemovingConnection, setIsRemovingConnection] = useState(false);

  return (
    <SettingsSection title={text.title}>
      <View style={settingsStyles.card} testID="host-page-connections-card">
        {host.connections.map((conn, index) => {
          const probe = probeByConnectionId.get(conn.id);
          return (
            <ConnectionRow
              key={conn.id}
              connection={conn}
              showBorder={index > 0}
              latencyMs={probe?.status === "available" ? probe.latencyMs : undefined}
              latencyLoading={!probe || probe.status === "pending"}
              latencyError={probe?.status === "unavailable"}
              onRemove={() => {
                setPendingRemoveConnection({
                  connectionId: conn.id,
                  title: formatHostConnectionLabel(conn, hostText),
                });
              }}
            />
          );
        })}
      </View>

      {pendingRemoveConnection ? (
        <AdaptiveModalSheet
          title={text.removeConnection}
          visible
          onClose={() => {
            if (isRemovingConnection) return;
            setPendingRemoveConnection(null);
          }}
          testID="remove-connection-confirm-modal"
        >
          <Text style={styles.confirmText}>
            {text.removeConnectionMessage(pendingRemoveConnection.title)}
          </Text>
          <View style={styles.confirmActions}>
            <Button
              variant="secondary"
              size="sm"
              style={{ flex: 1 }}
              onPress={() => setPendingRemoveConnection(null)}
              disabled={isRemovingConnection}
            >
              {hostText.cancel}
            </Button>
            <Button
              variant="destructive"
              size="sm"
              style={{ flex: 1 }}
              onPress={() => {
                const { connectionId } = pendingRemoveConnection;
                setIsRemovingConnection(true);
                void removeConnection(host.serverId, connectionId)
                  .then(() => setPendingRemoveConnection(null))
                  .catch((error) => {
                    console.error("[HostPage] Failed to remove connection", error);
                    Alert.alert(hostText.error, text.unableRemoveConnection);
                  })
                  .finally(() => setIsRemovingConnection(false));
              }}
              disabled={isRemovingConnection}
              testID="remove-connection-confirm"
            >
              {hostText.remove}
            </Button>
          </View>
        </AdaptiveModalSheet>
      ) : null}
    </SettingsSection>
  );
}

function ConnectionRow({
  connection,
  showBorder,
  latencyMs,
  latencyLoading,
  latencyError,
  onRemove,
}: {
  connection: HostConnection;
  showBorder: boolean;
  latencyMs: number | null | undefined;
  latencyLoading: boolean;
  latencyError: boolean;
  onRemove: () => void;
}) {
  const { theme } = useUnistyles();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).settings.host, [locale]);
  const title = formatHostConnectionLabel(connection, text);

  const latencyText = (() => {
    if (latencyLoading) return "...";
    if (latencyError) return text.timeout;
    if (latencyMs != null) return `${latencyMs}ms`;
    return "\u2014";
  })();
  const latencyColor = latencyError ? theme.colors.palette.red[300] : theme.colors.foregroundMuted;

  return (
    <View style={[settingsStyles.row, showBorder && settingsStyles.rowBorder]}>
      <View style={settingsStyles.rowContent}>
        <Text style={settingsStyles.rowTitle} numberOfLines={1}>
          {title}
        </Text>
      </View>
      <Text style={[styles.connectionLatency, { color: latencyColor }]}>{latencyText}</Text>
      <Button
        variant="ghost"
        size="sm"
        textStyle={{ color: theme.colors.destructive }}
        onPress={onRemove}
      >
        {text.remove}
      </Button>
    </View>
  );
}

function DaemonSection({ host, isLocalDaemon }: { host: HostProfile; isLocalDaemon: boolean }) {
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).settings.host, [locale]);
  return (
    <>
      <SettingsSection title={text.sections.operations}>
        <RestartDaemonCard host={host} />
        <InjectPaseoToolsCard serverId={host.serverId} />
      </SettingsSection>
      {isLocalDaemon ? (
        <SettingsSection title={text.sections.pairDevices}>
          <PairDeviceRow />
        </SettingsSection>
      ) : null}
      {isLocalDaemon ? <LocalDaemonSection /> : null}
    </>
  );
}

const delay = (ms: number) =>
  new Promise<void>((resolve) => {
    const timeout = setTimeout(() => {
      clearTimeout(timeout);
      resolve();
    }, ms);
  });

function RestartDaemonCard({ host }: { host: HostProfile }) {
  const { theme } = useUnistyles();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).settings.host, [locale]);
  const daemonClient = useHostRuntimeClient(host.serverId);
  const isConnected = useHostRuntimeIsConnected(host.serverId);
  const runtime = getHostRuntimeStore();
  const [isRestarting, setIsRestarting] = useState(false);
  const isMountedRef = useRef(true);

  useEffect(() => {
    return () => {
      isMountedRef.current = false;
    };
  }, []);

  const isHostConnected = useCallback(
    () => isHostRuntimeConnected(runtime.getSnapshot(host.serverId)),
    [host.serverId, runtime],
  );

  const waitForCondition = useCallback(
    async (predicate: () => boolean, timeoutMs: number, intervalMs = 250) => {
      const deadline = Date.now() + timeoutMs;
      while (Date.now() < deadline) {
        if (!isMountedRef.current) return false;
        if (predicate()) return true;
        await delay(intervalMs);
      }
      return predicate();
    },
    [],
  );

  const waitForDaemonRestart = useCallback(async () => {
    const disconnectTimeoutMs = 7000;
    const reconnectTimeoutMs = 30000;
    if (isHostConnected()) {
      await waitForCondition(() => !isHostConnected(), disconnectTimeoutMs);
    }
    const reconnected = await waitForCondition(() => isHostConnected(), reconnectTimeoutMs);
    if (isMountedRef.current) {
      setIsRestarting(false);
      if (!reconnected) {
        Alert.alert(text.unableReconnectTitle, text.unableReconnectMessage(host.label));
      }
    }
  }, [host.label, isHostConnected, text, waitForCondition]);

  const handleRestart = useCallback(() => {
    if (!daemonClient) {
      Alert.alert(text.hostUnavailableTitle, text.hostUnavailableMessage);
      return;
    }
    if (!isHostConnected()) {
      Alert.alert(text.hostOfflineTitle, text.hostOfflineMessage(APP_NAME));
      return;
    }

    void confirmDialog({
      title: text.restartConfirmTitle(host.label),
      message: text.restartConfirmationMessage,
      confirmLabel: text.restart,
      cancelLabel: text.cancel,
      destructive: true,
    })
      .then((confirmed) => {
        if (!confirmed) return;
        setIsRestarting(true);
        void daemonClient
          .restartServer(`settings_daemon_restart_${host.serverId}`)
          .catch((error) => {
            console.error(`[HostPage] Failed to restart daemon ${host.label}`, error);
            if (!isMountedRef.current) return;
            setIsRestarting(false);
            Alert.alert(text.error, text.failedRestartRequest(APP_NAME));
          });
        void waitForDaemonRestart();
      })
      .catch((error) => {
        console.error(`[HostPage] Failed to open restart confirmation for ${host.label}`, error);
        Alert.alert(text.error, text.unableOpenRestartConfirmation);
      });
  }, [daemonClient, host.label, host.serverId, isHostConnected, text, waitForDaemonRestart]);

  return (
    <View style={settingsStyles.card} testID="host-page-restart-card">
      <View style={settingsStyles.row}>
        <View style={settingsStyles.rowContent}>
          <Text style={settingsStyles.rowTitle}>{text.restartDaemon}</Text>
          <Text style={settingsStyles.rowHint}>{text.restartDaemonHint}</Text>
        </View>
        <Button
          variant="outline"
          size="sm"
          leftIcon={<RotateCw size={theme.iconSize.sm} color={theme.colors.foreground} />}
          onPress={handleRestart}
          disabled={isRestarting || !daemonClient || !isConnected}
          testID="host-page-restart-button"
        >
          {isRestarting ? text.restarting : text.restart}
        </Button>
      </View>
    </View>
  );
}

function InjectPaseoToolsCard({ serverId }: { serverId: string }) {
  const isConnected = useHostRuntimeIsConnected(serverId);
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).settings.host, [locale]);
  const { config, patchConfig } = useDaemonConfig(serverId);

  if (!isConnected) return null;

  return (
    <View style={settingsStyles.card} testID="host-page-inject-mcp-card">
      <View style={settingsStyles.row}>
        <View style={settingsStyles.rowContent}>
          <Text style={settingsStyles.rowTitle}>{text.injectToolsTitle(APP_NAME)}</Text>
          <Text style={settingsStyles.rowHint}>{text.injectToolsHint(APP_NAME)}</Text>
        </View>
        <SegmentedControl
          size="sm"
          value={config?.mcp.injectIntoAgents === false ? "off" : "on"}
          onValueChange={(value) => {
            void patchConfig({
              mcp: {
                injectIntoAgents: value === "on",
              },
            });
          }}
          options={[
            { value: "on", label: text.on },
            { value: "off", label: text.off },
          ]}
        />
      </View>
    </View>
  );
}

function PairDeviceRow() {
  const { theme } = useUnistyles();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).settings.host, [locale]);
  const [isModalOpen, setIsModalOpen] = useState(false);

  return (
    <View style={settingsStyles.card}>
      <Pressable
        style={settingsStyles.row}
        onPress={() => setIsModalOpen(true)}
        accessibilityRole="button"
        testID="host-page-pair-device-row"
      >
        <View style={settingsStyles.rowContent}>
          <Text style={settingsStyles.rowTitle}>{text.pairDevice}</Text>
          <Text style={settingsStyles.rowHint}>{text.pairDeviceHint}</Text>
        </View>
        <ChevronRight size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
      </Pressable>

      <PairDeviceModal
        visible={isModalOpen}
        onClose={() => setIsModalOpen(false)}
        testID="host-page-pair-device-card"
      />
    </View>
  );
}

function RemoveHostSection({ host, onRemoved }: { host: HostProfile; onRemoved?: () => void }) {
  const { theme } = useUnistyles();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).settings.host, [locale]);
  const { removeHost } = useHostMutations();
  const [isConfirming, setIsConfirming] = useState(false);
  const [isRemoving, setIsRemoving] = useState(false);

  return (
    <SettingsSection title={text.sections.dangerZone} testID="host-page-remove-host-card">
      <View style={settingsStyles.card}>
        <View style={settingsStyles.row}>
          <View style={settingsStyles.rowContent}>
            <Text style={settingsStyles.rowTitle}>{text.removeHost}</Text>
            <Text style={settingsStyles.rowHint}>{text.removeHostHint}</Text>
          </View>
          <Button
            variant="outline"
            size="sm"
            leftIcon={<Trash2 size={theme.iconSize.sm} color={theme.colors.destructive} />}
            textStyle={{ color: theme.colors.destructive }}
            onPress={() => setIsConfirming(true)}
            testID="host-page-remove-host-button"
          >
            {text.remove}
          </Button>
        </View>
      </View>

      {isConfirming ? (
        <AdaptiveModalSheet
          title={text.removeHost}
          visible
          onClose={() => {
            if (isRemoving) return;
            setIsConfirming(false);
          }}
          testID="remove-host-confirm-modal"
        >
          <Text style={styles.confirmText}>{text.removeHostConfirmMessage(host.label)}</Text>
          <View style={styles.confirmActions}>
            <Button
              variant="secondary"
              size="sm"
              style={{ flex: 1 }}
              onPress={() => setIsConfirming(false)}
              disabled={isRemoving}
            >
              {text.cancel}
            </Button>
            <Button
              variant="destructive"
              size="sm"
              style={{ flex: 1 }}
              onPress={() => {
                setIsRemoving(true);
                void removeHost(host.serverId)
                  .then(() => {
                    setIsConfirming(false);
                    onRemoved?.();
                  })
                  .catch((error) => {
                    console.error("[HostPage] Failed to remove host", error);
                    Alert.alert(text.error, text.unableRemoveHost);
                  })
                  .finally(() => setIsRemoving(false));
              }}
              disabled={isRemoving}
              testID="remove-host-confirm"
            >
              {text.remove}
            </Button>
          </View>
        </AdaptiveModalSheet>
      ) : null}
    </SettingsSection>
  );
}

const styles = StyleSheet.create((theme) => ({
  identityEditButton: {
    padding: theme.spacing[1],
    borderRadius: theme.borderRadius.md,
  },
  identityBadges: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[1],
    flexWrap: "wrap",
    marginBottom: theme.spacing[6],
  },
  statusPill: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    paddingHorizontal: theme.spacing[2],
    paddingVertical: 4,
    borderRadius: theme.borderRadius.full,
  },
  statusDot: {
    width: 6,
    height: 6,
    borderRadius: theme.borderRadius.full,
  },
  statusText: {
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.normal,
  },
  badgePill: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    paddingHorizontal: theme.spacing[2],
    paddingVertical: 4,
    borderRadius: theme.borderRadius.full,
    borderWidth: 1,
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.surface3,
    maxWidth: 200,
  },
  badgeText: {
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.normal,
    color: theme.colors.foregroundMuted,
    flexShrink: 1,
  },
  errorText: {
    color: theme.colors.palette.red[300],
    fontSize: theme.fontSize.xs,
    marginBottom: theme.spacing[2],
  },
  connectionLatency: {
    fontSize: theme.fontSize.sm,
    marginRight: theme.spacing[2],
  },
  confirmText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
  },
  confirmActions: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    marginTop: theme.spacing[4],
  },
  emptyCard: {
    padding: theme.spacing[4],
    alignItems: "center",
  },
  emptyText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
  },
  renameBody: {
    gap: theme.spacing[3],
    paddingBottom: theme.spacing[2],
  },
  renameInput: {
    backgroundColor: theme.colors.surface0,
    color: theme.colors.foreground,
    paddingVertical: theme.spacing[3],
    paddingHorizontal: theme.spacing[3],
    borderRadius: theme.borderRadius.md,
    borderWidth: 1,
    borderColor: theme.colors.border,
    fontSize: theme.fontSize.base,
  },
  renameActions: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
  },
}));

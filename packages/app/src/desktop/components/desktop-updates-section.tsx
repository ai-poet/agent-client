import { useCallback, useMemo, useState } from "react";
import { ActivityIndicator, Alert, Text, View } from "react-native";
import * as Clipboard from "expo-clipboard";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { settingsStyles } from "@/styles/settings";
import { SettingsSection } from "@/screens/settings/settings-section";
import { ArrowUpRight, Play, Pause, RotateCw, Copy, FileText, Activity } from "lucide-react-native";
import { AdaptiveModalSheet } from "@/components/adaptive-modal-sheet";
import { Button } from "@/components/ui/button";
import { useAppSettings } from "@/hooks/use-settings";
import { confirmDialog } from "@/utils/confirm-dialog";
import { openExternalUrl } from "@/utils/open-external-url";
import { isVersionMismatch } from "@/desktop/updates/desktop-updates";
import {
  getCliDaemonStatus,
  restartDesktopDaemon,
  shouldUseDesktopDaemon,
  startDesktopDaemon,
  stopDesktopDaemon,
} from "@/desktop/daemon/desktop-daemon";
import { useDaemonStatus } from "@/desktop/hooks/use-daemon-status";
import { resolveAppVersion } from "@/utils/app-version";
import { APP_NAME } from "@/config/branding";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";

export function LocalDaemonSection() {
  const { theme } = useUnistyles();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).settings.daemon, [locale]);
  const showSection = shouldUseDesktopDaemon();
  const appVersion = resolveAppVersion();
  const { settings, updateSettings } = useAppSettings();
  const { data, isLoading, error: statusError, setStatus, refetch } = useDaemonStatus();
  const [isRestartingDaemon, setIsRestartingDaemon] = useState(false);
  const [isUpdatingDaemonManagement, setIsUpdatingDaemonManagement] = useState(false);
  const [statusMessage, setStatusMessage] = useState<string | null>(null);
  const [isLogsModalOpen, setIsLogsModalOpen] = useState(false);
  const [cliStatusOutput, setCliStatusOutput] = useState<string | null>(null);
  const [isCliStatusModalOpen, setIsCliStatusModalOpen] = useState(false);
  const [isLoadingCliStatus, setIsLoadingCliStatus] = useState(false);

  const daemonStatus = data?.status ?? null;
  const daemonLogs = data?.logs ?? null;
  const daemonVersion = daemonStatus?.version ?? null;

  const daemonVersionMismatch = isVersionMismatch(appVersion, daemonVersion);
  const daemonStatusStateText =
    statusError ?? (daemonStatus?.status === "running" ? daemonStatus.status : text.notRunning);
  const daemonStatusDetailText = text.pid(daemonStatus?.pid);
  const isDaemonManagementPaused = !settings.manageBuiltInDaemon;
  const daemonActionLabel =
    daemonStatus?.status === "running" ? text.restartDaemon : text.startDaemon;
  const daemonActionMessage =
    daemonStatus?.status === "running" ? text.restartDaemonHint : text.startDaemonHint;

  const handleUpdateLocalDaemon = useCallback(() => {
    if (!showSection || isRestartingDaemon) {
      return;
    }

    void confirmDialog({
      title: daemonActionLabel,
      message:
        daemonStatus?.status === "running" ? text.restartConfirmMessage : text.startConfirmMessage,
      confirmLabel: daemonActionLabel,
      cancelLabel: text.cancel,
    })
      .then((confirmed) => {
        if (!confirmed) {
          return;
        }

        setIsRestartingDaemon(true);
        setStatusMessage(null);

        const action =
          daemonStatus?.status === "running" ? restartDesktopDaemon : startDesktopDaemon;

        void action()
          .then((newStatus) => {
            setStatus(newStatus);
            setStatusMessage(
              daemonStatus?.status === "running" ? text.daemonRestarted : text.daemonStarted,
            );
            refetch();
          })
          .catch((error) => {
            console.error("[Settings] Failed to change desktop daemon state", error);
            const message = error instanceof Error ? error.message : String(error);
            setStatusMessage(text.actionFailed(daemonActionLabel, message));
          })
          .finally(() => {
            setIsRestartingDaemon(false);
          });
      })
      .catch((error) => {
        console.error("[Settings] Failed to open desktop daemon action confirmation", error);
        Alert.alert(text.error, text.unableOpenDaemonConfirmation);
      });
  }, [
    daemonActionLabel,
    daemonStatus?.status,
    isRestartingDaemon,
    refetch,
    setStatus,
    showSection,
    text,
  ]);

  const handleToggleDaemonManagement = useCallback(() => {
    if (isUpdatingDaemonManagement) {
      return;
    }

    if (!settings.manageBuiltInDaemon) {
      setIsUpdatingDaemonManagement(true);
      setStatusMessage(null);
      void updateSettings({ manageBuiltInDaemon: true })
        .then(() => {
          setStatusMessage(text.managementResumed);
        })
        .catch((error) => {
          console.error("[Settings] Failed to update built-in daemon management", error);
          Alert.alert(text.error, text.unableUpdateManagement);
        })
        .finally(() => {
          setIsUpdatingDaemonManagement(false);
        });
      return;
    }

    void confirmDialog({
      title: text.pauseBuiltInDaemon,
      message: text.pauseBuiltInDaemonMessage,
      confirmLabel: text.pauseAndStop,
      cancelLabel: text.cancel,
      destructive: true,
    })
      .then((confirmed) => {
        if (!confirmed) {
          return;
        }

        setIsUpdatingDaemonManagement(true);
        setStatusMessage(null);

        const stopPromise =
          daemonStatus?.status === "running"
            ? stopDesktopDaemon()
            : Promise.resolve(daemonStatus ?? null);

        void stopPromise
          .then((newStatus) => {
            if (newStatus) {
              setStatus(newStatus);
            }
            return updateSettings({ manageBuiltInDaemon: false });
          })
          .then(() => {
            refetch();
            setStatusMessage(text.managementPausedAndStopped);
          })
          .catch((error) => {
            console.error("[Settings] Failed to pause built-in daemon management", error);
            Alert.alert(text.error, text.unablePauseManagement);
          })
          .finally(() => {
            setIsUpdatingDaemonManagement(false);
          });
      })
      .catch((error) => {
        console.error("[Settings] Failed to open built-in daemon pause confirmation", error);
        Alert.alert(text.error, text.unableOpenDaemonConfirmation);
      });
  }, [
    daemonStatus,
    isUpdatingDaemonManagement,
    refetch,
    setStatus,
    settings.manageBuiltInDaemon,
    text,
    updateSettings,
  ]);

  const handleCopyLogPath = useCallback(() => {
    const logPath = daemonLogs?.logPath;
    if (!logPath) {
      return;
    }

    void Clipboard.setStringAsync(logPath)
      .then(() => {
        Alert.alert(text.copied, text.logPathCopied);
      })
      .catch((error) => {
        console.error("[Settings] Failed to copy log path", error);
        Alert.alert(text.error, text.unableCopyLogPath);
      });
  }, [daemonLogs?.logPath, text]);

  const handleOpenLogs = useCallback(() => {
    if (!daemonLogs) {
      return;
    }
    setIsLogsModalOpen(true);
  }, [daemonLogs]);

  const handleOpenCliStatus = useCallback(async () => {
    setIsLoadingCliStatus(true);
    try {
      setCliStatusOutput(await getCliDaemonStatus());
      setIsCliStatusModalOpen(true);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setCliStatusOutput(text.failedFetchStatus(message));
      setIsCliStatusModalOpen(true);
    } finally {
      setIsLoadingCliStatus(false);
    }
  }, []);

  const handleCopyCliStatus = useCallback(() => {
    if (!cliStatusOutput) {
      return;
    }
    void Clipboard.setStringAsync(cliStatusOutput)
      .then(() => {
        Alert.alert(text.copied, text.statusCopied);
      })
      .catch((error) => {
        console.error("[Settings] Failed to copy daemon status", error);
      });
  }, [cliStatusOutput, text]);

  if (!showSection) {
    return null;
  }

  const advancedSettingsButton = (
    <Button
      variant="ghost"
      size="sm"
      leftIcon={<ArrowUpRight size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />}
      textStyle={settingsStyles.sectionHeaderLinkText}
      style={settingsStyles.sectionHeaderLink}
      onPress={() => void openExternalUrl(ADVANCED_DAEMON_SETTINGS_URL)}
      accessibilityLabel={text.openAdvancedSettings}
    >
      {text.advancedSettings}
    </Button>
  );

  return (
    <SettingsSection
      title={text.title}
      trailing={advancedSettingsButton}
      testID="host-page-daemon-lifecycle-card"
    >
      {isLoading ? (
        <View style={[settingsStyles.card, styles.loadingCard]}>
          <ActivityIndicator size="small" color={theme.colors.foregroundMuted} />
        </View>
      ) : (
        <>
          <View style={settingsStyles.card}>
            <View style={settingsStyles.row}>
              <View style={settingsStyles.rowContent}>
                <Text style={settingsStyles.rowTitle}>{text.status}</Text>
                <Text style={settingsStyles.rowHint}>{text.builtInOnly}</Text>
              </View>
              <View style={styles.statusValueGroup}>
                <Text style={styles.valueText}>{daemonStatusStateText}</Text>
                <Text style={styles.valueSubtext}>{daemonStatusDetailText}</Text>
              </View>
            </View>
            <View style={[settingsStyles.row, settingsStyles.rowBorder]}>
              <View style={settingsStyles.rowContent}>
                <Text style={settingsStyles.rowTitle}>{text.daemonManagement}</Text>
                <Text style={settingsStyles.rowHint}>
                  {isDaemonManagementPaused
                    ? text.managementPausedHint
                    : text.managementEnabledHint(APP_NAME)}
                </Text>
              </View>
              <Button
                variant="outline"
                size="sm"
                leftIcon={
                  isDaemonManagementPaused ? (
                    <Play size={theme.iconSize.sm} color={theme.colors.foreground} />
                  ) : (
                    <Pause size={theme.iconSize.sm} color={theme.colors.foreground} />
                  )
                }
                onPress={handleToggleDaemonManagement}
                disabled={isUpdatingDaemonManagement}
              >
                {isUpdatingDaemonManagement
                  ? isDaemonManagementPaused
                    ? text.resuming
                    : text.pausing
                  : isDaemonManagementPaused
                    ? text.resume
                    : text.pause}
              </Button>
            </View>
            <View style={[settingsStyles.row, settingsStyles.rowBorder]}>
              <View style={settingsStyles.rowContent}>
                <Text style={settingsStyles.rowTitle}>{daemonActionLabel}</Text>
                <Text style={settingsStyles.rowHint}>{daemonActionMessage}</Text>
                {statusMessage ? <Text style={styles.statusText}>{statusMessage}</Text> : null}
              </View>
              <Button
                variant="outline"
                size="sm"
                leftIcon={<RotateCw size={theme.iconSize.sm} color={theme.colors.foreground} />}
                onPress={handleUpdateLocalDaemon}
                disabled={isRestartingDaemon}
              >
                {isRestartingDaemon
                  ? daemonStatus?.status === "running"
                    ? text.restartDaemon
                    : text.startDaemon
                  : daemonActionLabel}
              </Button>
            </View>
            <View style={[settingsStyles.row, settingsStyles.rowBorder]}>
              <View style={settingsStyles.rowContent}>
                <Text style={settingsStyles.rowTitle}>{text.logFile}</Text>
                <Text style={settingsStyles.rowHint}>
                  {daemonLogs?.logPath ?? text.logPathUnavailable}
                </Text>
              </View>
              <View style={styles.actionGroup}>
                {daemonLogs?.logPath ? (
                  <Button
                    variant="outline"
                    size="sm"
                    leftIcon={<Copy size={theme.iconSize.sm} color={theme.colors.foreground} />}
                    onPress={handleCopyLogPath}
                  >
                    {text.copyPath}
                  </Button>
                ) : null}
                <Button
                  variant="outline"
                  size="sm"
                  leftIcon={<FileText size={theme.iconSize.sm} color={theme.colors.foreground} />}
                  onPress={handleOpenLogs}
                  disabled={!daemonLogs}
                >
                  {text.openLogs}
                </Button>
              </View>
            </View>
            <View style={[settingsStyles.row, settingsStyles.rowBorder]}>
              <View style={settingsStyles.rowContent}>
                <Text style={settingsStyles.rowTitle}>{text.fullStatus}</Text>
                <Text style={settingsStyles.rowHint}>{text.fullStatusHint}</Text>
              </View>
              <Button
                variant="outline"
                size="sm"
                leftIcon={<Activity size={theme.iconSize.sm} color={theme.colors.foreground} />}
                onPress={() => void handleOpenCliStatus()}
                disabled={isLoadingCliStatus}
              >
                {isLoadingCliStatus ? text.loading : text.viewStatus}
              </Button>
            </View>
          </View>

          {daemonVersionMismatch ? (
            <View style={styles.warningCard}>
              <Text style={styles.warningText}>{text.versionMismatch}</Text>
            </View>
          ) : null}
        </>
      )}

      <AdaptiveModalSheet
        visible={isLogsModalOpen}
        onClose={() => setIsLogsModalOpen(false)}
        title={text.daemonLogs}
        testID="managed-daemon-logs-dialog"
        snapPoints={["70%", "92%"]}
      >
        <View style={styles.modalBody}>
          <Text style={settingsStyles.rowHint}>
            {daemonLogs?.logPath ?? text.logPathUnavailable}
          </Text>
          <Text style={styles.logOutput} selectable>
            {daemonLogs?.contents.length ? daemonLogs.contents : text.logFileEmpty}
          </Text>
        </View>
      </AdaptiveModalSheet>

      <AdaptiveModalSheet
        visible={isCliStatusModalOpen}
        onClose={() => setIsCliStatusModalOpen(false)}
        title={text.daemonStatus}
        testID="daemon-cli-status-dialog"
        snapPoints={["60%", "85%"]}
      >
        <View style={styles.modalBody}>
          <Text style={styles.logOutput} selectable>
            {cliStatusOutput ?? ""}
          </Text>
          <View style={styles.modalActions}>
            <Button variant="outline" size="sm" onPress={() => setIsCliStatusModalOpen(false)}>
              {text.close}
            </Button>
            <Button size="sm" onPress={handleCopyCliStatus}>
              {text.copy}
            </Button>
          </View>
        </View>
      </AdaptiveModalSheet>
    </SettingsSection>
  );
}

const ADVANCED_DAEMON_SETTINGS_URL = "https://paseo.sh/docs/configuration";

const styles = StyleSheet.create((theme) => ({
  actionGroup: {
    flexDirection: "row",
    gap: theme.spacing[2],
    flexWrap: "wrap",
    justifyContent: "flex-end",
  },
  loadingCard: {
    alignItems: "center",
    justifyContent: "center",
    paddingVertical: theme.spacing[6],
  },
  statusValueGroup: {
    alignItems: "flex-end",
    gap: 2,
  },
  valueText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
  },
  valueSubtext: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  statusText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    marginTop: theme.spacing[1],
  },
  warningCard: {
    marginTop: theme.spacing[3],
    borderRadius: theme.borderRadius.lg,
    borderWidth: 1,
    borderColor: theme.colors.palette.amber[500],
    backgroundColor: "rgba(245, 158, 11, 0.12)",
    paddingVertical: theme.spacing[2],
    paddingHorizontal: theme.spacing[3],
  },
  warningText: {
    color: theme.colors.palette.amber[500],
    fontSize: theme.fontSize.xs,
  },
  modalBody: {
    gap: theme.spacing[3],
    paddingBottom: theme.spacing[2],
  },
  logOutput: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace",
    lineHeight: 18,
  },
  modalActions: {
    flexDirection: "row",
    justifyContent: "flex-end",
    gap: theme.spacing[2],
  },
}));

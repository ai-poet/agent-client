import { useCallback, useMemo, useState } from "react";
import { ActivityIndicator, Alert, Text, View } from "react-native";
import { useFocusEffect } from "@react-navigation/native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { ArrowUpRight, Terminal, Blocks, Check, Cpu, Wrench } from "lucide-react-native";
import { settingsStyles } from "@/styles/settings";
import { SettingsSection } from "@/screens/settings/settings-section";
import { Button } from "@/components/ui/button";
import { openExternalUrl } from "@/utils/open-external-url";
import {
  shouldUseDesktopDaemon,
  getCliInstallStatus,
  installCli,
  getSkillsInstallStatus,
  installSkills,
  type InstallStatus,
  getModelCliRuntimeStatus,
  installAllModelClis,
  installClaudeCodeCli,
  installCodexCli,
  installNode22Runtime,
  type ModelCliRuntimeStatus,
} from "@/desktop/daemon/desktop-daemon";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";

const CLI_DOCS_URL = "https://paseo.sh/docs/cli";
const SKILLS_DOCS_URL = "https://paseo.sh/docs/skills";

export function IntegrationsSection() {
  const { theme } = useUnistyles();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).settings.integrations, [locale]);
  const showSection = shouldUseDesktopDaemon();

  const [cliStatus, setCliStatus] = useState<InstallStatus | null>(null);
  const [skillsStatus, setSkillsStatus] = useState<InstallStatus | null>(null);
  const [modelCliStatus, setModelCliStatus] = useState<ModelCliRuntimeStatus | null>(null);
  const [isInstallingCli, setIsInstallingCli] = useState(false);
  const [isInstallingSkills, setIsInstallingSkills] = useState(false);
  const [isInstallingNodeRuntime, setIsInstallingNodeRuntime] = useState(false);
  const [isInstallingCodex, setIsInstallingCodex] = useState(false);
  const [isInstallingClaudeCode, setIsInstallingClaudeCode] = useState(false);
  const [isInstallingAll, setIsInstallingAll] = useState(false);
  const [integrationCheckPending, setIntegrationCheckPending] = useState(true);
  const [modelRuntimeUnavailable, setModelRuntimeUnavailable] = useState(false);

  const loadStatus = useCallback(() => {
    if (!showSection) return;
    setIntegrationCheckPending(true);
    setModelRuntimeUnavailable(false);
    let remaining = 3;
    const markDone = () => {
      remaining -= 1;
      if (remaining === 0) {
        setIntegrationCheckPending(false);
      }
    };

    void getCliInstallStatus()
      .then(setCliStatus)
      .catch((error) => {
        console.error("[Integrations] Failed to load CLI status", error);
        setCliStatus(null);
      })
      .finally(markDone);

    void getSkillsInstallStatus()
      .then(setSkillsStatus)
      .catch((error) => {
        console.error("[Integrations] Failed to load skills status", error);
        setSkillsStatus(null);
      })
      .finally(markDone);

    void getModelCliRuntimeStatus()
      .then((status) => {
        setModelCliStatus(status);
        setModelRuntimeUnavailable(false);
      })
      .catch((error) => {
        console.error("[Integrations] Failed to load model CLI runtime status", error);
        setModelCliStatus(null);
        setModelRuntimeUnavailable(true);
      })
      .finally(markDone);
  }, [showSection]);

  useFocusEffect(
    useCallback(() => {
      if (!showSection) return undefined;
      loadStatus();
      return undefined;
    }, [loadStatus, showSection]),
  );

  const handleInstallCli = useCallback(() => {
    if (isInstallingCli) return;
    setIsInstallingCli(true);
    void installCli()
      .then(setCliStatus)
      .catch((error) => {
        console.error("[Integrations] Failed to install CLI", error);
        Alert.alert(text.installFailed, error instanceof Error ? error.message : String(error));
      })
      .finally(() => {
        setIsInstallingCli(false);
      });
  }, [isInstallingCli]);

  const handleInstallSkills = useCallback(() => {
    if (isInstallingSkills) return;
    setIsInstallingSkills(true);
    void installSkills()
      .then(setSkillsStatus)
      .catch((error) => {
        console.error("[Integrations] Failed to install skills", error);
        Alert.alert(text.installFailed, error instanceof Error ? error.message : String(error));
      })
      .finally(() => {
        setIsInstallingSkills(false);
      });
  }, [isInstallingSkills]);

  const handleInstallNodeRuntime = useCallback(() => {
    if (isInstallingNodeRuntime) return;
    setIsInstallingNodeRuntime(true);
    void installNode22Runtime()
      .then((result) => {
        setModelCliStatus(result.status);
      })
      .catch((error) => {
        console.error("[Integrations] Failed to install Node.js 22 runtime", error);
        Alert.alert(text.installFailed, error instanceof Error ? error.message : String(error));
      })
      .finally(() => {
        setIsInstallingNodeRuntime(false);
      });
  }, [isInstallingNodeRuntime]);

  const handleInstallCodex = useCallback(() => {
    if (isInstallingCodex) return;
    setIsInstallingCodex(true);
    void installCodexCli()
      .then((result) => {
        setModelCliStatus(result.status);
      })
      .catch((error) => {
        console.error("[Integrations] Failed to install Codex CLI", error);
        Alert.alert(text.installFailed, error instanceof Error ? error.message : String(error));
      })
      .finally(() => {
        setIsInstallingCodex(false);
      });
  }, [isInstallingCodex]);

  const handleInstallClaudeCode = useCallback(() => {
    if (isInstallingClaudeCode) return;
    setIsInstallingClaudeCode(true);
    void installClaudeCodeCli()
      .then((result) => {
        setModelCliStatus(result.status);
      })
      .catch((error) => {
        console.error("[Integrations] Failed to install Claude Code CLI", error);
        Alert.alert(text.installFailed, error instanceof Error ? error.message : String(error));
      })
      .finally(() => {
        setIsInstallingClaudeCode(false);
      });
  }, [isInstallingClaudeCode]);

  const handleInstallAll = useCallback(() => {
    if (isInstallingAll) return;
    setIsInstallingAll(true);
    void installAllModelClis()
      .then((result) => {
        setModelCliStatus(result.status);
      })
      .catch((error) => {
        console.error("[Integrations] Failed to install Node.js and model CLIs", error);
        Alert.alert(text.installFailed, error instanceof Error ? error.message : String(error));
      })
      .finally(() => {
        setIsInstallingAll(false);
      });
  }, [isInstallingAll]);

  if (!showSection) {
    return null;
  }

  const nodeRuntimeHint = integrationCheckPending
    ? text.checkingEnvironment
    : modelRuntimeUnavailable
      ? text.runtimeStatusUnavailable
      : modelCliStatus?.node.installed
        ? modelCliStatus.node.satisfies
          ? text.nodeRuntimeStatus(modelCliStatus.node.version, modelCliStatus.node.npmVersion ?? text.unknown)
          : text.nodeVersionMismatch(modelCliStatus.node.version)
        : (modelCliStatus?.node.error ?? text.nodeNotDetected);
  const codexHint = integrationCheckPending
    ? text.checkingEnvironment
    : modelRuntimeUnavailable
      ? text.runtimeStatusUnavailable
      : modelCliStatus?.codex.installed
        ? text.codexInstalled(modelCliStatus.codex.version ?? text.installed)
        : (modelCliStatus?.codex.error ?? text.codexInstallHint);
  const claudeHint = integrationCheckPending
    ? text.checkingEnvironment
    : modelRuntimeUnavailable
      ? text.runtimeStatusUnavailable
      : modelCliStatus?.claude.installed
        ? text.claudeInstalled(modelCliStatus.claude.version ?? text.installed)
        : (modelCliStatus?.claude.error ?? text.claudeInstallHint);
  const isRuntimeBusy =
    isInstallingNodeRuntime || isInstallingCodex || isInstallingClaudeCode || isInstallingAll;
  const runtimeActionsDisabled = isRuntimeBusy || integrationCheckPending;

  const trailing = (
    <View style={styles.headerLinks}>
      <Button
        variant="ghost"
        size="sm"
        leftIcon={<ArrowUpRight size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />}
        textStyle={settingsStyles.sectionHeaderLinkText}
        style={settingsStyles.sectionHeaderLink}
        onPress={() => void openExternalUrl(CLI_DOCS_URL)}
        accessibilityLabel={text.openCliDocs}
      >
        {text.cliDocs}
      </Button>
      <Button
        variant="ghost"
        size="sm"
        leftIcon={<ArrowUpRight size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />}
        textStyle={settingsStyles.sectionHeaderLinkText}
        style={settingsStyles.sectionHeaderLink}
        onPress={() => void openExternalUrl(SKILLS_DOCS_URL)}
        accessibilityLabel={text.openSkillsDocs}
      >
        {text.skillsDocs}
      </Button>
    </View>
  );

  return (
    <SettingsSection title={text.title} trailing={trailing}>
      <View style={settingsStyles.card}>
        <View style={settingsStyles.row}>
          <View style={settingsStyles.rowContent}>
            <View style={styles.rowTitleRow}>
              <Terminal size={theme.iconSize.md} color={theme.colors.foreground} />
              <Text style={settingsStyles.rowTitle}>{text.commandLine}</Text>
            </View>
            <Text style={settingsStyles.rowHint}>{text.commandLineHint}</Text>
          </View>
          {integrationCheckPending ? (
            <ActivityIndicator size="small" color={theme.colors.foregroundMuted} />
          ) : cliStatus?.installed ? (
            <View style={styles.installedLabel}>
              <Check size={14} color={theme.colors.foregroundMuted} />
              <Text style={styles.mutedText}>{text.installed}</Text>
            </View>
          ) : (
            <Button
              variant="outline"
              size="sm"
              onPress={handleInstallCli}
              disabled={isInstallingCli || integrationCheckPending}
            >
              {isInstallingCli ? text.installing : text.install}
            </Button>
          )}
        </View>
        <View style={[settingsStyles.row, settingsStyles.rowBorder]}>
          <View style={settingsStyles.rowContent}>
            <View style={styles.rowTitleRow}>
              <Blocks size={theme.iconSize.md} color={theme.colors.foreground} />
              <Text style={settingsStyles.rowTitle}>{text.orchestrationSkills}</Text>
            </View>
            <Text style={settingsStyles.rowHint}>{text.orchestrationSkillsHint}</Text>
          </View>
          {integrationCheckPending ? (
            <ActivityIndicator size="small" color={theme.colors.foregroundMuted} />
          ) : skillsStatus?.installed ? (
            <View style={styles.installedLabel}>
              <Check size={14} color={theme.colors.foregroundMuted} />
              <Text style={styles.mutedText}>{text.installed}</Text>
            </View>
          ) : (
            <Button
              variant="outline"
              size="sm"
              onPress={handleInstallSkills}
              disabled={isInstallingSkills || integrationCheckPending}
            >
              {isInstallingSkills ? text.installing : text.install}
            </Button>
          )}
        </View>
      </View>

      <View style={settingsStyles.card}>
        <View style={settingsStyles.row}>
          <View style={settingsStyles.rowContent}>
            <View style={styles.rowTitleRow}>
              <Cpu size={theme.iconSize.md} color={theme.colors.foreground} />
              <Text style={settingsStyles.rowTitle}>{text.nodeRuntime}</Text>
            </View>
            <Text style={settingsStyles.rowHint}>{nodeRuntimeHint}</Text>
          </View>
          {integrationCheckPending ? (
            <ActivityIndicator size="small" color={theme.colors.foregroundMuted} />
          ) : modelCliStatus?.node.satisfies ? (
            <View style={styles.installedLabel}>
              <Check size={14} color={theme.colors.foregroundMuted} />
              <Text style={styles.mutedText}>{text.ready}</Text>
            </View>
          ) : (
            <Button
              variant="outline"
              size="sm"
              onPress={handleInstallNodeRuntime}
              disabled={runtimeActionsDisabled}
            >
              {isInstallingNodeRuntime ? text.installing : text.install}
            </Button>
          )}
        </View>
        <View style={[settingsStyles.row, settingsStyles.rowBorder]}>
          <View style={settingsStyles.rowContent}>
            <View style={styles.rowTitleRow}>
              <Terminal size={theme.iconSize.md} color={theme.colors.foreground} />
              <Text style={settingsStyles.rowTitle}>{text.codexCli}</Text>
            </View>
            <Text style={settingsStyles.rowHint}>{codexHint}</Text>
          </View>
          <Button
            variant="outline"
            size="sm"
            onPress={handleInstallCodex}
            disabled={runtimeActionsDisabled}
          >
            {isInstallingCodex
              ? text.installing
              : modelCliStatus?.codex.installed
                ? text.reinstall
                : text.install}
          </Button>
        </View>
        <View style={[settingsStyles.row, settingsStyles.rowBorder]}>
          <View style={settingsStyles.rowContent}>
            <View style={styles.rowTitleRow}>
              <Terminal size={theme.iconSize.md} color={theme.colors.foreground} />
              <Text style={settingsStyles.rowTitle}>{text.claudeCodeCli}</Text>
            </View>
            <Text style={settingsStyles.rowHint}>{claudeHint}</Text>
          </View>
          <Button
            variant="outline"
            size="sm"
            onPress={handleInstallClaudeCode}
            disabled={runtimeActionsDisabled}
          >
            {isInstallingClaudeCode
              ? text.installing
              : modelCliStatus?.claude.installed
                ? text.reinstall
                : text.install}
          </Button>
        </View>
        <View style={[settingsStyles.row, settingsStyles.rowBorder]}>
          <View style={settingsStyles.rowContent}>
            <View style={styles.rowTitleRow}>
              <Wrench size={theme.iconSize.md} color={theme.colors.foreground} />
              <Text style={settingsStyles.rowTitle}>{text.externalAgentStack}</Text>
            </View>
            <Text style={settingsStyles.rowHint}>{text.externalAgentStackHint}</Text>
          </View>
          <Button
            variant="outline"
            size="sm"
            onPress={handleInstallAll}
            disabled={runtimeActionsDisabled}
          >
            {isInstallingAll ? text.installing : text.installAll}
          </Button>
        </View>
      </View>
    </SettingsSection>
  );
}

const styles = StyleSheet.create((theme) => ({
  headerLinks: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[0],
  },
  rowTitleRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
  },
  installedLabel: {
    flexDirection: "row",
    alignItems: "center",
    gap: 4,
  },
  mutedText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
  },
}));

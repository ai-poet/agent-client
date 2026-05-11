import { useCallback, useState } from "react";
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

const CLI_DOCS_URL = "https://paseo.sh/docs/cli";
const SKILLS_DOCS_URL = "https://paseo.sh/docs/skills";

const CHECKING_ENV_MESSAGE = "Checking environment…";
const RUNTIME_STATUS_UNAVAILABLE =
  "Couldn't read managed Node / Codex / Claude status. You can still try Install below.";

export function IntegrationsSection() {
  const { theme } = useUnistyles();
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
        Alert.alert("Install failed", error instanceof Error ? error.message : String(error));
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
        Alert.alert("Install failed", error instanceof Error ? error.message : String(error));
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
        Alert.alert("Install failed", error instanceof Error ? error.message : String(error));
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
        Alert.alert("Install failed", error instanceof Error ? error.message : String(error));
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
        Alert.alert("Install failed", error instanceof Error ? error.message : String(error));
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
        Alert.alert("Install failed", error instanceof Error ? error.message : String(error));
      })
      .finally(() => {
        setIsInstallingAll(false);
      });
  }, [isInstallingAll]);

  if (!showSection) {
    return null;
  }

  const nodeRuntimeHint = integrationCheckPending
    ? CHECKING_ENV_MESSAGE
    : modelRuntimeUnavailable
      ? RUNTIME_STATUS_UNAVAILABLE
      : modelCliStatus?.node.installed
        ? modelCliStatus.node.satisfies
          ? `Node.js ${modelCliStatus.node.version} · npm ${modelCliStatus.node.npmVersion ?? "unknown"}`
          : `Detected Node.js ${modelCliStatus.node.version}. Use Node 22 for Codex and Claude Code installs.`
        : (modelCliStatus?.node.error ?? "Node.js was not detected yet.");
  const codexHint = integrationCheckPending
    ? CHECKING_ENV_MESSAGE
    : modelRuntimeUnavailable
      ? RUNTIME_STATUS_UNAVAILABLE
      : modelCliStatus?.codex.installed
        ? `Codex ${modelCliStatus.codex.version ?? "installed"}`
        : (modelCliStatus?.codex.error ??
          "Install the Codex CLI into the managed Node 22 runtime.");
  const claudeHint = integrationCheckPending
    ? CHECKING_ENV_MESSAGE
    : modelRuntimeUnavailable
      ? RUNTIME_STATUS_UNAVAILABLE
      : modelCliStatus?.claude.installed
        ? `Claude Code ${modelCliStatus.claude.version ?? "installed"}`
        : (modelCliStatus?.claude.error ??
          "Install the Claude Code CLI into the managed Node 22 runtime.");
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
        accessibilityLabel="Open CLI documentation"
      >
        CLI docs
      </Button>
      <Button
        variant="ghost"
        size="sm"
        leftIcon={<ArrowUpRight size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />}
        textStyle={settingsStyles.sectionHeaderLinkText}
        style={settingsStyles.sectionHeaderLink}
        onPress={() => void openExternalUrl(SKILLS_DOCS_URL)}
        accessibilityLabel="Open skills documentation"
      >
        Skills docs
      </Button>
    </View>
  );

  return (
    <SettingsSection title="Integrations" trailing={trailing}>
      <View style={settingsStyles.card}>
        <View style={settingsStyles.row}>
          <View style={settingsStyles.rowContent}>
            <View style={styles.rowTitleRow}>
              <Terminal size={theme.iconSize.md} color={theme.colors.foreground} />
              <Text style={settingsStyles.rowTitle}>Command line</Text>
            </View>
            <Text style={settingsStyles.rowHint}>
              Control and script agents from your terminal.
            </Text>
          </View>
          {integrationCheckPending ? (
            <ActivityIndicator size="small" color={theme.colors.foregroundMuted} />
          ) : cliStatus?.installed ? (
            <View style={styles.installedLabel}>
              <Check size={14} color={theme.colors.foregroundMuted} />
              <Text style={styles.mutedText}>Installed</Text>
            </View>
          ) : (
            <Button
              variant="outline"
              size="sm"
              onPress={handleInstallCli}
              disabled={isInstallingCli || integrationCheckPending}
            >
              {isInstallingCli ? "Installing..." : "Install"}
            </Button>
          )}
        </View>
        <View style={[settingsStyles.row, settingsStyles.rowBorder]}>
          <View style={settingsStyles.rowContent}>
            <View style={styles.rowTitleRow}>
              <Blocks size={theme.iconSize.md} color={theme.colors.foreground} />
              <Text style={settingsStyles.rowTitle}>Orchestration skills</Text>
            </View>
            <Text style={settingsStyles.rowHint}>
              Teach your agents to orchestrate through the CLI.
            </Text>
          </View>
          {integrationCheckPending ? (
            <ActivityIndicator size="small" color={theme.colors.foregroundMuted} />
          ) : skillsStatus?.installed ? (
            <View style={styles.installedLabel}>
              <Check size={14} color={theme.colors.foregroundMuted} />
              <Text style={styles.mutedText}>Installed</Text>
            </View>
          ) : (
            <Button
              variant="outline"
              size="sm"
              onPress={handleInstallSkills}
              disabled={isInstallingSkills || integrationCheckPending}
            >
              {isInstallingSkills ? "Installing..." : "Install"}
            </Button>
          )}
        </View>
      </View>

      <View style={settingsStyles.card}>
        <View style={settingsStyles.row}>
          <View style={settingsStyles.rowContent}>
            <View style={styles.rowTitleRow}>
              <Cpu size={theme.iconSize.md} color={theme.colors.foreground} />
              <Text style={settingsStyles.rowTitle}>Node.js 22 runtime</Text>
            </View>
            <Text style={settingsStyles.rowHint}>{nodeRuntimeHint}</Text>
          </View>
          {integrationCheckPending ? (
            <ActivityIndicator size="small" color={theme.colors.foregroundMuted} />
          ) : modelCliStatus?.node.satisfies ? (
            <View style={styles.installedLabel}>
              <Check size={14} color={theme.colors.foregroundMuted} />
              <Text style={styles.mutedText}>Ready</Text>
            </View>
          ) : (
            <Button
              variant="outline"
              size="sm"
              onPress={handleInstallNodeRuntime}
              disabled={runtimeActionsDisabled}
            >
              {isInstallingNodeRuntime ? "Installing..." : "Install"}
            </Button>
          )}
        </View>
        <View style={[settingsStyles.row, settingsStyles.rowBorder]}>
          <View style={settingsStyles.rowContent}>
            <View style={styles.rowTitleRow}>
              <Terminal size={theme.iconSize.md} color={theme.colors.foreground} />
              <Text style={settingsStyles.rowTitle}>Codex CLI</Text>
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
              ? "Installing..."
              : modelCliStatus?.codex.installed
                ? "Reinstall"
                : "Install"}
          </Button>
        </View>
        <View style={[settingsStyles.row, settingsStyles.rowBorder]}>
          <View style={settingsStyles.rowContent}>
            <View style={styles.rowTitleRow}>
              <Terminal size={theme.iconSize.md} color={theme.colors.foreground} />
              <Text style={settingsStyles.rowTitle}>Claude Code CLI</Text>
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
              ? "Installing..."
              : modelCliStatus?.claude.installed
                ? "Reinstall"
                : "Install"}
          </Button>
        </View>
        <View style={[settingsStyles.row, settingsStyles.rowBorder]}>
          <View style={settingsStyles.rowContent}>
            <View style={styles.rowTitleRow}>
              <Wrench size={theme.iconSize.md} color={theme.colors.foreground} />
              <Text style={settingsStyles.rowTitle}>External agent stack</Text>
            </View>
            <Text style={settingsStyles.rowHint}>
              Install Git Bash, Node.js 22, Codex, and Claude Code in one pass.
            </Text>
          </View>
          <Button
            variant="outline"
            size="sm"
            onPress={handleInstallAll}
            disabled={runtimeActionsDisabled}
          >
            {isInstallingAll ? "Installing..." : "Install all"}
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

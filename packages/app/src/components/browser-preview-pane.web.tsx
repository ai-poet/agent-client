import { useMemo, useState, type ReactElement } from "react";
import { ExternalLink, RotateCw } from "lucide-react-native";
import { Pressable, Text, View } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import type { WorkspaceScriptPayload } from "@server/shared/messages";
import { useAppLocale } from "@/hooks/use-app-locale";
import { useHostRuntimeSnapshot } from "@/runtime/host-runtime";
import { openExternalUrl } from "@/utils/open-external-url";
import { resolveWorkspaceScriptLink } from "@/utils/workspace-script-links";
import { getAppMessages } from "@/i18n/sub2api";

interface BrowserPreviewPaneProps {
  serverId: string;
  scriptName: string;
  scripts: readonly WorkspaceScriptPayload[];
}

function stripUrlProtocol(url: string): string {
  return url.replace(/^https?:\/\//, "");
}

function findPreviewScript(
  scripts: readonly WorkspaceScriptPayload[],
  scriptName: string,
): WorkspaceScriptPayload | null {
  return (
    scripts.find(
      (script) =>
        script.scriptName === scriptName &&
        script.type === "service" &&
        script.lifecycle === "running",
    ) ?? null
  );
}

export function BrowserPreviewPane({
  serverId,
  scriptName,
  scripts,
}: BrowserPreviewPaneProps): ReactElement {
  const { theme } = useUnistyles();
  const locale = useAppLocale();
  const text = useMemo(() => getAppMessages(locale).workspace, [locale]);
  const activeConnection = useHostRuntimeSnapshot(serverId)?.activeConnection ?? null;
  const [reloadKey, setReloadKey] = useState(0);

  const preview = useMemo(() => {
    const script = findPreviewScript(scripts, scriptName);
    if (!script) {
      return null;
    }
    const link = resolveWorkspaceScriptLink({ script, activeConnection });
    return link.openUrl ? { script, url: link.openUrl } : null;
  }, [activeConnection, scriptName, scripts]);

  if (!preview) {
    return (
      <View style={styles.emptyState} testID="browser-preview-pane-unavailable">
        <Text style={styles.emptyTitle}>{scriptName}</Text>
        <Text style={styles.emptyText}>{text.previewUnavailable}</Text>
      </View>
    );
  }

  return (
    <View style={styles.container} testID="browser-preview-pane">
      <View style={styles.toolbar}>
        <View style={styles.titleGroup}>
          <Text style={styles.title} numberOfLines={1}>
            {preview.script.scriptName}
          </Text>
          <Text style={styles.url} numberOfLines={1} selectable>
            {stripUrlProtocol(preview.url)}
          </Text>
        </View>
        <View style={styles.actions}>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={text.reloadPreview}
            testID="browser-preview-reload"
            onPress={() => setReloadKey((current) => current + 1)}
            style={({ hovered, pressed }) => [
              styles.iconButton,
              (hovered || pressed) && styles.iconButtonActive,
            ]}
          >
            <RotateCw size={16} color={theme.colors.foregroundMuted} />
          </Pressable>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={text.openPreviewExternal}
            testID="browser-preview-open-external"
            onPress={() => void openExternalUrl(preview.url)}
            style={({ hovered, pressed }) => [
              styles.iconButton,
              (hovered || pressed) && styles.iconButtonActive,
            ]}
          >
            <ExternalLink size={16} color={theme.colors.foregroundMuted} />
          </Pressable>
        </View>
      </View>
      <iframe
        key={`${preview.url}:${reloadKey}`}
        src={preview.url}
        title={preview.script.scriptName}
        sandbox="allow-forms allow-modals allow-popups allow-same-origin allow-scripts"
        style={{
          flex: 1,
          width: "100%",
          height: "100%",
          border: 0,
          background: theme.colors.surface0,
        }}
      />
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  container: {
    flex: 1,
    minWidth: 0,
    backgroundColor: theme.colors.surface0,
  },
  toolbar: {
    minHeight: 44,
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    paddingHorizontal: theme.spacing[3],
    borderBottomWidth: theme.borderWidth[1],
    borderBottomColor: theme.colors.border,
    backgroundColor: theme.colors.surface1,
  },
  titleGroup: {
    flex: 1,
    minWidth: 0,
  },
  title: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.medium,
    lineHeight: theme.fontSize.sm * 1.35,
  },
  url: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    lineHeight: theme.fontSize.xs * 1.35,
  },
  actions: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[1],
    flexShrink: 0,
  },
  iconButton: {
    width: 30,
    height: 30,
    alignItems: "center",
    justifyContent: "center",
  },
  iconButtonActive: {
    backgroundColor: theme.colors.surface2,
  },
  emptyState: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    padding: theme.spacing[6],
    backgroundColor: theme.colors.surface0,
  },
  emptyTitle: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.lg,
    fontWeight: theme.fontWeight.medium,
    marginBottom: theme.spacing[2],
  },
  emptyText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    textAlign: "center",
  },
}));

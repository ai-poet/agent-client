import { useMemo, type ReactElement } from "react";
import { Text, View } from "react-native";
import { StyleSheet } from "react-native-unistyles";
import type { WorkspaceScriptPayload } from "@server/shared/messages";
import { useAppLocale } from "@/hooks/use-app-locale";
import { getAppMessages } from "@/i18n/sub2api";

interface BrowserPreviewPaneProps {
  serverId: string;
  scriptName: string;
  scripts: readonly WorkspaceScriptPayload[];
}

export function BrowserPreviewPane({ scriptName }: BrowserPreviewPaneProps): ReactElement {
  const locale = useAppLocale();
  const text = useMemo(() => getAppMessages(locale).workspace, [locale]);

  return (
    <View style={styles.emptyState} testID="browser-preview-pane-native-unavailable">
      <Text style={styles.emptyTitle}>{scriptName}</Text>
      <Text style={styles.emptyText}>{text.previewUnavailableNative}</Text>
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
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

import { useLocalSearchParams, useRouter } from "expo-router";
import { ArrowLeft, GitGraph } from "lucide-react-native";
import { Pressable, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { CommitGraphPane } from "@/components/git-graph";
import { useWorkspaceExecutionAuthority } from "@/stores/session-store-hooks";

function getParamValue(value: string | string[] | undefined): string {
  if (typeof value === "string") {
    return value.trim();
  }
  if (Array.isArray(value)) {
    const firstValue = value[0];
    return typeof firstValue === "string" ? firstValue.trim() : "";
  }
  return "";
}

export default function CommitGraphRoute() {
  const { serverId, workspaceId } = useLocalSearchParams();
  const router = useRouter();
  const { theme } = useUnistyles();
  const insets = useSafeAreaInsets();

  const normalizedServerId = getParamValue(serverId);
  const normalizedWorkspaceId = getParamValue(workspaceId);
  const workspaceAuthority = useWorkspaceExecutionAuthority(
    normalizedServerId || null,
    normalizedWorkspaceId || null,
  );
  const graphCwd =
    workspaceAuthority?.ok === true
      ? workspaceAuthority.authority.workspaceDirectory
      : normalizedWorkspaceId;

  const handleBack = () => {
    router.back();
  };

  return (
    <View style={[styles.container, { paddingTop: insets.top }]}>
      <View style={styles.header}>
        <Pressable
          onPress={handleBack}
          style={({ pressed }) => [styles.backButton, pressed && styles.backButtonPressed]}
          accessibilityRole="button"
          accessibilityLabel="Go back"
        >
          <ArrowLeft size={20} color={theme.colors.foreground} />
        </Pressable>
        <View style={styles.headerCenter}>
          <Text style={[styles.headerTitle, { color: theme.colors.foreground }]}>Git Graph</Text>
          <Text style={[styles.headerSubtitle, { color: theme.colors.foregroundMuted }]}>
            {normalizedWorkspaceId}
          </Text>
        </View>
        <GitGraph size={20} color={theme.colors.foregroundMuted} />
      </View>
      <View style={styles.content}>
        {normalizedServerId && graphCwd ? (
          <CommitGraphPane serverId={normalizedServerId} cwd={graphCwd} />
        ) : (
          <View style={styles.error}>
            <Text style={[styles.errorText, { color: theme.colors.foregroundMuted }]}>
              Missing server or workspace ID
            </Text>
          </View>
        )}
      </View>
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  container: {
    flex: 1,
    backgroundColor: theme.colors.background,
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: 16,
    paddingVertical: 12,
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
  },
  backButton: {
    padding: 8,
    borderRadius: theme.borderRadius.md,
  },
  backButtonPressed: {
    backgroundColor: theme.colors.surface2,
  },
  headerCenter: {
    flex: 1,
    alignItems: "center",
  },
  headerTitle: {
    fontSize: 16,
    fontWeight: "600",
  },
  headerSubtitle: {
    fontSize: 12,
    marginTop: 2,
  },
  content: {
    flex: 1,
  },
  error: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    padding: 24,
  },
  errorText: {
    fontSize: 15,
  },
}));

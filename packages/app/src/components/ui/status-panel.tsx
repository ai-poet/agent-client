import type { ReactNode } from "react";
import { ActivityIndicator, Text, View } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";

interface StatusPanelProps {
  title: string;
  /**
   * Say what to do next or why the list is empty — never filler. Prefer query-aware copy:
   * "No matching X" when filtered vs "No X yet" when genuinely empty.
   */
  description?: string;
  icon?: ReactNode;
  /** Swaps the icon for a spinner. */
  loading?: boolean;
  /** Tints the copy as an error. Pass the message as `description`. */
  error?: boolean;
  /** A recovery action — retry, or the thing the description tells the user to do. */
  action?: ReactNode;
}

/**
 * One shell for empty, loading and error states so a list never renders three
 * differently-shaped placeholders.
 */
export function StatusPanel({
  title,
  description,
  icon,
  loading = false,
  error = false,
  action,
}: StatusPanelProps) {
  const { theme } = useUnistyles();

  return (
    <View style={styles.container}>
      {loading ? (
        <ActivityIndicator size="small" color={theme.colors.foregroundMuted} />
      ) : icon ? (
        <View style={styles.icon}>{icon}</View>
      ) : null}
      <Text style={[styles.title, error && styles.titleError]}>{title}</Text>
      {description ? (
        <Text style={[styles.description, error && styles.descriptionError]}>{description}</Text>
      ) : null}
      {action ? <View style={styles.action}>{action}</View> : null}
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  container: {
    alignItems: "center",
    justifyContent: "center",
    gap: theme.spacing[2],
    paddingVertical: theme.spacing[6],
    paddingHorizontal: theme.spacing[4],
  },
  icon: {
    flexDirection: "row",
    alignItems: "center",
  },
  title: {
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.medium,
    color: theme.colors.foreground,
    textAlign: "center",
  },
  titleError: {
    color: theme.colors.destructive,
  },
  description: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
    textAlign: "center",
    lineHeight: 18,
  },
  descriptionError: {
    color: theme.colors.foregroundMuted,
  },
  action: {
    marginTop: theme.spacing[1],
  },
}));

import type { ReactNode } from "react";
import { View, Text } from "react-native";
import { StyleSheet } from "react-native-unistyles";

/**
 * `pending` and `muted` look different on purpose: "checking" must not read the same
 * as "not installed".
 */
type StatusBadgeVariant = "success" | "error" | "warning" | "pending" | "muted";

interface StatusBadgeProps {
  label: string;
  variant?: StatusBadgeVariant;
  /** Rendered before the label — a spinner or a semantic glyph. */
  icon?: ReactNode;
}

export function StatusBadge({ label, variant = "muted", icon }: StatusBadgeProps) {
  return (
    <View
      style={[
        styles.pill,
        variant === "success" && styles.pillSuccess,
        variant === "error" && styles.pillError,
        variant === "warning" && styles.pillWarning,
        variant === "pending" && styles.pillPending,
      ]}
    >
      {icon ? <View style={styles.icon}>{icon}</View> : null}
      <Text
        style={[
          styles.pillText,
          variant === "success" && styles.pillTextSuccess,
          variant === "error" && styles.pillTextError,
          variant === "warning" && styles.pillTextWarning,
          variant === "pending" && styles.pillTextPending,
        ]}
      >
        {label}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  pill: {
    flexDirection: "row",
    alignItems: "center",
    gap: 4,
    borderRadius: theme.borderRadius.full,
    borderWidth: 1,
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.surface3,
    paddingHorizontal: theme.spacing[2],
    paddingVertical: 3,
  },
  icon: {
    flexDirection: "row",
    alignItems: "center",
  },
  pillSuccess: {
    backgroundColor: theme.colors.palette.green[900],
    borderColor: theme.colors.palette.green[800],
  },
  pillError: {
    backgroundColor: theme.colors.palette.red[900],
    borderColor: theme.colors.palette.red[800],
  },
  pillWarning: {
    backgroundColor: theme.colors.surface3,
    borderColor: theme.colors.statusWarning,
  },
  pillPending: {
    backgroundColor: theme.colors.surface2,
    borderColor: theme.colors.borderAccent,
  },
  pillText: {
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.normal,
    color: theme.colors.foregroundMuted,
  },
  pillTextSuccess: {
    color: theme.colors.palette.green[400],
  },
  pillTextError: {
    color: theme.colors.palette.red[500],
  },
  pillTextWarning: {
    color: theme.colors.statusWarning,
  },
  pillTextPending: {
    color: theme.colors.foreground,
  },
}));

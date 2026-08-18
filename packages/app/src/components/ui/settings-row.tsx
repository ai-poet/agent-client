import type { ReactNode } from "react";
import { View, Text } from "react-native";
import { StyleSheet } from "react-native-unistyles";

interface SettingsRowProps {
  title: string;
  /** Explains the setting, or — when there is no control — documents behavior in place. */
  description?: string;
  /** Leading glyph, sized by the caller. */
  icon?: ReactNode;
  /** Badge or hint rendered right after the title. */
  titleSuffix?: ReactNode;
  /** The control. Omit to render a pure information row. */
  children?: ReactNode;
  /** Move the control below the text instead of beside it — for wide inputs. */
  stack?: boolean;
  /** Dim the row when it is informational rather than actionable. */
  muted?: boolean;
  /** Adds the separator that divides consecutive rows in a card. */
  bordered?: boolean;
}

/**
 * The one row shape used across settings screens: icon | (title + description) | control.
 */
export function SettingsRow({
  title,
  description,
  icon,
  titleSuffix,
  children,
  stack = false,
  muted = false,
  bordered = false,
}: SettingsRowProps) {
  const text = (
    <View style={styles.content}>
      <View style={styles.titleRow}>
        {icon ? <View style={styles.icon}>{icon}</View> : null}
        <Text style={[styles.title, muted && styles.titleMuted]}>{title}</Text>
        {titleSuffix}
      </View>
      {description ? <Text style={styles.description}>{description}</Text> : null}
    </View>
  );

  if (stack) {
    return (
      <View style={[styles.row, styles.rowStacked, bordered && styles.rowBordered]}>
        {text}
        {children ? <View style={styles.stackedControl}>{children}</View> : null}
      </View>
    );
  }

  return (
    <View style={[styles.row, bordered && styles.rowBordered]}>
      {text}
      {children ? <View style={styles.control}>{children}</View> : null}
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  row: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: theme.spacing[3],
    paddingVertical: theme.spacing[3],
    paddingHorizontal: theme.spacing[4],
  },
  rowStacked: {
    flexDirection: "column",
    alignItems: "stretch",
  },
  rowBordered: {
    borderTopWidth: 1,
    borderTopColor: theme.colors.border,
  },
  content: {
    flex: 1,
    gap: 2,
  },
  titleRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
  },
  icon: {
    flexDirection: "row",
    alignItems: "center",
  },
  title: {
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.medium,
    color: theme.colors.foreground,
  },
  titleMuted: {
    color: theme.colors.foregroundMuted,
    fontWeight: theme.fontWeight.normal,
  },
  description: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
    lineHeight: 17,
  },
  control: {
    flexShrink: 0,
  },
  stackedControl: {
    marginTop: theme.spacing[2],
  },
}));

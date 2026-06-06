import { GitBranch, Tag } from "lucide-react-native";
import { Text, View } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import type { RefBadgeProps } from "./graph-types";

export function RefBadge({ label, kind }: RefBadgeProps) {
  const { theme } = useUnistyles();
  const Icon = kind === "tag" ? Tag : GitBranch;
  return (
    <View style={[styles.refBadge, kind === "tag" ? styles.tagBadge : styles.branchBadge]}>
      <Icon size={11} color={kind === "tag" ? theme.colors.foregroundMuted : theme.colors.accent} />
      <Text
        numberOfLines={1}
        style={[styles.refBadgeText, kind === "tag" ? styles.tagBadgeText : styles.branchBadgeText]}
      >
        {label}
      </Text>
    </View>
  );
}

export function HiddenRefsBadge({ count }: { count: number }) {
  return (
    <View style={styles.hiddenRefsBadge}>
      <Text style={styles.hiddenRefsText}>+{count}</Text>
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  refBadge: {
    maxWidth: 96,
    height: 18,
    flexDirection: "row",
    alignItems: "center",
    gap: 4,
    paddingHorizontal: 5,
    borderRadius: theme.borderRadius.base,
    borderWidth: 1,
  },
  branchBadge: {
    borderColor: theme.colors.borderAccent,
    backgroundColor: theme.colors.surface2,
  },
  tagBadge: {
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.surface1,
  },
  refBadgeText: {
    flexShrink: 1,
    minWidth: 0,
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.normal,
  },
  branchBadgeText: {
    color: theme.colors.accent,
  },
  tagBadgeText: {
    color: theme.colors.foregroundMuted,
  },
  hiddenRefsBadge: {
    height: 18,
    justifyContent: "center",
    paddingHorizontal: 5,
    borderRadius: theme.borderRadius.base,
    borderWidth: 1,
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.surface1,
  },
  hiddenRefsText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
}));

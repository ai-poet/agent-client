import { GitCompare, X } from "lucide-react-native";
import { Pressable, Text, View } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { Fonts } from "@/constants/theme";
import { formatCommitDate } from "@/utils/git-graph-helpers";
import { RefBadge } from "./ref-badge";
import type { CommitDetailPanelProps } from "./graph-types";

export function CommitDetailPanel({
  commit,
  compareCommit,
  onClearCompare,
  onClose,
}: CommitDetailPanelProps) {
  const { theme } = useUnistyles();

  return (
    <View style={styles.panel} testID="commit-graph-detail-panel">
      <View style={styles.panelHeader}>
        <Text style={styles.panelHash} selectable numberOfLines={1}>
          {commit.fullHash}
        </Text>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="关闭详情"
          onPress={onClose}
          style={styles.closeButton}
        >
          <X size={14} color={theme.colors.foregroundMuted} />
        </Pressable>
      </View>

      <View style={styles.panelBody}>
        <View style={styles.infoRow}>
          <Text style={styles.infoItem}>
            <Text style={styles.infoLabel}>Author: </Text>
            {commit.author}
          </Text>
          <Text style={styles.infoItem}>
            <Text style={styles.infoLabel}>Date: </Text>
            {formatCommitDate(commit.date)}
          </Text>
          <Text style={styles.infoItem} numberOfLines={1}>
            <Text style={styles.infoLabel}>Parents: </Text>
            {commit.parents.length > 0 ? commit.parents.join(" ") : "None"}
          </Text>
        </View>

        <Text style={styles.message} numberOfLines={2}>
          {commit.message || "(no commit message)"}
        </Text>

        {commit.branchTips.length > 0 || commit.tags.length > 0 ? (
          <View style={styles.refsRow}>
            {commit.branchTips.map((branch) => (
              <RefBadge key={`detail-branch-${branch}`} label={branch} kind="branch" />
            ))}
            {commit.tags.map((tag) => (
              <RefBadge key={`detail-tag-${tag}`} label={tag} kind="tag" />
            ))}
          </View>
        ) : null}

        {compareCommit ? (
          <View style={styles.compareBanner}>
            <GitCompare size={14} color={theme.colors.foregroundMuted} />
            <Text style={styles.compareText} numberOfLines={1}>
              Comparing {compareCommit.hash} to {commit.hash}
            </Text>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="清除对比"
              onPress={onClearCompare}
              style={styles.compareClearButton}
            >
              <X size={14} color={theme.colors.foregroundMuted} />
            </Pressable>
          </View>
        ) : null}
      </View>
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  panel: {
    height: 140,
    borderTopWidth: 1,
    borderTopColor: theme.colors.border,
    backgroundColor: theme.colors.surface1,
    paddingHorizontal: 12,
    paddingVertical: 10,
  },
  panelHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    marginBottom: 6,
  },
  panelHash: {
    flex: 1,
    color: theme.colors.foreground,
    fontSize: theme.fontSize.xs,
    fontFamily: Fonts.mono,
  },
  closeButton: {
    padding: 2,
    borderRadius: theme.borderRadius.base,
  },
  panelBody: {
    flex: 1,
    gap: 4,
  },
  infoRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[3],
    flexWrap: "wrap",
  },
  infoItem: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    fontFamily: Fonts.mono,
  },
  infoLabel: {
    color: theme.colors.foreground,
    fontWeight: theme.fontWeight.medium,
  },
  message: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.xs,
    lineHeight: 18,
  },
  refsRow: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: theme.spacing[1],
  },
  compareBanner: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    paddingHorizontal: theme.spacing[2],
    paddingVertical: theme.spacing[1],
    borderRadius: theme.borderRadius.md,
    backgroundColor: theme.colors.surface2,
  },
  compareText: {
    flex: 1,
    minWidth: 0,
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  compareClearButton: {
    padding: 2,
    borderRadius: theme.borderRadius.base,
  },
}));

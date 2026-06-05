import { Copy, GitBranch, GitCommitHorizontal } from "lucide-react-native";
import { type GestureResponderEvent, Pressable, Text, View } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { Fonts } from "@/constants/theme";
import { formatCommitDate } from "@/utils/git-graph-helpers";
import { RefBadge, HiddenRefsBadge } from "./ref-badge";
import type { CommitRowProps } from "./graph-types";

export function CommitRow({
  node,
  layout,
  graph,
  graphWidth,
  showMetadata,
  showAuthor,
  showRefs,
  isMatched,
  hasQuery,
  selected,
  compareSelected,
  onPress,
  onCopyHash,
  onCopyRefs,
}: CommitRowProps) {
  const { theme } = useUnistyles();
  const commit = node.commit;
  const isHead = graph.headCommit === commit.fullHash;
  const branchBadges = showRefs ? commit.branchTips.slice(0, 2) : [];
  const tagBadges = showRefs ? commit.tags.slice(0, 1) : [];
  const hiddenRefCount = Math.max(
    0,
    commit.branchTips.length + commit.tags.length - branchBadges.length - tagBadges.length,
  );

  return (
    <ContextMenu>
      <ContextMenuTrigger
        testID={`commit-graph-row-${commit.hash}`}
        style={({ hovered, pressed, open }) => [
          styles.commitRow,
          { height: layout.rowHeight },
          (hovered || pressed || open) && styles.commitRowHovered,
          selected && styles.commitRowSelected,
          compareSelected && styles.commitRowCompareSelected,
          hasQuery && !isMatched && styles.commitRowDimmed,
          isMatched && styles.commitRowMatched,
        ]}
        onPress={(event) => onPress(commit, event)}
        accessibilityRole="button"
        accessibilityLabel={`Commit ${commit.hash}: ${commit.message}`}
        enabledOnMobile
      >
        {/* Graph column spacer */}
        <View style={{ width: graphWidth }} />

        {/* Message cell */}
        <View style={styles.messageCell}>
          {showRefs && (branchBadges.length > 0 || tagBadges.length > 0) ? (
            <View style={styles.refsInline}>
              {branchBadges.map((branch) => (
                <RefBadge
                  key={`${commit.fullHash}-branch-${branch}`}
                  label={branch}
                  kind="branch"
                />
              ))}
              {tagBadges.map((tag) => (
                <RefBadge key={`${commit.fullHash}-tag-${tag}`} label={tag} kind="tag" />
              ))}
              {hiddenRefCount > 0 ? <HiddenRefsBadge count={hiddenRefCount} /> : null}
            </View>
          ) : null}
          <Text numberOfLines={1} style={styles.commitMessage}>
            {commit.message || "(no commit message)"}
          </Text>
          {isHead ? (
            <View style={styles.headBadge}>
              <Text style={styles.headBadgeText}>HEAD</Text>
            </View>
          ) : null}
        </View>

        {/* Date */}
        {showMetadata ? (
          <View style={styles.dateCell}>
            <Text numberOfLines={1} style={styles.dateText}>
              {formatCommitDate(commit.date)}
            </Text>
          </View>
        ) : null}

        {/* Author */}
        {showAuthor ? (
          <View style={styles.authorCell}>
            <Text numberOfLines={1} style={styles.authorText}>
              {commit.author}
            </Text>
          </View>
        ) : null}

        {/* Hash */}
        {showMetadata ? (
          <Text selectable numberOfLines={1} style={styles.hashText}>
            {commit.hash}
          </Text>
        ) : null}
      </ContextMenuTrigger>

      <ContextMenuContent width={240} testID={`commit-graph-menu-${commit.hash}`}>
        <ContextMenuItem
          testID={`commit-graph-copy-hash-${commit.hash}`}
          leading={<Copy size={15} color={theme.colors.foregroundMuted} />}
          onSelect={() => onCopyHash(commit)}
        >
          复制提交哈希
        </ContextMenuItem>
        <ContextMenuItem
          testID={`commit-graph-copy-refs-${commit.hash}`}
          leading={<GitBranch size={15} color={theme.colors.foregroundMuted} />}
          disabled={commit.branchTips.length === 0 && commit.tags.length === 0}
          onSelect={() => onCopyRefs(commit)}
        >
          复制引用
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem
          testID={`commit-graph-open-files-${commit.hash}`}
          leading={<GitCommitHorizontal size={15} color={theme.colors.foregroundMuted} />}
          disabled
          tooltip="提交文件列表需要 daemon 支持"
        >
          打开变更文件
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}

const styles = StyleSheet.create((theme) => ({
  commitRow: {
    width: "100%",
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: 8,
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
    backgroundColor: theme.colors.background,
  },
  commitRowHovered: {
    backgroundColor: theme.colors.surface1,
  },
  commitRowSelected: {
    backgroundColor: theme.colors.surface2,
  },
  commitRowCompareSelected: {
    borderLeftWidth: 2,
    borderLeftColor: theme.colors.accent,
  },
  commitRowDimmed: {
    opacity: 0.45,
  },
  commitRowMatched: {
    backgroundColor: theme.colors.surface2,
  },
  messageCell: {
    flex: 1,
    minWidth: 0,
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[1],
    paddingRight: theme.spacing[2],
  },
  refsInline: {
    maxWidth: "35%",
    minWidth: 0,
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[1],
    overflow: "hidden",
  },
  commitMessage: {
    flex: 1,
    minWidth: 0,
    color: theme.colors.foreground,
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.normal,
  },
  headBadge: {
    height: 18,
    justifyContent: "center",
    paddingHorizontal: 6,
    borderRadius: theme.borderRadius.base,
    backgroundColor: theme.colors.accent,
  },
  headBadgeText: {
    color: theme.colors.palette.white,
    fontSize: 10,
    fontWeight: theme.fontWeight.medium,
  },
  dateCell: {
    width: 118,
    justifyContent: "center",
    paddingRight: theme.spacing[2],
  },
  dateText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  authorCell: {
    width: 76,
    justifyContent: "center",
    paddingRight: theme.spacing[2],
  },
  authorText: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.xs,
  },
  hashText: {
    width: 70,
    alignSelf: "center",
    color: theme.colors.foregroundMuted,
    fontFamily: Fonts.mono,
    fontSize: theme.fontSize.xs,
  },
}));

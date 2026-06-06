import {
  ChevronDown,
  ChevronUp,
  Copy,
  Crosshair,
  Eye,
  EyeOff,
  GitBranch,
  GitCommitHorizontal,
  RefreshCcw,
  Search,
  X,
} from "lucide-react-native";
import { ActivityIndicator, Pressable, Text, TextInput, View } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { isWeb } from "@/constants/platform";
import type { GraphToolbarProps } from "./graph-types";

function ToolbarButton({
  label,
  onPress,
  children,
  active,
  disabled,
}: {
  label: string;
  onPress?: () => void;
  children: React.ReactNode;
  active?: boolean;
  disabled?: boolean;
}) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      onPress={onPress}
      disabled={disabled}
      style={({ hovered, pressed }) => [
        styles.toolbarButton,
        (hovered || pressed || active) && styles.toolbarButtonActive,
        disabled && styles.toolbarButtonDisabled,
      ]}
    >
      {children}
    </Pressable>
  );
}

export function GraphToolbar({
  title,
  isFetching,
  searchQuery,
  onSearchChange,
  onClearSearch,
  matchCount,
  totalCount,
  onNextMatch,
  onPrevMatch,
  onRefresh,
  onLocateHead,
  showRefs,
  onToggleRefs,
  onClose,
}: GraphToolbarProps) {
  const { theme } = useUnistyles();
  const hasQuery = searchQuery.length > 0;
  const showNav = hasQuery && matchCount > 0;

  return (
    <View style={styles.toolbar}>
      <View style={styles.titleGroup}>
        <GitCommitHorizontal size={16} color={theme.colors.foregroundMuted} />
        <Text style={styles.title} numberOfLines={1}>
          {title}
        </Text>
        {isFetching ? (
          <ActivityIndicator size="small" color={theme.colors.foregroundMuted} />
        ) : null}
      </View>

      <View style={styles.searchBox}>
        <Search size={14} color={theme.colors.foregroundMuted} />
        <TextInput
          value={searchQuery}
          onChangeText={onSearchChange}
          placeholder="搜索提交"
          placeholderTextColor={theme.colors.foregroundMuted}
          style={styles.searchInput}
        />
        {hasQuery ? (
          <View style={styles.searchActions}>
            {showNav ? (
              <View style={styles.matchNav}>
                <Text style={styles.matchCount}>
                  {matchCount}/{totalCount}
                </Text>
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel="上一个匹配"
                  onPress={onPrevMatch}
                  style={styles.navButton}
                >
                  <ChevronUp size={13} color={theme.colors.foregroundMuted} />
                </Pressable>
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel="下一个匹配"
                  onPress={onNextMatch}
                  style={styles.navButton}
                >
                  <ChevronDown size={13} color={theme.colors.foregroundMuted} />
                </Pressable>
              </View>
            ) : null}
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="清除搜索"
              onPress={onClearSearch}
              style={styles.clearSearchButton}
            >
              <X size={13} color={theme.colors.foregroundMuted} />
            </Pressable>
          </View>
        ) : null}
      </View>

      <View style={styles.toolbarActions}>
        <ToolbarButton label="刷新" onPress={onRefresh}>
          <RefreshCcw size={15} color={theme.colors.foregroundMuted} />
        </ToolbarButton>
        <ToolbarButton label="定位 HEAD" onPress={onLocateHead}>
          <Crosshair size={15} color={theme.colors.foregroundMuted} />
        </ToolbarButton>
        <ToolbarButton
          label={showRefs ? "隐藏引用" : "显示引用"}
          onPress={onToggleRefs}
          active={showRefs}
        >
          {showRefs ? (
            <Eye size={15} color={theme.colors.foregroundMuted} />
          ) : (
            <EyeOff size={15} color={theme.colors.foregroundMuted} />
          )}
        </ToolbarButton>
        {onClose ? (
          <ToolbarButton label="关闭" onPress={onClose}>
            <X size={15} color={theme.colors.foregroundMuted} />
          </ToolbarButton>
        ) : null}
      </View>
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  toolbar: {
    minHeight: 38,
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    paddingHorizontal: theme.spacing[2],
    paddingVertical: theme.spacing[1],
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
    backgroundColor: theme.colors.surface1,
  },
  titleGroup: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    minWidth: 0,
    flexShrink: 1,
  },
  title: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.medium,
    flexShrink: 1,
  },
  searchBox: {
    flex: 1,
    minWidth: 120,
    maxWidth: 320,
    height: 28,
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.md,
    backgroundColor: theme.colors.surface0,
    paddingHorizontal: theme.spacing[2],
  },
  searchInput: {
    flex: 1,
    minWidth: 0,
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
    paddingVertical: 0,
    outlineStyle: isWeb ? "none" : undefined,
  } as any,
  searchActions: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[1],
  },
  matchNav: {
    flexDirection: "row",
    alignItems: "center",
    gap: 2,
  },
  matchCount: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    marginRight: 2,
  },
  navButton: {
    padding: 2,
    borderRadius: theme.borderRadius.base,
  },
  clearSearchButton: {
    padding: 2,
    borderRadius: theme.borderRadius.base,
  },
  toolbarActions: {
    flexDirection: "row",
    alignItems: "center",
    flexShrink: 0,
    gap: theme.spacing[1],
  },
  toolbarButton: {
    width: 28,
    height: 28,
    borderRadius: theme.borderRadius.md,
    alignItems: "center",
    justifyContent: "center",
  },
  toolbarButtonActive: {
    backgroundColor: theme.colors.surface2,
  },
  toolbarButtonDisabled: {
    opacity: 0.4,
  },
}));

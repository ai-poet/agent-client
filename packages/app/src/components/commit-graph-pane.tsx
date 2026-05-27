import type { GitGraph, GitGraphCommit } from "@server/shared/messages";
import * as Clipboard from "expo-clipboard";
import {
  Copy,
  Crosshair,
  Eye,
  EyeOff,
  GitBranch,
  GitCommitHorizontal,
  GitCompare,
  RefreshCcw,
  Search,
  Tag,
  X,
} from "lucide-react-native";
import { memo, useCallback, useMemo, useRef, useState } from "react";
import {
  ActivityIndicator,
  type GestureResponderEvent,
  type LayoutChangeEvent,
  Pressable,
  ScrollView,
  type ScrollView as ScrollViewInstance,
  Text,
  TextInput,
  View,
} from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { isWeb } from "@/constants/platform";
import { Fonts } from "@/constants/theme";
import { useToast } from "@/contexts/toast-context";
import { useCommitGraphQuery } from "@/hooks/use-commit-graph-query";
import { type GraphLayout, type GraphNode, layoutGitGraph } from "@/utils/git-graph-layout";

interface CommitGraphPaneProps {
  serverId: string;
  cwd: string;
  onClose?: () => void;
}

const GRAPH_COLUMN_WIDTH = 92;
const MEDIUM_GRAPH_COLUMN_WIDTH = 78;
const NARROW_GRAPH_COLUMN_WIDTH = 64;
const GRAPH_COLUMN_SPACING = 14;
const GRAPH_NODE_RADIUS = 3;
const COMMIT_ROW_HEIGHT = 28;
const DATE_COLUMN_WIDTH = 118;
const AUTHOR_COLUMN_WIDTH = 76;
const HASH_COLUMN_WIDTH = 70;
const ROW_HORIZONTAL_PADDING = 8;
const DETAIL_CARD_HEIGHT = 128;
const COMMIT_LIMIT = 200;

function formatCommitDate(timestamp: number): string {
  const date = new Date(timestamp * 1000);
  if (Number.isNaN(date.getTime())) {
    return "";
  }
  return date.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

function normalizeSearch(value: string): string {
  return value.trim().toLocaleLowerCase();
}

function commitMatchesSearch(commit: GitGraphCommit, query: string): boolean {
  if (!query) {
    return true;
  }
  const haystack = [
    commit.message,
    commit.hash,
    commit.fullHash,
    commit.author,
    commit.authorEmail,
    ...commit.branchTips,
    ...commit.tags,
  ]
    .join(" ")
    .toLocaleLowerCase();
  return haystack.includes(query);
}

function buildFilteredGraph(graph: GitGraph, query: string): GitGraph {
  if (!query) {
    return graph;
  }
  const commits = graph.commits.filter((commit) => commitMatchesSearch(commit, query));
  const visibleHashes = new Set(commits.map((commit) => commit.fullHash));
  return {
    ...graph,
    commits,
    branches: graph.branches.filter((branch) => visibleHashes.has(branch.tipCommit)),
    rootCommits: graph.rootCommits.filter((hash) => visibleHashes.has(hash)),
    headCommit: graph.headCommit && visibleHashes.has(graph.headCommit) ? graph.headCommit : null,
  };
}

function visibleRefLabels(commit: GitGraphCommit, showRefs: boolean): string[] {
  if (!showRefs) {
    return [];
  }
  return [...commit.branchTips, ...commit.tags];
}

function ToolbarButton({
  label,
  onPress,
  children,
  active,
}: {
  label: string;
  onPress?: () => void;
  children: React.ReactNode;
  active?: boolean;
}) {
  const { theme } = useUnistyles();
  return (
    <Tooltip delayDuration={0} enabledOnDesktop enabledOnMobile={false}>
      <TooltipTrigger asChild>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={label}
          onPress={onPress}
          style={({ hovered, pressed }) => [
            styles.toolbarButton,
            (hovered || pressed || active) && styles.toolbarButtonActive,
          ]}
          testID={`commit-graph-${label.toLocaleLowerCase().replaceAll(" ", "-")}`}
        >
          {children}
        </Pressable>
      </TooltipTrigger>
      <TooltipContent side="bottom" align="center" offset={8}>
        <Text style={styles.tooltipText}>{label}</Text>
      </TooltipContent>
    </Tooltip>
  );
}

function RefBadge({ label, kind }: { label: string; kind: "branch" | "tag" }) {
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

function HiddenRefsBadge({ count }: { count: number }) {
  return (
    <View style={styles.hiddenRefsBadge}>
      <Text style={styles.hiddenRefsText}>+{count}</Text>
    </View>
  );
}

function CommitCard({
  commit,
  compareCommit,
  onClearCompare,
  graphWidth,
}: {
  commit: GitGraphCommit;
  compareCommit: GitGraphCommit | null;
  onClearCompare: () => void;
  graphWidth: number;
}) {
  const { theme } = useUnistyles();
  return (
    <View style={[styles.commitCardRow, { minHeight: DETAIL_CARD_HEIGHT }]}>
      <View style={{ width: graphWidth }} />
      <View style={styles.commitCard} testID="commit-graph-detail">
        <View style={styles.commitCardMain}>
          <Text selectable numberOfLines={1} style={styles.detailLine}>
            <Text style={styles.detailLabel}>Commit: </Text>
            {commit.fullHash}
          </Text>
          <Text selectable numberOfLines={1} style={styles.detailLine}>
            <Text style={styles.detailLabel}>Parents: </Text>
            {commit.parents.length > 0 ? commit.parents.join(" ") : "None"}
          </Text>
          <Text numberOfLines={1} style={styles.detailLine}>
            <Text style={styles.detailLabel}>Author: </Text>
            {commit.author} &lt;{commit.authorEmail}&gt;
          </Text>
          <Text numberOfLines={1} style={styles.detailLine}>
            <Text style={styles.detailLabel}>Date: </Text>
            {formatCommitDate(commit.date)}
          </Text>

          <Text numberOfLines={2} style={styles.detailMessage}>
            {commit.message || "(no commit message)"}
          </Text>

          {compareCommit ? (
            <View style={styles.compareBanner} testID="commit-graph-compare-banner">
              <GitCompare size={14} color={theme.colors.foregroundMuted} />
              <Text style={styles.compareBannerText} numberOfLines={1}>
                Comparing {compareCommit.hash} to {commit.hash}
              </Text>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="Clear compare"
                onPress={onClearCompare}
                style={styles.compareClearButton}
              >
                <X size={14} color={theme.colors.foregroundMuted} />
              </Pressable>
            </View>
          ) : null}
        </View>

        <View style={styles.commitCardAside}>
          <View style={styles.detailRefs}>
            {commit.branchTips.slice(0, 4).map((branch) => (
              <RefBadge key={`branch-${branch}`} label={branch} kind="branch" />
            ))}
            {commit.tags.slice(0, 3).map((tagName) => (
              <RefBadge key={`tag-${tagName}`} label={tagName} kind="tag" />
            ))}
            {commit.branchTips.length + commit.tags.length > 7 ? (
              <HiddenRefsBadge count={commit.branchTips.length + commit.tags.length - 7} />
            ) : null}
          </View>
          <View style={styles.changedFilesStub}>
            <Text style={styles.changedFilesTitle}>Changed files</Text>
            <Text style={styles.changedFilesText}>
              File list is not available from this daemon yet.
            </Text>
          </View>
        </View>
      </View>
    </View>
  );
}

interface GraphLayerProps {
  layout: GraphLayout;
  nodes: GraphNode[];
  graphWidth: number;
  graphHeight: number;
  selectedHash: string | null;
  headHash: string | null;
}

function GraphLayer({
  layout,
  nodes,
  graphWidth,
  graphHeight,
  selectedHash,
  headHash,
}: GraphLayerProps) {
  const { theme } = useUnistyles();
  const displayNodes = useMemo(() => {
    const selectedIndex = selectedHash
      ? nodes.findIndex((node) => node.commit.fullHash === selectedHash)
      : -1;
    return nodes.map((node, index) => ({
      ...node,
      y: selectedIndex >= 0 && index > selectedIndex ? node.y + DETAIL_CARD_HEIGHT : node.y,
    }));
  }, [nodes, selectedHash]);
  const nodeByHash = useMemo(() => {
    const map = new Map<string, GraphNode>();
    for (const node of displayNodes) {
      map.set(node.commit.fullHash, node);
    }
    return map;
  }, [displayNodes]);

  return (
    <View
      pointerEvents="none"
      style={[styles.graphLayer, { width: graphWidth, height: graphHeight }]}
    >
      {layout.edges.map((edge) => {
        const fromNode = nodeByHash.get(edge.from);
        const toNode = nodeByHash.get(edge.to);
        if (!fromNode || !toNode) {
          return null;
        }

        const x1 = fromNode.x;
        const x2 = toNode.x;
        const y1 = fromNode.y;
        const y2 = toNode.y;
        const minX = Math.min(x1, x2);
        const width = Math.max(1, Math.abs(x2 - x1));
        const midY = y1 + Math.max(8, Math.min(18, (y2 - y1) / 2));

        if (x1 === x2) {
          return (
            <View
              key={`${edge.from}-${edge.to}`}
              pointerEvents="none"
              style={[
                styles.graphLine,
                {
                  backgroundColor: edge.color,
                  left: x1 - 1,
                  top: y1,
                  height: Math.max(y2 - y1, 1),
                  width: 1.5,
                },
              ]}
            />
          );
        }

        return (
          <View key={`${edge.from}-${edge.to}`} pointerEvents="none">
            <View
              style={[
                styles.graphLine,
                {
                  backgroundColor: edge.color,
                  left: x1 - 1,
                  top: y1,
                  height: Math.max(midY - y1, 1),
                  width: 1.5,
                },
              ]}
            />
            <View
              style={[
                styles.graphLine,
                {
                  backgroundColor: edge.color,
                  left: minX,
                  top: midY - 1,
                  width,
                  height: 1.5,
                },
              ]}
            />
            <View
              style={[
                styles.graphLine,
                {
                  backgroundColor: edge.color,
                  left: x2 - 1,
                  top: midY,
                  height: Math.max(y2 - midY, 1),
                  width: 1.5,
                },
              ]}
            />
          </View>
        );
      })}

      {displayNodes.map((node) => {
        const selected = selectedHash === node.commit.fullHash;
        const isHead = headHash === node.commit.fullHash;
        return (
          <View key={node.commit.fullHash} pointerEvents="none">
            <View
              style={[
                styles.graphNode,
                {
                  left: node.x - layout.nodeRadius,
                  top: node.y - layout.nodeRadius,
                  width: layout.nodeRadius * 2,
                  height: layout.nodeRadius * 2,
                  borderRadius: layout.nodeRadius,
                  borderColor: node.color,
                  backgroundColor: selected ? node.color : theme.colors.background,
                },
                selected && styles.graphNodeSelected,
              ]}
            />
            {isHead ? (
              <View
                style={[
                  styles.headRing,
                  {
                    left: node.x - layout.nodeRadius - 3,
                    top: node.y - layout.nodeRadius - 3,
                    width: layout.nodeRadius * 2 + 6,
                    height: layout.nodeRadius * 2 + 6,
                    borderRadius: layout.nodeRadius + 3,
                    borderColor: node.color,
                  },
                ]}
              />
            ) : null}
          </View>
        );
      })}
    </View>
  );
}

interface CommitRowProps {
  node: GraphNode;
  layout: GraphLayout;
  graph: GitGraph;
  graphWidth: number;
  showMetadata: boolean;
  showAuthor: boolean;
  showRefs: boolean;
  selected: boolean;
  compareSelected: boolean;
  onPress: (commit: GitGraphCommit, event: GestureResponderEvent) => void;
  onCopyHash: (commit: GitGraphCommit) => void;
  onCopyRefs: (commit: GitGraphCommit) => void;
}

function CommitRow({
  node,
  layout,
  graph,
  graphWidth,
  showMetadata,
  showAuthor,
  showRefs,
  selected,
  compareSelected,
  onPress,
  onCopyHash,
  onCopyRefs,
}: CommitRowProps) {
  const { theme } = useUnistyles();
  const isHead = graph.headCommit === node.commit.fullHash;
  const refs = visibleRefLabels(node.commit, showRefs);
  const branchBadges = showRefs ? node.commit.branchTips.slice(0, 2) : [];
  const tagBadges = showRefs ? node.commit.tags.slice(0, 1) : [];
  const hiddenRefCount = Math.max(0, refs.length - branchBadges.length - tagBadges.length);

  return (
    <ContextMenu>
      <ContextMenuTrigger
        testID={`commit-graph-row-${node.commit.hash}`}
        style={({ hovered, pressed, open }) => [
          styles.commitRow,
          { height: layout.rowHeight },
          (hovered || pressed || open) && styles.commitRowHovered,
          selected && styles.commitRowSelected,
          compareSelected && styles.commitRowCompareSelected,
        ]}
        onPress={(event) => onPress(node.commit, event)}
        accessibilityRole="button"
        accessibilityLabel={`Commit ${node.commit.hash}: ${node.commit.message}`}
        enabledOnMobile
      >
        <View style={{ width: graphWidth }} />

        <View style={styles.messageCell}>
          {refs.length > 0 ? (
            <View style={styles.refsInline}>
              {branchBadges.map((branch) => (
                <RefBadge
                  key={`${node.commit.fullHash}-branch-${branch}`}
                  label={branch}
                  kind="branch"
                />
              ))}
              {tagBadges.map((tagName) => (
                <RefBadge
                  key={`${node.commit.fullHash}-tag-${tagName}`}
                  label={tagName}
                  kind="tag"
                />
              ))}
              {hiddenRefCount > 0 ? <HiddenRefsBadge count={hiddenRefCount} /> : null}
            </View>
          ) : null}
          <Text numberOfLines={1} style={styles.commitMessage}>
            {node.commit.message || "(no commit message)"}
          </Text>
          {isHead ? (
            <View style={styles.headBadge}>
              <Text style={styles.headBadgeText}>HEAD</Text>
            </View>
          ) : null}
        </View>

        {showMetadata ? (
          <View style={styles.dateCell}>
            <Text numberOfLines={1} style={styles.dateText}>
              {formatCommitDate(node.commit.date)}
            </Text>
          </View>
        ) : null}

        {showAuthor ? (
          <View style={styles.authorCell}>
            <Text numberOfLines={1} style={styles.authorText}>
              {node.commit.author}
            </Text>
          </View>
        ) : null}

        {showMetadata ? (
          <Text selectable numberOfLines={1} style={styles.hashText}>
            {node.commit.hash}
          </Text>
        ) : null}
      </ContextMenuTrigger>
      <ContextMenuContent width={240} testID={`commit-graph-menu-${node.commit.hash}`}>
        <ContextMenuItem
          testID={`commit-graph-copy-hash-${node.commit.hash}`}
          leading={<Copy size={15} color={theme.colors.foregroundMuted} />}
          onSelect={() => onCopyHash(node.commit)}
        >
          Copy commit hash
        </ContextMenuItem>
        <ContextMenuItem
          testID={`commit-graph-copy-refs-${node.commit.hash}`}
          leading={<GitBranch size={15} color={theme.colors.foregroundMuted} />}
          disabled={node.commit.branchTips.length === 0 && node.commit.tags.length === 0}
          onSelect={() => onCopyRefs(node.commit)}
        >
          Copy refs
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem
          testID={`commit-graph-open-files-${node.commit.hash}`}
          leading={<GitCommitHorizontal size={15} color={theme.colors.foregroundMuted} />}
          disabled
          tooltip="Commit file list needs daemon support"
        >
          Open changed files
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}

export const CommitGraphPane = memo(function CommitGraphPane({
  serverId,
  cwd,
  onClose,
}: CommitGraphPaneProps) {
  const { graph, isLoading, isFetching, isError, error, refetch } = useCommitGraphQuery({
    serverId,
    cwd,
    limit: COMMIT_LIMIT,
  });
  const { theme } = useUnistyles();
  const toast = useToast();
  const scrollRef = useRef<ScrollViewInstance | null>(null);
  const [selectedCommit, setSelectedCommit] = useState<GitGraphCommit | null>(null);
  const [compareCommit, setCompareCommit] = useState<GitGraphCommit | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [showRefs, setShowRefs] = useState(true);
  const [containerWidth, setContainerWidth] = useState(0);

  const normalizedSearchQuery = normalizeSearch(searchQuery);
  const filteredGraph = useMemo(() => {
    if (!graph) {
      return null;
    }
    return buildFilteredGraph(graph, normalizedSearchQuery);
  }, [graph, normalizedSearchQuery]);

  const layout = useMemo(() => {
    if (!filteredGraph || filteredGraph.commits.length === 0) return null;
    return layoutGitGraph(filteredGraph, {
      isDark: theme.colorScheme === "dark",
      columnWidth: GRAPH_COLUMN_SPACING,
      rowHeight: COMMIT_ROW_HEIGHT,
      nodeRadius: GRAPH_NODE_RADIUS,
    });
  }, [filteredGraph, theme.colorScheme]);

  const showMetadata = containerWidth >= 540;
  const showAuthor = containerWidth >= 680;
  const graphWidth =
    containerWidth > 0 && containerWidth < 420
      ? NARROW_GRAPH_COLUMN_WIDTH
      : containerWidth > 0 && containerWidth < 560
        ? MEDIUM_GRAPH_COLUMN_WIDTH
        : GRAPH_COLUMN_WIDTH;
  const graphHeight = layout
    ? layout.height +
      (selectedCommit &&
      layout.nodes.some((node) => node.commit.fullHash === selectedCommit.fullHash)
        ? DETAIL_CARD_HEIGHT
        : 0)
    : 0;
  const headIndex = useMemo(() => {
    if (!filteredGraph?.headCommit) {
      return -1;
    }
    return filteredGraph.commits.findIndex(
      (commit) => commit.fullHash === filteredGraph.headCommit,
    );
  }, [filteredGraph]);

  const handleCommitPress = useCallback((commit: GitGraphCommit, event: GestureResponderEvent) => {
    const nativeEvent = event.nativeEvent as GestureResponderEvent["nativeEvent"] & {
      metaKey?: boolean;
      ctrlKey?: boolean;
    };
    const wantsCompare = Boolean(nativeEvent.metaKey || nativeEvent.ctrlKey);
    if (wantsCompare) {
      setCompareCommit((previous) => (previous?.fullHash === commit.fullHash ? null : commit));
      setSelectedCommit(commit);
      return;
    }
    setSelectedCommit((previous) => (previous?.fullHash === commit.fullHash ? null : commit));
  }, []);

  const handleCopyHash = useCallback(
    async (commit: GitGraphCommit) => {
      try {
        await Clipboard.setStringAsync(commit.fullHash);
        toast.copied("commit hash");
      } catch {
        toast.error("Failed to copy commit hash");
      }
    },
    [toast],
  );

  const handleCopyRefs = useCallback(
    async (commit: GitGraphCommit) => {
      const refs = [...commit.branchTips, ...commit.tags].join("\n");
      if (!refs) {
        return;
      }
      try {
        await Clipboard.setStringAsync(refs);
        toast.copied("refs");
      } catch {
        toast.error("Failed to copy refs");
      }
    },
    [toast],
  );

  const handleRefresh = useCallback(() => {
    void refetch();
  }, [refetch]);

  const handleLocateHead = useCallback(() => {
    if (headIndex < 0 || !layout) {
      return;
    }
    scrollRef.current?.scrollTo({ y: headIndex * layout.rowHeight, animated: true });
  }, [headIndex, layout]);

  const handleClearSearch = useCallback(() => {
    setSearchQuery("");
  }, []);

  const handleContainerLayout = useCallback((event: LayoutChangeEvent) => {
    const nextWidth = event.nativeEvent.layout.width;
    setContainerWidth((previous) => (Math.abs(previous - nextWidth) > 1 ? nextWidth : previous));
  }, []);

  if (isLoading) {
    return (
      <View style={styles.center} testID="commit-graph-loading">
        <ActivityIndicator size="large" color={theme.colors.foregroundMuted} />
      </View>
    );
  }

  if (isError || !graph) {
    const errorMessage = error instanceof Error ? error.message : "Failed to load commit graph";
    return (
      <View style={styles.center} testID="commit-graph-error">
        <Text style={styles.errorText}>{errorMessage}</Text>
      </View>
    );
  }

  const hasCommits = graph.commits.length > 0;
  const visibleCommits = filteredGraph?.commits ?? [];

  return (
    <View style={styles.container} testID="commit-graph-pane" onLayout={handleContainerLayout}>
      <View style={styles.toolbar}>
        <View style={styles.titleGroup}>
          <GitCommitHorizontal size={16} color={theme.colors.foregroundMuted} />
          <Text style={styles.title}>Git Graph</Text>
          {isFetching ? (
            <ActivityIndicator size="small" color={theme.colors.foregroundMuted} />
          ) : null}
        </View>

        <View style={styles.searchBox}>
          <Search size={14} color={theme.colors.foregroundMuted} />
          <TextInput
            value={searchQuery}
            onChangeText={setSearchQuery}
            placeholder="Search commits"
            placeholderTextColor={theme.colors.foregroundMuted}
            style={styles.searchInput}
            testID="commit-graph-search"
          />
          {searchQuery ? (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="Clear commit graph search"
              onPress={handleClearSearch}
              style={styles.clearSearchButton}
            >
              <X size={13} color={theme.colors.foregroundMuted} />
            </Pressable>
          ) : null}
        </View>

        <View style={styles.toolbarActions}>
          <ToolbarButton label="Refresh" onPress={handleRefresh}>
            <RefreshCcw size={15} color={theme.colors.foregroundMuted} />
          </ToolbarButton>
          <ToolbarButton label="Locate HEAD" onPress={handleLocateHead}>
            <Crosshair size={15} color={theme.colors.foregroundMuted} />
          </ToolbarButton>
          <ToolbarButton
            label={showRefs ? "Hide refs" : "Show refs"}
            onPress={() => setShowRefs((value) => !value)}
            active={showRefs}
          >
            {showRefs ? (
              <Eye size={15} color={theme.colors.foregroundMuted} />
            ) : (
              <EyeOff size={15} color={theme.colors.foregroundMuted} />
            )}
          </ToolbarButton>
          {onClose ? (
            <ToolbarButton label="Close" onPress={onClose}>
              <X size={15} color={theme.colors.foregroundMuted} />
            </ToolbarButton>
          ) : null}
        </View>
      </View>

      {!hasCommits ? (
        <View style={styles.center} testID="commit-graph-empty">
          <GitCommitHorizontal size={42} color={theme.colors.foregroundMuted} />
          <Text style={styles.emptyText}>No commits found</Text>
        </View>
      ) : !layout || !filteredGraph || visibleCommits.length === 0 ? (
        <View style={styles.center} testID="commit-graph-no-search-results">
          <Search size={36} color={theme.colors.foregroundMuted} />
          <Text style={styles.emptyText}>No matching commits</Text>
        </View>
      ) : (
        <>
          <View style={styles.tableHeader}>
            <View style={{ width: graphWidth }} />
            <Text style={[styles.columnHeader, styles.descriptionHeader]}>Description</Text>
            {showMetadata ? (
              <Text style={[styles.columnHeader, { width: DATE_COLUMN_WIDTH }]}>Date</Text>
            ) : null}
            {showAuthor ? (
              <Text style={[styles.columnHeader, { width: AUTHOR_COLUMN_WIDTH }]}>Author</Text>
            ) : null}
            {showMetadata ? (
              <Text style={[styles.columnHeader, { width: HASH_COLUMN_WIDTH }]}>Commit</Text>
            ) : null}
          </View>
          <ScrollView
            ref={scrollRef}
            style={styles.scrollView}
            showsVerticalScrollIndicator={false}
            contentContainerStyle={styles.scrollContent}
          >
            <View style={[styles.graphTable, { minHeight: graphHeight }]}>
              <GraphLayer
                layout={layout}
                nodes={layout.nodes}
                graphWidth={graphWidth}
                graphHeight={graphHeight}
                selectedHash={selectedCommit?.fullHash ?? null}
                headHash={filteredGraph.headCommit}
              />
              {layout.nodes.map((node) => (
                <View key={node.commit.fullHash}>
                  <CommitRow
                    node={node}
                    layout={layout}
                    graph={filteredGraph}
                    graphWidth={graphWidth}
                    showMetadata={showMetadata}
                    showAuthor={showAuthor}
                    showRefs={showRefs}
                    selected={selectedCommit?.fullHash === node.commit.fullHash}
                    compareSelected={compareCommit?.fullHash === node.commit.fullHash}
                    onPress={handleCommitPress}
                    onCopyHash={handleCopyHash}
                    onCopyRefs={handleCopyRefs}
                  />
                  {selectedCommit?.fullHash === node.commit.fullHash ? (
                    <CommitCard
                      commit={selectedCommit}
                      compareCommit={
                        compareCommit && compareCommit.fullHash !== selectedCommit.fullHash
                          ? compareCommit
                          : null
                      }
                      onClearCompare={() => setCompareCommit(null)}
                      graphWidth={graphWidth}
                    />
                  ) : null}
                </View>
              ))}
            </View>
          </ScrollView>
        </>
      )}
    </View>
  );
});

const styles = StyleSheet.create((theme) => ({
  container: {
    flex: 1,
    minHeight: 0,
    backgroundColor: theme.colors.background,
  },
  center: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    gap: theme.spacing[3],
    padding: theme.spacing[6],
  },
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
    maxWidth: 260,
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
  tableHeader: {
    height: 26,
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: ROW_HORIZONTAL_PADDING,
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
    backgroundColor: theme.colors.surface0,
  },
  columnHeader: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.medium,
    textTransform: "uppercase",
  },
  descriptionHeader: {
    flex: 1,
    minWidth: 0,
  },
  scrollView: {
    flex: 1,
    minHeight: 0,
  },
  scrollContent: {
    flexGrow: 1,
  },
  graphTable: {
    position: "relative",
    overflow: "hidden",
  },
  commitRow: {
    width: "100%",
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: ROW_HORIZONTAL_PADDING,
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
  graphLayer: {
    position: "absolute",
    left: ROW_HORIZONTAL_PADDING,
    top: 0,
    zIndex: 1,
    overflow: "hidden",
  },
  graphLine: {
    position: "absolute",
    opacity: 0.95,
  },
  graphNode: {
    position: "absolute",
    borderWidth: 1.5,
    zIndex: 3,
  },
  graphNodeSelected: {
    borderWidth: 2,
  },
  headRing: {
    position: "absolute",
    borderWidth: 1.5,
    zIndex: 2,
  },
  messageCell: {
    flex: 1,
    minWidth: 0,
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[1],
    paddingRight: theme.spacing[2],
  },
  commitMessage: {
    flex: 1,
    minWidth: 0,
    color: theme.colors.foreground,
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.normal,
  },
  refsInline: {
    maxWidth: "35%",
    minWidth: 0,
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[1],
    overflow: "hidden",
  },
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
    width: DATE_COLUMN_WIDTH,
    justifyContent: "center",
    paddingRight: theme.spacing[2],
  },
  authorCell: {
    width: AUTHOR_COLUMN_WIDTH,
    justifyContent: "center",
    paddingRight: theme.spacing[2],
  },
  authorText: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.xs,
  },
  dateText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  hashText: {
    width: HASH_COLUMN_WIDTH,
    alignSelf: "center",
    color: theme.colors.foregroundMuted,
    fontFamily: Fonts.mono,
    fontSize: theme.fontSize.xs,
  },
  commitCardRow: {
    flexDirection: "row",
    paddingHorizontal: ROW_HORIZONTAL_PADDING,
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
    backgroundColor: theme.colors.background,
  },
  commitCard: {
    flex: 1,
    minWidth: 0,
    flexDirection: "row",
    flexWrap: "wrap",
    gap: theme.spacing[3],
    marginVertical: theme.spacing[2],
    padding: theme.spacing[2],
    borderWidth: 1,
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.surface2,
  },
  commitCardMain: {
    flex: 1,
    minWidth: 0,
    gap: 3,
  },
  commitCardAside: {
    width: 220,
    maxWidth: "100%",
    gap: theme.spacing[2],
  },
  detailRefs: {
    minHeight: 22,
    flexDirection: "row",
    flexWrap: "wrap",
    gap: theme.spacing[1],
  },
  detailLine: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    fontFamily: Fonts.mono,
  },
  detailLabel: {
    color: theme.colors.foreground,
    fontWeight: theme.fontWeight.medium,
  },
  detailMessage: {
    marginTop: theme.spacing[3],
    color: theme.colors.foreground,
    fontSize: theme.fontSize.xs,
    lineHeight: 18,
  },
  changedFilesStub: {
    flex: 1,
    padding: theme.spacing[2],
    borderLeftWidth: 1,
    borderLeftColor: theme.colors.border,
  },
  changedFilesTitle: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.medium,
  },
  changedFilesText: {
    marginTop: theme.spacing[1],
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  compareBanner: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    paddingHorizontal: theme.spacing[2],
    paddingVertical: theme.spacing[2],
    borderRadius: theme.borderRadius.md,
    backgroundColor: theme.colors.surface2,
  },
  compareBannerText: {
    flex: 1,
    minWidth: 0,
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  compareClearButton: {
    padding: 2,
    borderRadius: theme.borderRadius.base,
  },
  errorText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    textAlign: "center",
  },
  emptyText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    textAlign: "center",
  },
  tooltipText: {
    color: theme.colors.popoverForeground,
    fontSize: theme.fontSize.sm,
  },
}));

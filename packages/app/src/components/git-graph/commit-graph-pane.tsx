import * as Clipboard from "expo-clipboard";
import { memo, useCallback, useMemo, useRef, useState } from "react";
import {
  ActivityIndicator,
  type GestureResponderEvent,
  ScrollView,
  type ScrollView as ScrollViewInstance,
  Text,
  View,
} from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { useToast } from "@/contexts/toast-context";
import type { GitGraphCommit } from "@server/shared/messages";
import { useCommitGraphQuery } from "@/hooks/use-commit-graph-query";
import { layoutGitGraph } from "@/utils/git-graph-layout";
import { buildSearchMatches, normalizeSearch } from "@/utils/git-graph-helpers";
import { GraphToolbar } from "./graph-toolbar";
import { GraphSvgLayer } from "./graph-svg-layer";
import { CommitRow } from "./commit-row";
import { CommitDetailPanel } from "./commit-detail-panel";
import type { CommitGraphPaneProps } from "./graph-types";
import {
  GRAPH_COLUMN_WIDTH,
  MEDIUM_GRAPH_COLUMN_WIDTH,
  NARROW_GRAPH_COLUMN_WIDTH,
  GRAPH_NODE_RADIUS,
  COMMIT_ROW_HEIGHT,
  COMMIT_LIMIT,
} from "./graph-types";

function useGraphConfig(containerWidth: number) {
  return useMemo(() => {
    if (containerWidth < 320) {
      return {
        graphWidth: NARROW_GRAPH_COLUMN_WIDTH,
        showMetadata: false,
        showAuthor: false,
        showRefs: false,
        nodeRadius: 4,
      };
    }
    if (containerWidth < 420) {
      return {
        graphWidth: NARROW_GRAPH_COLUMN_WIDTH,
        showMetadata: false,
        showAuthor: false,
        showRefs: true,
        nodeRadius: 4,
      };
    }
    if (containerWidth < 560) {
      return {
        graphWidth: MEDIUM_GRAPH_COLUMN_WIDTH,
        showMetadata: true,
        showAuthor: false,
        showRefs: true,
        nodeRadius: GRAPH_NODE_RADIUS,
      };
    }
    return {
      graphWidth: GRAPH_COLUMN_WIDTH,
      showMetadata: true,
      showAuthor: true,
      showRefs: true,
      nodeRadius: GRAPH_NODE_RADIUS,
    };
  }, [containerWidth]);
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
  const [selectedCommit, setSelectedCommit] = useState<string | null>(null);
  const [compareCommit, setCompareCommit] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [showRefs, setShowRefs] = useState(true);
  const [containerWidth, setContainerWidth] = useState(0);
  const [matchIndex, setMatchIndex] = useState(0);

  const config = useGraphConfig(containerWidth);

  const normalizedQuery = normalizeSearch(searchQuery);
  const matchedHashes = useMemo(() => {
    if (!graph || !normalizedQuery) return new Set<string>();
    return buildSearchMatches(graph, normalizedQuery);
  }, [graph, normalizedQuery]);

  const matchArray = useMemo(() => Array.from(matchedHashes), [matchedHashes]);

  const layout = useMemo(() => {
    if (!graph || graph.commits.length === 0) return null;
    return layoutGitGraph(graph, {
      isDark: theme.colorScheme === "dark",
      columnWidth: 14,
      rowHeight: COMMIT_ROW_HEIGHT,
      nodeRadius: config.nodeRadius,
    });
  }, [graph, theme.colorScheme, config.nodeRadius]);

  const graphHeight = layout ? layout.height : 0;

  const headIndex = useMemo(() => {
    if (!graph?.headCommit) return -1;
    return graph.commits.findIndex((c) => c.fullHash === graph.headCommit);
  }, [graph]);

  const selectedCommitData = useMemo(() => {
    if (!selectedCommit || !graph) return null;
    return graph.commits.find((c) => c.fullHash === selectedCommit) ?? null;
  }, [selectedCommit, graph]);

  const compareCommitData = useMemo(() => {
    if (!compareCommit || !graph) return null;
    return graph.commits.find((c) => c.fullHash === compareCommit) ?? null;
  }, [compareCommit, graph]);

  const handleCommitPress = useCallback((commit: GitGraphCommit, event: GestureResponderEvent) => {
    const nativeEvent = event.nativeEvent as GestureResponderEvent["nativeEvent"] & {
      metaKey?: boolean;
      ctrlKey?: boolean;
    };
    const wantsCompare = Boolean(nativeEvent.metaKey || nativeEvent.ctrlKey);
    if (wantsCompare) {
      setCompareCommit((prev) => (prev === commit.fullHash ? null : commit.fullHash));
      setSelectedCommit(commit.fullHash);
      return;
    }
    setSelectedCommit((prev) => (prev === commit.fullHash ? null : commit.fullHash));
  }, []);

  const handleCopyHash = useCallback(
    async (commit: { fullHash: string }) => {
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
    async (commit: { branchTips: string[]; tags: string[] }) => {
      const refs = [...commit.branchTips, ...commit.tags].join("\n");
      if (!refs) return;
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
    if (headIndex < 0 || !layout) return;
    scrollRef.current?.scrollTo({
      y: headIndex * layout.rowHeight,
      animated: true,
    });
  }, [headIndex, layout]);

  const navigateMatch = useCallback(
    (direction: 1 | -1) => {
      if (matchArray.length === 0) return;
      const next = (matchIndex + direction + matchArray.length) % matchArray.length;
      setMatchIndex(next);
      const targetHash = matchArray[next];
      const targetRow = graph?.commits.findIndex((c) => c.fullHash === targetHash);
      if (targetRow !== undefined && targetRow >= 0 && layout) {
        scrollRef.current?.scrollTo({
          y: targetRow * layout.rowHeight - 40,
          animated: true,
        });
      }
    },
    [matchArray, matchIndex, graph, layout],
  );

  const handleContainerLayout = useCallback(
    (event: { nativeEvent: { layout: { width: number } } }) => {
      const nextWidth = event.nativeEvent.layout.width;
      setContainerWidth((prev) => (Math.abs(prev - nextWidth) > 1 ? nextWidth : prev));
    },
    [],
  );

  if (isLoading) {
    return (
      <View style={styles.center} testID="commit-graph-loading">
        <ActivityIndicator size="large" color={theme.colors.foregroundMuted} />
      </View>
    );
  }

  if (isError || !graph) {
    const errorMessage = error instanceof Error ? error.message : "加载提交图谱失败";
    return (
      <View style={styles.center} testID="commit-graph-error">
        <Text style={styles.errorText}>{errorMessage}</Text>
      </View>
    );
  }

  const hasCommits = graph.commits.length > 0;

  return (
    <View style={styles.container} testID="commit-graph-pane" onLayout={handleContainerLayout}>
      <GraphToolbar
        title="Git 图谱"
        isFetching={isFetching}
        searchQuery={searchQuery}
        onSearchChange={setSearchQuery}
        onClearSearch={() => {
          setSearchQuery("");
          setMatchIndex(0);
        }}
        matchCount={matchArray.length}
        totalCount={graph.commits.length}
        onNextMatch={matchArray.length > 0 ? () => navigateMatch(1) : undefined}
        onPrevMatch={matchArray.length > 0 ? () => navigateMatch(-1) : undefined}
        onRefresh={handleRefresh}
        onLocateHead={handleLocateHead}
        showRefs={showRefs}
        onToggleRefs={() => setShowRefs((v) => !v)}
        onClose={onClose}
      />

      {!hasCommits ? (
        <View style={styles.center} testID="commit-graph-empty">
          <Text style={styles.emptyText}>没有提交</Text>
        </View>
      ) : (
        <>
          {/* Table header */}
          <View style={styles.tableHeader}>
            <View style={{ width: config.graphWidth }} />
            <Text style={[styles.columnHeader, styles.descriptionHeader]}>描述</Text>
            {config.showMetadata ? (
              <Text style={[styles.columnHeader, { width: 118 }]}>日期</Text>
            ) : null}
            {config.showAuthor ? (
              <Text style={[styles.columnHeader, { width: 76 }]}>作者</Text>
            ) : null}
            {config.showMetadata ? (
              <Text style={[styles.columnHeader, { width: 70 }]}>提交</Text>
            ) : null}
          </View>

          {/* Scrollable commit list */}
          <ScrollView
            ref={scrollRef}
            style={styles.scrollView}
            showsVerticalScrollIndicator={false}
            contentContainerStyle={styles.scrollContent}
          >
            <View style={[styles.graphTable, { minHeight: graphHeight }]}>
              {/* SVG graph layer */}
              {layout ? (
                <View
                  style={[
                    styles.graphLayerContainer,
                    { width: config.graphWidth, height: graphHeight },
                  ]}
                >
                  <GraphSvgLayer
                    layout={layout}
                    selectedHash={selectedCommit}
                    headHash={graph.headCommit}
                  />
                </View>
              ) : null}

              {/* Commit rows */}
              {layout?.nodes.map((node) => (
                <CommitRow
                  key={node.commit.fullHash}
                  node={node}
                  layout={layout}
                  graph={graph}
                  graphWidth={config.graphWidth}
                  showMetadata={config.showMetadata}
                  showAuthor={config.showAuthor}
                  showRefs={showRefs}
                  isMatched={matchedHashes.has(node.commit.fullHash)}
                  hasQuery={normalizedQuery.length > 0}
                  selected={selectedCommit === node.commit.fullHash}
                  compareSelected={compareCommit === node.commit.fullHash}
                  onPress={handleCommitPress}
                  onCopyHash={handleCopyHash}
                  onCopyRefs={handleCopyRefs}
                />
              ))}
            </View>
          </ScrollView>

          {/* Detail panel */}
          {selectedCommitData ? (
            <CommitDetailPanel
              commit={selectedCommitData}
              compareCommit={
                compareCommitData && compareCommitData.fullHash !== selectedCommitData.fullHash
                  ? compareCommitData
                  : null
              }
              onClearCompare={() => setCompareCommit(null)}
              onClose={() => setSelectedCommit(null)}
            />
          ) : null}
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
  tableHeader: {
    height: 26,
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: 8,
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
  graphLayerContainer: {
    position: "absolute",
    left: 8,
    top: 0,
    zIndex: 1,
    overflow: "hidden",
  },
}));

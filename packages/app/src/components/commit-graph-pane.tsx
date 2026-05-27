import { useState, useCallback, useMemo, memo } from "react";
import { View, Text, Pressable, ScrollView, ActivityIndicator } from "react-native";
import Svg, { Path, Circle, G, Text as SvgText, Line } from "react-native-svg";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { GitCommitHorizontal, GitBranch } from "lucide-react-native";
import { useCommitGraphQuery } from "@/hooks/use-commit-graph-query";
import { layoutGitGraph } from "@/utils/git-graph-layout";
import { useIsCompactFormFactor } from "@/constants/layout";
import type { GitGraphCommit } from "@server/shared/messages";

interface CommitGraphPaneProps {
  serverId: string;
  cwd: string;
}

function CommitDetail({ commit, isDark }: { commit: GitGraphCommit; isDark: boolean }) {
  const date = new Date(commit.date * 1000);
  const dateStr = date.toLocaleDateString();
  const timeStr = date.toLocaleTimeString();

  return (
    <View style={detailStyles.container}>
      <Text style={[detailStyles.message, { color: isDark ? "#e6edf3" : "#1f2328" }]}>
        {commit.message}
      </Text>
      <Text style={[detailStyles.meta, { color: isDark ? "#7d8590" : "#656d76" }]}>
        {commit.author} · {dateStr} {timeStr}
      </Text>
      <Text style={[detailStyles.hash, { color: isDark ? "#7d8590" : "#656d76" }]}>
        {commit.hash}
      </Text>
      {commit.branchTips.length > 0 && (
        <View style={detailStyles.branches}>
          {commit.branchTips.map((branch) => (
            <View key={branch} style={detailStyles.branchTag}>
              <GitBranch size={12} color={isDark ? "#58a6ff" : "#0969da"} />
              <Text style={[detailStyles.branchName, { color: isDark ? "#58a6ff" : "#0969da" }]}>
                {branch}
              </Text>
            </View>
          ))}
        </View>
      )}
    </View>
  );
}

const detailStyles = StyleSheet.create({
  container: {
    padding: 16,
    gap: 8,
  },
  message: {
    fontSize: 16,
    fontWeight: "600",
    lineHeight: 24,
  },
  meta: {
    fontSize: 13,
    lineHeight: 18,
  },
  hash: {
    fontSize: 12,
    fontFamily: "monospace",
    lineHeight: 18,
  },
  branches: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 8,
    marginTop: 4,
  },
  branchTag: {
    flexDirection: "row",
    alignItems: "center",
    gap: 4,
    backgroundColor: "rgba(9, 105, 218, 0.1)",
    paddingHorizontal: 8,
    paddingVertical: 4,
    borderRadius: 12,
  },
  branchName: {
    fontSize: 12,
    fontWeight: "500",
  },
});

export const CommitGraphPane = memo(function CommitGraphPane({
  serverId,
  cwd,
}: CommitGraphPaneProps) {
  const { graph, isLoading, isError, error } = useCommitGraphQuery({ serverId, cwd });
  const [selectedCommit, setSelectedCommit] = useState<GitGraphCommit | null>(null);
  const { theme } = useUnistyles();
  const isCompact = useIsCompactFormFactor();
  const isDark = theme.colorScheme === "dark";

  const layout = useMemo(() => {
    if (!graph || graph.commits.length === 0) return null;
    return layoutGitGraph(graph, { isDark });
  }, [graph, isDark]);

  const handleNodePress = useCallback((commit: GitGraphCommit) => {
    setSelectedCommit((prev) => (prev?.fullHash === commit.fullHash ? null : commit));
  }, []);

  if (isLoading) {
    return (
      <View style={styles.center}>
        <ActivityIndicator size="large" color={theme.colors.foregroundMuted} />
      </View>
    );
  }

  if (isError || !graph) {
    const errorMessage = error instanceof Error ? error.message : "Failed to load commit graph";
    return (
      <View style={styles.center}>
        <Text style={[styles.errorText, { color: theme.colors.foregroundMuted }]}>
          {errorMessage}
        </Text>
      </View>
    );
  }

  if (graph.commits.length === 0) {
    return (
      <View style={styles.center}>
        <GitCommitHorizontal size={48} color={theme.colors.foregroundMuted} />
        <Text style={[styles.emptyText, { color: theme.colors.foregroundMuted }]}>
          No commits found
        </Text>
      </View>
    );
  }

  if (!layout) return null;
  const svgWidth = Math.max(layout.width, 300);
  const svgHeight = Math.max(layout.height, 200);
  const padding = 20;

  return (
    <View style={styles.container}>
      <ScrollView
        horizontal
        showsHorizontalScrollIndicator
        contentContainerStyle={styles.scrollContent}
      >
        <ScrollView showsVerticalScrollIndicator contentContainerStyle={styles.scrollContent}>
          <Svg
            width={svgWidth + padding * 2}
            height={svgHeight + padding * 2}
            viewBox={`${-padding} ${-padding} ${svgWidth + padding * 2} ${svgHeight + padding * 2}`}
          >
            {/* Branch lines */}
            {layout.edges.map((edge, i) => (
              <Path
                key={`edge-${i}`}
                d={edge.path}
                fill="none"
                stroke={edge.color}
                strokeWidth={2}
                opacity={0.6}
              />
            ))}

            {/* Commit nodes */}
            {layout.nodes.map((node) => {
              const isHead = graph.headCommit === node.commit.fullHash;
              const isSelected = selectedCommit?.fullHash === node.commit.fullHash;
              const radius = isHead ? layout.nodeRadius + 3 : layout.nodeRadius;

              return (
                <G key={node.commit.fullHash}>
                  {/* Outer ring for HEAD */}
                  {isHead && (
                    <Circle
                      cx={node.x}
                      cy={node.y}
                      r={radius + 3}
                      fill="none"
                      stroke={node.color}
                      strokeWidth={2}
                      opacity={0.8}
                    />
                  )}
                  {/* Commit dot */}
                  <Circle
                    cx={node.x}
                    cy={node.y}
                    r={radius}
                    fill={isSelected ? node.color : isDark ? "#0d1117" : "#ffffff"}
                    stroke={node.color}
                    strokeWidth={isSelected ? 3 : 2}
                  />
                  {/* Commit message label */}
                  <SvgText
                    x={node.x + radius + 8}
                    y={node.y + 4}
                    fill={isDark ? "#e6edf3" : "#1f2328"}
                    fontSize={isCompact ? 10 : 12}
                    fontFamily="system-ui, sans-serif"
                  >
                    {node.commit.message.length > 30
                      ? `${node.commit.message.slice(0, 30)}...`
                      : node.commit.message}
                  </SvgText>
                  {/* Branch tip labels */}
                  {node.commit.branchTips.map((tip, tipIndex) => (
                    <SvgText
                      key={tip}
                      x={node.x - radius - 8}
                      y={node.y + 4 + tipIndex * 14}
                      fill={node.color}
                      fontSize={10}
                      fontWeight="600"
                      textAnchor="end"
                      fontFamily="system-ui, sans-serif"
                    >
                      {tip}
                    </SvgText>
                  ))}
                  {/* Invisible touch target */}
                  <Circle
                    cx={node.x}
                    cy={node.y}
                    r={layout.rowHeight / 2}
                    fill="transparent"
                    onPress={() => handleNodePress(node.commit)}
                    pointerEvents="auto"
                  />
                </G>
              );
            })}
          </Svg>
        </ScrollView>
      </ScrollView>

      {/* Selected commit detail */}
      {selectedCommit && (
        <View style={[styles.detailPanel, { borderTopColor: theme.colors.border }]}>
          <CommitDetail commit={selectedCommit} isDark={isDark} />
        </View>
      )}
    </View>
  );
});

const styles = StyleSheet.create((theme) => ({
  container: {
    flex: 1,
    backgroundColor: theme.colors.background,
  },
  center: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    gap: 12,
    padding: 24,
  },
  scrollContent: {
    flexGrow: 1,
  },
  errorText: {
    fontSize: 15,
  },
  emptyText: {
    fontSize: 15,
    textAlign: "center",
  },
  detailPanel: {
    borderTopWidth: 1,
    maxHeight: 200,
    backgroundColor: theme.colors.surface1,
  },
}));

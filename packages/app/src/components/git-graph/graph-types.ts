import type { GitGraph, GitGraphCommit } from "@server/shared/messages";
import type { GraphLayout, GraphNode } from "@/utils/git-graph-layout";
import type { GestureResponderEvent } from "react-native";

export interface CommitGraphPaneProps {
  serverId: string;
  cwd: string;
  onClose?: () => void;
}

export interface GraphToolbarProps {
  title: string;
  isFetching: boolean;
  searchQuery: string;
  onSearchChange: (query: string) => void;
  onClearSearch: () => void;
  matchCount: number;
  totalCount: number;
  onNextMatch?: () => void;
  onPrevMatch?: () => void;
  onRefresh: () => void;
  onLocateHead: () => void;
  showRefs: boolean;
  onToggleRefs: () => void;
  onClose?: () => void;
}

export interface GraphSvgLayerProps {
  layout: GraphLayout;
  selectedHash: string | null;
  headHash: string | null;
}

export interface CommitRowProps {
  node: GraphNode;
  layout: GraphLayout;
  graph: GitGraph;
  graphWidth: number;
  showMetadata: boolean;
  showAuthor: boolean;
  showRefs: boolean;
  isMatched: boolean;
  hasQuery: boolean;
  selected: boolean;
  compareSelected: boolean;
  onPress: (commit: GitGraphCommit, event: GestureResponderEvent) => void;
  onCopyHash: (commit: GitGraphCommit) => void;
  onCopyRefs: (commit: GitGraphCommit) => void;
}

export interface CommitDetailPanelProps {
  commit: GitGraphCommit;
  compareCommit: GitGraphCommit | null;
  onClearCompare: () => void;
  onClose: () => void;
}

export interface RefBadgeProps {
  label: string;
  kind: "branch" | "tag";
}

// Layout constants
export const GRAPH_COLUMN_WIDTH = 92;
export const MEDIUM_GRAPH_COLUMN_WIDTH = 78;
export const NARROW_GRAPH_COLUMN_WIDTH = 64;
export const GRAPH_COLUMN_SPACING = 14;
export const GRAPH_NODE_RADIUS = 5;
export const COMMIT_ROW_HEIGHT = 28;
export const DATE_COLUMN_WIDTH = 118;
export const AUTHOR_COLUMN_WIDTH = 76;
export const HASH_COLUMN_WIDTH = 70;
export const ROW_HORIZONTAL_PADDING = 8;
export const DETAIL_PANEL_HEIGHT = 140;
export const COMMIT_LIMIT = 200;

import type { GitGraph, GitGraphCommit, GitGraphBranch } from "@server/shared/messages";

export interface GraphNode {
  commit: GitGraphCommit;
  x: number;
  y: number;
  color: string;
}

export interface GraphEdge {
  from: string;
  to: string;
  path: string;
  color: string;
}

export interface GraphLayout {
  nodes: GraphNode[];
  edges: GraphEdge[];
  width: number;
  height: number;
  columnWidth: number;
  rowHeight: number;
  nodeRadius: number;
}

const DEFAULT_COLUMN_WIDTH = 48;
const DEFAULT_ROW_HEIGHT = 40;
const DEFAULT_NODE_RADIUS = 6;

function hashString(str: string): number {
  let hash = 0;
  for (let i = 0; i < str.length; i++) {
    const char = str.charCodeAt(i);
    hash = ((hash << 5) - hash + char) | 0;
  }
  return Math.abs(hash);
}

function branchColor(name: string, themeColors: string[]): string {
  const index = hashString(name) % themeColors.length;
  return themeColors[index];
}

function getThemeColors(isDark: boolean): string[] {
  return isDark
    ? ["#58a6ff", "#7ee787", "#ffa657", "#d2a8ff", "#ff7b72", "#79c0ff", "#56d364", "#e3b341"]
    : ["#0969da", "#1a7f37", "#bc4c00", "#8250df", "#cf222e", "#0969da", "#1a7f37", "#9a6700"];
}

export function layoutGitGraph(
  graph: GitGraph,
  options?: { isDark?: boolean; columnWidth?: number; rowHeight?: number; nodeRadius?: number },
): GraphLayout {
  const isDark = options?.isDark ?? true;
  const columnWidth = options?.columnWidth ?? DEFAULT_COLUMN_WIDTH;
  const rowHeight = options?.rowHeight ?? DEFAULT_ROW_HEIGHT;
  const nodeRadius = options?.nodeRadius ?? DEFAULT_NODE_RADIUS;
  const themeColors = getThemeColors(isDark);

  const commits = [...graph.commits];
  const commitIndex = new Map<string, number>();
  for (let i = 0; i < commits.length; i++) {
    commitIndex.set(commits[i].fullHash, i);
  }

  // Column assignment
  const columnMap = new Map<string, number>();
  const branchColumnMap = new Map<string, number>();
  let nextColumn = 0;

  // Assign main branch (containing HEAD) to column 0
  const headCommit = graph.headCommit;
  if (headCommit) {
    const headBranch = graph.branches.find((b: GitGraphBranch) => b.isCurrent);
    if (headBranch) {
      branchColumnMap.set(headBranch.name, 0);
    }
  }

  // First pass: assign branch columns
  for (const branch of graph.branches) {
    if (branchColumnMap.has(branch.name)) continue;
    branchColumnMap.set(branch.name, ++nextColumn);
  }

  // Second pass: assign commit columns
  for (let i = 0; i < commits.length; i++) {
    const commit = commits[i];
    let col = -1;

    // If this commit is a branch tip, use that branch's column
    for (const branchTip of commit.branchTips) {
      const branchCol = branchColumnMap.get(branchTip);
      if (branchCol !== undefined) {
        col = branchCol;
        break;
      }
    }

    // If no branch tip, try to inherit from children (commits that have this as parent)
    if (col === -1) {
      for (let j = 0; j < i; j++) {
        const child = commits[j];
        if (child.parents.includes(commit.fullHash)) {
          const childCol = columnMap.get(child.fullHash);
          if (childCol !== undefined) {
            col = childCol;
            break;
          }
        }
      }
    }

    // If still no column, use the parent's column or create a new one
    if (col === -1 && commit.parents.length > 0) {
      for (const parentHash of commit.parents) {
        const parentCol = columnMap.get(parentHash);
        if (parentCol !== undefined) {
          col = parentCol;
          break;
        }
      }
    }

    if (col === -1) {
      col = 0;
    }

    columnMap.set(commit.fullHash, col);
  }

  // Build nodes
  const nodes: GraphNode[] = [];
  for (let i = 0; i < commits.length; i++) {
    const commit = commits[i];
    const col = columnMap.get(commit.fullHash) ?? 0;

    // Determine color from first branch tip, or default
    let color = themeColors[0];
    if (commit.branchTips.length > 0) {
      color = branchColor(commit.branchTips[0], themeColors);
    } else if (commit.parents.length > 0) {
      // Inherit color from first parent
      const parentCol = columnMap.get(commit.parents[0]);
      if (parentCol !== undefined) {
        const parentBranch = graph.branches.find(
          (b: GitGraphBranch) => branchColumnMap.get(b.name) === parentCol,
        );
        if (parentBranch) {
          color = branchColor(parentBranch.name, themeColors);
        }
      }
    }

    nodes.push({
      commit,
      x: col * columnWidth + columnWidth / 2,
      y: i * rowHeight + rowHeight / 2,
      color,
    });
  }

  // Build edges
  const edges: GraphEdge[] = [];
  for (let i = 0; i < commits.length; i++) {
    const commit = commits[i];
    const childNode = nodes[i];

    for (const parentHash of commit.parents) {
      const parentIndex = commitIndex.get(parentHash);
      if (parentIndex === undefined) continue;

      const parentNode = nodes[parentIndex];
      const isMerge = commit.parents.length > 1 && parentHash !== commit.parents[0];

      // Bezier curve from child to parent
      const midY = (childNode.y + parentNode.y) / 2;
      const path = `M ${childNode.x} ${childNode.y} C ${childNode.x} ${midY}, ${parentNode.x} ${midY}, ${parentNode.x} ${parentNode.y}`;

      edges.push({
        from: commit.fullHash,
        to: parentHash,
        path,
        color: isMerge ? themeColors[4] : childNode.color,
      });
    }
  }

  const maxColumn = Math.max(0, ...Array.from(columnMap.values()));
  const width = (maxColumn + 1) * columnWidth;
  const height = commits.length * rowHeight;

  return { nodes, edges, width, height, columnWidth, rowHeight, nodeRadius };
}

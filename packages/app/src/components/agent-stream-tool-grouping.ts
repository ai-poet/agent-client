import { isAgentToolCallItem, type AgentToolCallItem, type StreamItem } from "@/types/stream";

/**
 * A run of consecutive tool calls collapses into one row so a long stretch of reads or
 * greps does not bury the narrative. Grouping is presentation-only: the underlying
 * timeline items are preserved in order inside the group.
 */
export type StreamToolGroupNode = {
  kind: "tool_group";
  id: string;
  items: AgentToolCallItem[];
  /** Worst status wins, so a single failure stays visible on the collapsed row. */
  status: "failed" | "running" | "completed";
};

export type StreamRenderNode = { kind: "item"; item: StreamItem } | StreamToolGroupNode;

/** Reads are noisier and more repetitive than other tools, so they collapse sooner. */
const READ_ONLY_GROUP_THRESHOLD = 2;
const MIXED_GROUP_THRESHOLD = 3;

/**
 * Tools that render as interactive cards (approvals, questions, plans) must never be
 * folded away — the user has to act on them.
 */
const NON_GROUPABLE_TOOL_NAMES = new Set([
  "todowrite",
  "todo_write",
  "askuserquestion",
  "requestuserinput",
  "exitplanmode",
  "enterplanmode",
  "approvalrequest",
  "task",
]);

function normalizeToolName(item: AgentToolCallItem): string {
  return item.payload.data.name.trim().toLowerCase();
}

function isReadTool(item: AgentToolCallItem): boolean {
  return item.payload.data.detail.type === "read";
}

function isGroupable(item: StreamItem): item is AgentToolCallItem {
  if (!isAgentToolCallItem(item)) {
    return false;
  }
  // A tool that is still executing stays on its own row so the user can watch it.
  if (item.payload.data.status === "running") {
    return false;
  }
  if (item.payload.data.detail.type === "plan") {
    return false;
  }
  return !NON_GROUPABLE_TOOL_NAMES.has(normalizeToolName(item));
}

function resolveGroupStatus(items: AgentToolCallItem[]): StreamToolGroupNode["status"] {
  if (items.some((item) => item.payload.data.status === "failed")) {
    return "failed";
  }
  if (items.some((item) => item.payload.data.status === "running")) {
    return "running";
  }
  return "completed";
}

function resolveGroupThreshold(items: AgentToolCallItem[]): number {
  return items.every(isReadTool) ? READ_ONLY_GROUP_THRESHOLD : MIXED_GROUP_THRESHOLD;
}

function flushRun(run: AgentToolCallItem[], nodes: StreamRenderNode[]): void {
  if (run.length === 0) {
    return;
  }
  if (run.length < resolveGroupThreshold(run)) {
    for (const item of run) {
      nodes.push({ kind: "item", item });
    }
    return;
  }
  const first = run[0];
  if (!first) {
    return;
  }
  nodes.push({
    kind: "tool_group",
    id: `tool-group:${first.id}:${run.length}`,
    items: [...run],
    status: resolveGroupStatus(run),
  });
}

/**
 * Collapses runs of consecutive completed tool calls into group nodes. Any non-tool item
 * (assistant text, a question card, a running tool) ends the current run.
 */
export function groupStreamToolCalls(items: StreamItem[]): StreamRenderNode[] {
  const nodes: StreamRenderNode[] = [];
  let run: AgentToolCallItem[] = [];

  for (const item of items) {
    if (isGroupable(item)) {
      run.push(item);
      continue;
    }
    flushRun(run, nodes);
    run = [];
    nodes.push({ kind: "item", item });
  }
  flushRun(run, nodes);

  return nodes;
}

export type ToolGroupRole =
  /** Renders the collapsed summary row for the whole run. */
  | { role: "anchor"; group: StreamToolGroupNode }
  /** Hidden while the group is collapsed; revealed under the anchor when expanded. */
  | { role: "member"; anchorId: string };

/**
 * Maps each grouped item id to its role so the item-keyed renderer can stay as-is:
 * item ids and their order never change, only what each one draws.
 */
export function buildToolGroupIndex(items: StreamItem[]): Map<string, ToolGroupRole> {
  const index = new Map<string, ToolGroupRole>();
  for (const node of groupStreamToolCalls(items)) {
    if (node.kind !== "tool_group") {
      continue;
    }
    node.items.forEach((item, position) => {
      if (position === 0) {
        index.set(item.id, { role: "anchor", group: node });
        return;
      }
      const anchor = node.items[0];
      if (anchor) {
        index.set(item.id, { role: "member", anchorId: anchor.id });
      }
    });
  }
  return index;
}

export type ToolGroupSummary = {
  /** e.g. 5 for "read 5 files" */
  count: number;
  /** Present when every call in the group is a file read. */
  readFileNames: string[] | null;
  /** e.g. [["Bash", 2], ["Grep", 1]] — insertion ordered by first appearance. */
  toolCounts: [string, number][];
};

/**
 * Describes a group's contents so the collapsed row can say what actually happened
 * rather than just "5 tool calls".
 */
export function summarizeToolGroup(
  group: StreamToolGroupNode,
  resolveDisplayName: (item: AgentToolCallItem) => string,
  resolveReadFileName: (item: AgentToolCallItem) => string | null,
): ToolGroupSummary {
  const allReads = group.items.every(isReadTool);
  const readFileNames = allReads
    ? group.items
        .map(resolveReadFileName)
        .filter((name): name is string => Boolean(name && name.length > 0))
    : null;

  const counts = new Map<string, number>();
  for (const item of group.items) {
    const name = resolveDisplayName(item);
    counts.set(name, (counts.get(name) ?? 0) + 1);
  }

  return {
    count: group.items.length,
    readFileNames: readFileNames && readFileNames.length > 0 ? readFileNames : null,
    toolCounts: [...counts.entries()],
  };
}

import { describe, expect, it } from "vitest";

import type { AgentToolCallItem, StreamItem } from "@/types/stream";
import type { AgentToolCallStatus } from "@/types/stream";
import {
  buildToolGroupIndex,
  groupStreamToolCalls,
  summarizeToolGroup,
  type StreamToolGroupNode,
} from "./agent-stream-tool-grouping";

let sequence = 0;

function makeToolCall(options: {
  name: string;
  detailType?: string;
  status?: AgentToolCallStatus;
  filePath?: string;
}): AgentToolCallItem {
  sequence += 1;
  const id = `tool-${sequence}`;
  return {
    kind: "tool_call",
    id,
    timestamp: new Date(0),
    payload: {
      source: "agent",
      data: {
        provider: "claude",
        callId: id,
        name: options.name,
        status: options.status ?? "completed",
        error: null,
        detail: {
          type: options.detailType ?? "read",
          ...(options.filePath ? { filePath: options.filePath } : {}),
        },
      },
    },
  } as AgentToolCallItem;
}

function makeAssistantMessage(text: string): StreamItem {
  sequence += 1;
  return {
    kind: "assistant_message",
    id: `assistant-${sequence}`,
    text,
    timestamp: new Date(0),
  } as StreamItem;
}

function groupsOf(nodes: ReturnType<typeof groupStreamToolCalls>): StreamToolGroupNode[] {
  return nodes.filter((node): node is StreamToolGroupNode => node.kind === "tool_group");
}

describe("groupStreamToolCalls", () => {
  it("groups two consecutive reads", () => {
    const nodes = groupStreamToolCalls([
      makeToolCall({ name: "Read" }),
      makeToolCall({ name: "Read" }),
    ]);

    expect(nodes).toHaveLength(1);
    expect(groupsOf(nodes)[0]?.items).toHaveLength(2);
  });

  it("leaves a single read ungrouped", () => {
    const nodes = groupStreamToolCalls([makeToolCall({ name: "Read" })]);

    expect(nodes).toEqual([{ kind: "item", item: expect.objectContaining({ kind: "tool_call" }) }]);
  });

  it("requires three calls before grouping mixed tools", () => {
    const twoMixed = groupStreamToolCalls([
      makeToolCall({ name: "Bash", detailType: "shell" }),
      makeToolCall({ name: "Read" }),
    ]);
    expect(groupsOf(twoMixed)).toHaveLength(0);
    expect(twoMixed).toHaveLength(2);

    const threeMixed = groupStreamToolCalls([
      makeToolCall({ name: "Bash", detailType: "shell" }),
      makeToolCall({ name: "Read" }),
      makeToolCall({ name: "Grep", detailType: "search" }),
    ]);
    expect(groupsOf(threeMixed)).toHaveLength(1);
    expect(groupsOf(threeMixed)[0]?.items).toHaveLength(3);
  });

  it("never groups a running tool so the live call stays visible", () => {
    const nodes = groupStreamToolCalls([
      makeToolCall({ name: "Read" }),
      makeToolCall({ name: "Read", status: "running" }),
      makeToolCall({ name: "Read" }),
    ]);

    // The running call splits the run, leaving three standalone rows.
    expect(groupsOf(nodes)).toHaveLength(0);
    expect(nodes).toHaveLength(3);
  });

  it("breaks a run when narrative text appears between tools", () => {
    const nodes = groupStreamToolCalls([
      makeToolCall({ name: "Read" }),
      makeToolCall({ name: "Read" }),
      makeAssistantMessage("Now editing"),
      makeToolCall({ name: "Read" }),
      makeToolCall({ name: "Read" }),
    ]);

    expect(nodes.map((node) => node.kind)).toEqual(["tool_group", "item", "tool_group"]);
  });

  it("excludes interactive tools from groups", () => {
    const nodes = groupStreamToolCalls([
      makeToolCall({ name: "Read" }),
      makeToolCall({ name: "TodoWrite", detailType: "unknown" }),
      makeToolCall({ name: "Read" }),
    ]);

    expect(groupsOf(nodes)).toHaveLength(0);
    expect(nodes).toHaveLength(3);
  });

  it("keeps plan tool calls out of groups", () => {
    const nodes = groupStreamToolCalls([
      makeToolCall({ name: "Read" }),
      makeToolCall({ name: "Plan", detailType: "plan" }),
      makeToolCall({ name: "Read" }),
    ]);

    expect(groupsOf(nodes)).toHaveLength(0);
  });

  it("surfaces the worst status on the group", () => {
    const failed = groupStreamToolCalls([
      makeToolCall({ name: "Read" }),
      makeToolCall({ name: "Read", status: "failed" }),
    ]);
    expect(groupsOf(failed)[0]?.status).toBe("failed");

    const completed = groupStreamToolCalls([
      makeToolCall({ name: "Read" }),
      makeToolCall({ name: "Read" }),
    ]);
    expect(groupsOf(completed)[0]?.status).toBe("completed");
  });

  it("preserves the original order of items outside groups", () => {
    const first = makeAssistantMessage("first");
    const second = makeAssistantMessage("second");
    const nodes = groupStreamToolCalls([first, second]);

    expect(nodes).toEqual([
      { kind: "item", item: first },
      { kind: "item", item: second },
    ]);
  });

  it("returns nothing for an empty timeline", () => {
    expect(groupStreamToolCalls([])).toEqual([]);
  });
});

describe("buildToolGroupIndex", () => {
  it("marks the first item of a run as the anchor and the rest as members", () => {
    const first = makeToolCall({ name: "Read" });
    const second = makeToolCall({ name: "Read" });
    const third = makeToolCall({ name: "Read" });

    const index = buildToolGroupIndex([first, second, third]);

    expect(index.get(first.id)).toMatchObject({ role: "anchor" });
    expect(index.get(second.id)).toEqual({ role: "member", anchorId: first.id });
    expect(index.get(third.id)).toEqual({ role: "member", anchorId: first.id });
  });

  it("leaves ungrouped items out of the index entirely", () => {
    const lone = makeToolCall({ name: "Read" });
    const message = makeAssistantMessage("hello");

    const index = buildToolGroupIndex([lone, message]);

    expect(index.size).toBe(0);
  });

  it("indexes each run separately", () => {
    const runOne = [makeToolCall({ name: "Read" }), makeToolCall({ name: "Read" })];
    const divider = makeAssistantMessage("next");
    const runTwo = [makeToolCall({ name: "Read" }), makeToolCall({ name: "Read" })];

    const index = buildToolGroupIndex([...runOne, divider, ...runTwo]);

    expect(index.get(runOne[1]!.id)).toEqual({ role: "member", anchorId: runOne[0]!.id });
    expect(index.get(runTwo[1]!.id)).toEqual({ role: "member", anchorId: runTwo[0]!.id });
  });
});

describe("summarizeToolGroup", () => {
  const displayName = (item: AgentToolCallItem) => item.payload.data.name;
  const readFileName = (item: AgentToolCallItem) => {
    const detail = item.payload.data.detail as { filePath?: string };
    return detail.filePath ?? null;
  };

  it("lists file names when every call is a read", () => {
    const nodes = groupStreamToolCalls([
      makeToolCall({ name: "Read", filePath: "a.ts" }),
      makeToolCall({ name: "Read", filePath: "b.ts" }),
    ]);
    const group = groupsOf(nodes)[0];
    if (!group) {
      throw new Error("expected a group");
    }

    expect(summarizeToolGroup(group, displayName, readFileName)).toEqual({
      count: 2,
      readFileNames: ["a.ts", "b.ts"],
      toolCounts: [["Read", 2]],
    });
  });

  it("counts tools by name for mixed groups", () => {
    const nodes = groupStreamToolCalls([
      makeToolCall({ name: "Bash", detailType: "shell" }),
      makeToolCall({ name: "Bash", detailType: "shell" }),
      makeToolCall({ name: "Grep", detailType: "search" }),
    ]);
    const group = groupsOf(nodes)[0];
    if (!group) {
      throw new Error("expected a group");
    }

    const summary = summarizeToolGroup(group, displayName, readFileName);
    expect(summary.count).toBe(3);
    expect(summary.readFileNames).toBeNull();
    expect(summary.toolCounts).toEqual([
      ["Bash", 2],
      ["Grep", 1],
    ]);
  });
});

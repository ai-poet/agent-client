import { describe, expect, it } from "vitest";

import type { StreamItem } from "@/types/stream";
import {
  collectStreamingText,
  estimateOutputTokens,
  formatElapsed,
  resolveDisplayTokens,
  resolveTurnPhase,
} from "./turn-progress";

let sequence = 0;

function userMessage(): StreamItem {
  sequence += 1;
  return {
    kind: "user_message",
    id: `user-${sequence}`,
    text: "hi",
    timestamp: new Date(0),
  } as StreamItem;
}

function assistantMessage(text: string): StreamItem {
  sequence += 1;
  return {
    kind: "assistant_message",
    id: `assistant-${sequence}`,
    text,
    timestamp: new Date(0),
  } as StreamItem;
}

function thought(status: "loading" | "ready"): StreamItem {
  sequence += 1;
  return {
    kind: "thought",
    id: `thought-${sequence}`,
    text: "hmm",
    timestamp: new Date(0),
    status,
  } as StreamItem;
}

function toolCall(status: "running" | "completed"): StreamItem {
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
        name: "Read",
        status,
        error: null,
        detail: { type: "read" },
      },
    },
  } as StreamItem;
}

const base = { pendingPermissionCount: 0, awaitingUserInput: false };

describe("resolveTurnPhase", () => {
  it("puts approval above everything else", () => {
    expect(
      resolveTurnPhase({
        pendingPermissionCount: 1,
        awaitingUserInput: true,
        items: [toolCall("running")],
      }),
    ).toBe("awaiting_approval");
  });

  it("puts a pending question above agent activity", () => {
    expect(
      resolveTurnPhase({
        pendingPermissionCount: 0,
        awaitingUserInput: true,
        items: [toolCall("running")],
      }),
    ).toBe("awaiting_input");
  });

  it("reports a running tool", () => {
    expect(resolveTurnPhase({ ...base, items: [toolCall("running")] })).toBe("running_tool");
  });

  it("reports thinking only while the thought is open", () => {
    expect(resolveTurnPhase({ ...base, items: [thought("loading")] })).toBe("thinking");
    expect(resolveTurnPhase({ ...base, items: [thought("ready")] })).toBe("generating");
  });

  it("reports generating once a tool has finished", () => {
    expect(resolveTurnPhase({ ...base, items: [toolCall("completed")] })).toBe("generating");
  });

  it("uses the newest item, not the oldest", () => {
    expect(resolveTurnPhase({ ...base, items: [thought("loading"), toolCall("running")] })).toBe(
      "running_tool",
    );
    expect(resolveTurnPhase({ ...base, items: [toolCall("running"), thought("loading")] })).toBe(
      "thinking",
    );
  });

  it("falls back to working right after the user speaks", () => {
    expect(resolveTurnPhase({ ...base, items: [userMessage()] })).toBe("working");
    expect(resolveTurnPhase({ ...base, items: [] })).toBe("working");
  });
});

describe("estimateOutputTokens", () => {
  it("returns zero for empty text", () => {
    expect(estimateOutputTokens("")).toBe(0);
  });

  it("counts ascii at roughly four characters per token", () => {
    expect(estimateOutputTokens("a".repeat(40))).toBe(10);
  });

  it("counts CJK far more densely than ascii", () => {
    const cjk = estimateOutputTokens("你好世界".repeat(10));
    const ascii = estimateOutputTokens("a".repeat(40));
    expect(cjk).toBeGreaterThan(ascii);
  });

  it("grows monotonically as text streams in", () => {
    expect(estimateOutputTokens("hello world")).toBeGreaterThan(estimateOutputTokens("hello"));
  });
});

describe("resolveDisplayTokens", () => {
  it("shows the estimate while real usage lags behind", () => {
    expect(resolveDisplayTokens({ estimated: 120, reported: 0 })).toEqual({
      tokens: 120,
      isEstimate: true,
    });
    expect(resolveDisplayTokens({ estimated: 120, reported: undefined })).toEqual({
      tokens: 120,
      isEstimate: true,
    });
  });

  it("switches to real usage once it catches up", () => {
    expect(resolveDisplayTokens({ estimated: 120, reported: 150 })).toEqual({
      tokens: 150,
      isEstimate: false,
    });
  });

  it("never lets the counter jump backwards", () => {
    // Real usage below the estimate would visibly rewind the number.
    expect(resolveDisplayTokens({ estimated: 200, reported: 50 })).toEqual({
      tokens: 200,
      isEstimate: true,
    });
  });
});

describe("collectStreamingText", () => {
  it("collects assistant text since the last user message", () => {
    const items = [
      assistantMessage("stale"),
      userMessage(),
      assistantMessage("Hello "),
      assistantMessage("world"),
    ];

    expect(collectStreamingText(items)).toBe("Hello world");
  });

  it("returns empty when the user just spoke", () => {
    expect(collectStreamingText([assistantMessage("old"), userMessage()])).toBe("");
  });
});

describe("formatElapsed", () => {
  it("floors sub-second durations", () => {
    expect(formatElapsed(0)).toBe("0s");
    expect(formatElapsed(400)).toBe("0s");
  });

  it("formats seconds, minutes and hours", () => {
    expect(formatElapsed(45_000)).toBe("45s");
    expect(formatElapsed(90_000)).toBe("1m 30s");
    expect(formatElapsed(120_000)).toBe("2m");
    expect(formatElapsed(3_600_000)).toBe("1h");
    expect(formatElapsed(5_400_000)).toBe("1h 30m");
  });
});

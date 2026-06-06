import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { JSDOM } from "jsdom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CommitGraphResponse } from "@server/shared/messages";
import { commitGraphQueryKey, useCommitGraphQuery } from "./use-commit-graph-query";

type CommitGraphPayload = CommitGraphResponse["payload"];

const { mockRuntime, mockClient } = vi.hoisted(() => {
  const mockClient = {
    getCommitGraph: vi.fn(),
  };

  return {
    mockClient,
    mockRuntime: {
      client: mockClient,
      isConnected: true,
    },
  };
});

vi.mock("@/runtime/host-runtime", () => ({
  useHostRuntimeClient: () => mockRuntime.client,
  useHostRuntimeIsConnected: () => mockRuntime.isConnected,
}));

const serverId = "server-1";
const cwd = "/repo";

function commitGraphPayload(overrides: Partial<CommitGraphPayload> = {}): CommitGraphPayload {
  return {
    cwd,
    error: null,
    requestId: "commit-graph-1",
    graph: {
      commits: [],
      branches: [],
      headCommit: null,
      rootCommits: [],
    },
    ...overrides,
  };
}

function createTestQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
      },
    },
  });
}

function renderCommitGraphHook({
  queryClient = createTestQueryClient(),
}: {
  queryClient?: QueryClient;
} = {}) {
  let latest: ReturnType<typeof useCommitGraphQuery> | null = null;

  function Probe() {
    latest = useCommitGraphQuery({ serverId, cwd });
    return null;
  }

  const container = document.getElementById("root");
  if (!container) {
    throw new Error("Missing root container");
  }

  const root = createRoot(container);

  return {
    get latest() {
      if (!latest) {
        throw new Error("Expected hook result");
      }
      return latest;
    },
    async mount() {
      await act(async () => {
        root.render(
          React.createElement(
            QueryClientProvider,
            { client: queryClient },
            React.createElement(Probe),
          ),
        );
      });
    },
    async unmount() {
      await act(async () => {
        root.unmount();
      });
    },
  };
}

async function waitForExpectation(assertion: () => void): Promise<void> {
  let lastError: unknown;

  for (let attempt = 0; attempt < 60; attempt += 1) {
    try {
      assertion();
      return;
    } catch (error) {
      lastError = error;
      await act(async () => {
        vi.advanceTimersByTime(10);
        await Promise.resolve();
      });
    }
  }

  throw lastError;
}

describe("useCommitGraphQuery", () => {
  beforeEach(() => {
    vi.useFakeTimers();

    const dom = new JSDOM("<!doctype html><html><body><div id='root'></div></body></html>", {
      url: "http://localhost",
    });

    Object.defineProperty(globalThis, "document", {
      value: dom.window.document,
      configurable: true,
    });
    Object.defineProperty(globalThis, "window", {
      value: dom.window,
      configurable: true,
    });
    Object.defineProperty(globalThis, "navigator", {
      value: dom.window.navigator,
      configurable: true,
    });
    Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
      value: true,
      configurable: true,
    });

    mockRuntime.client = mockClient;
    mockRuntime.isConnected = true;
    mockClient.getCommitGraph.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("fetches the commit graph for the supplied cwd", async () => {
    mockClient.getCommitGraph.mockResolvedValue(commitGraphPayload());
    const hook = renderCommitGraphHook();

    await hook.mount();

    await waitForExpectation(() => {
      expect(hook.latest.graph).toEqual(commitGraphPayload().graph);
    });
    expect(mockClient.getCommitGraph).toHaveBeenCalledWith({ cwd, limit: undefined });
  });

  it("surfaces daemon payload errors as query errors", async () => {
    mockClient.getCommitGraph.mockResolvedValue(
      commitGraphPayload({
        error: "fatal: not a git repository",
      }),
    );
    const hook = renderCommitGraphHook();

    await hook.mount();

    await waitForExpectation(() => {
      expect(hook.latest.isError).toBe(true);
      expect(hook.latest.error).toEqual(new Error("fatal: not a git repository"));
    });
  });

  it("surfaces cached payload errors without calling the daemon", async () => {
    const queryClient = createTestQueryClient();
    queryClient.setQueryData(
      commitGraphQueryKey(serverId, cwd),
      commitGraphPayload({
        error: "cached graph failure",
      }),
    );
    const hook = renderCommitGraphHook({ queryClient });

    await hook.mount();

    await waitForExpectation(() => {
      expect(hook.latest.isError).toBe(true);
      expect(hook.latest.error).toEqual(new Error("cached graph failure"));
    });
    expect(mockClient.getCommitGraph).not.toHaveBeenCalled();
  });

  it("explains old daemon schema errors", async () => {
    const error = new Error(
      "Unknown request schema requestType=commit_graph_request code=unknown_schema",
    );
    error.name = "DaemonRpcError";
    Object.assign(error, {
      code: "unknown_schema",
      requestType: "commit_graph_request",
    });
    mockClient.getCommitGraph.mockRejectedValue(error);
    const hook = renderCommitGraphHook();

    await hook.mount();

    await waitForExpectation(() => {
      expect(hook.latest.isError).toBe(true);
      expect(hook.latest.error).toEqual(
        new Error("Commit graph requires a newer Paseo daemon. Update or restart the daemon."),
      );
    });
  });
});

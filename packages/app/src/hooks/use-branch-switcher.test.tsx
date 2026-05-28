/**
 * @vitest-environment jsdom
 */
import React from "react";
import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { DaemonClient } from "@server/client/daemon-client";
import { useBranchSwitcher } from "./use-branch-switcher";

vi.mock("@/utils/confirm-dialog", () => ({
  confirmDialog: vi.fn(),
}));

vi.mock("@/stores/checkout-git-actions-store", () => ({
  invalidateCheckoutGitQueriesForClient: vi.fn(async () => {}),
}));

function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
      },
    },
  });
}

interface BranchSwitcherClient {
  checkoutSwitchBranch: ReturnType<typeof vi.fn>;
  getBranchSuggestions: ReturnType<typeof vi.fn>;
  stashList: ReturnType<typeof vi.fn>;
}

function renderSwitcherHook(input: {
  client: BranchSwitcherClient;
  toast: {
    copied: ReturnType<typeof vi.fn>;
    error: ReturnType<typeof vi.fn>;
    show: ReturnType<typeof vi.fn>;
  };
}) {
  const queryClient = createQueryClient();
  const wrapper = ({ children }: { children: React.ReactNode }) =>
    React.createElement(QueryClientProvider, { client: queryClient }, children);

  return renderHook(
    () =>
      useBranchSwitcher({
        client: input.client as unknown as DaemonClient,
        normalizedServerId: "server-1",
        normalizedWorkspaceId: "/repo",
        currentBranchName: "main",
        isGitCheckout: true,
        isConnected: true,
        toast: input.toast,
        queryClient,
        text: {
          branchAlreadyCheckedOut:
            "That branch is already open in another worktree. Switch to the existing worktree or continue from a different branch.",
          close: "Close",
          failedStashChanges: "Failed to stash changes",
          failedSwitchBranch: "Failed to switch branch",
          later: "Later",
          restore: "Restore",
          restoreStashedChangesMessage: "Restore stashed changes?",
          restoreStashedChangesTitle: "Restore stashed changes?",
          stashAndSwitch: "Stash & Switch",
          stashBeforeSwitchMessage: "Stash before switching?",
          stashedChangesRestored: "Stashed changes restored",
          uncommittedChangesTitle: "Uncommitted changes",
        } as never,
      }),
    { wrapper },
  );
}

describe("useBranchSwitcher", () => {
  afterEach(() => {
    vi.clearAllMocks();
  });

  it("shows a friendly message when a branch is already checked out in another worktree", async () => {
    const toast = { copied: vi.fn(), error: vi.fn(), show: vi.fn() };
    const client = {
      getBranchSuggestions: vi.fn(),
      stashList: vi.fn(),
      checkoutSwitchBranch: vi.fn(async () => ({
        cwd: "/repo",
        success: false,
        branch: "test",
        error: {
          code: "NOT_ALLOWED" as const,
          message:
            'Branch "test" is already checked out in another worktree: C:/Users/test/workspaces/repo/test',
          reason: "BRANCH_ALREADY_CHECKED_OUT" as const,
        },
        requestId: "req",
      })),
    };

    const { result } = renderSwitcherHook({ client, toast });

    act(() => {
      result.current.handleBranchSelect("test");
    });

    await waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith(
        "That branch is already open in another worktree. Switch to the existing worktree or continue from a different branch.",
      );
    });
  });
});

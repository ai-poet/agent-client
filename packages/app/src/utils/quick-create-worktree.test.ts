import { describe, expect, it, vi } from "vitest";
import type { DaemonClient } from "@server/client/daemon-client";
import type { WorkspaceDescriptor } from "@/stores/session-store";
import {
  buildCreatingWorktreePlaceholder,
  createWorktreeQuickly,
  isCreatingWorktreePlaceholderId,
} from "./quick-create-worktree";

vi.mock("mnemonic-id", () => ({
  createNameId: () => "eager-squid",
}));

describe("quick-create-worktree", () => {
  it("builds a creating placeholder under the current git project", () => {
    const placeholder = buildCreatingWorktreePlaceholder({
      sourceDirectory: "/repo",
      projectId: "project-1",
      projectDisplayName: "Repo",
      projectRootPath: "/repo",
    });

    expect(placeholder.id).toBe("__creating_worktree__:eager-squid");
    expect(placeholder.name).toBe("eager-squid");
    expect(placeholder.projectId).toBe("project-1");
    expect(placeholder.workspaceKind).toBe("worktree");
    expect(placeholder.status).toBe("running");
    expect(isCreatingWorktreePlaceholderId(placeholder.id)).toBe(true);
  });

  it("inserts a placeholder, then replaces it with the real workspace on success", async () => {
    const mergeWorkspaces = vi.fn();
    const removeWorkspace = vi.fn();
    const toast = { error: vi.fn() };
    const client = {
      createPaseoWorktree: vi.fn(async () => ({
        requestId: "req-1",
        setupTerminalId: null,
        error: null,
        workspace: {
          id: "/repo/.paseo/worktrees/eager-squid",
          projectId: "project-1",
          projectDisplayName: "Repo",
          projectRootPath: "/repo",
          workspaceDirectory: "/repo/.paseo/worktrees/eager-squid",
          projectKind: "git",
          workspaceKind: "worktree",
          name: "eager-squid",
          status: "done",
          diffStat: null,
          scripts: [],
        },
      })),
    } as unknown as Pick<DaemonClient, "createPaseoWorktree">;
    const onCreated = vi.fn();
    const onPlaceholderCreated = vi.fn();

    const result = await createWorktreeQuickly({
      client,
      isConnected: true,
      serverId: "server-1",
      sourceDirectory: "/repo",
      projectId: "project-1",
      projectDisplayName: "Repo",
      projectRootPath: "/repo",
      mergeWorkspaces,
      removeWorkspace,
      toast,
      onPlaceholderCreated,
      onCreated,
    });

    expect(client.createPaseoWorktree).toHaveBeenCalledWith({
      cwd: "/repo",
      worktreeSlug: "eager-squid",
    });
    expect(mergeWorkspaces).toHaveBeenNthCalledWith(
      1,
      "server-1",
      expect.arrayContaining([
        expect.objectContaining({
          id: "__creating_worktree__:eager-squid",
          name: "eager-squid",
          status: "running",
        }),
      ]),
    );
    expect(removeWorkspace).toHaveBeenCalledWith("server-1", "__creating_worktree__:eager-squid");
    expect(mergeWorkspaces).toHaveBeenNthCalledWith(
      2,
      "server-1",
      expect.arrayContaining([
        expect.objectContaining({
          id: "/repo/.paseo/worktrees/eager-squid",
          name: "eager-squid",
          status: "done",
        }),
      ]),
    );
    expect(onPlaceholderCreated).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "__creating_worktree__:eager-squid",
      }),
    );
    expect(onCreated).toHaveBeenCalledWith({
      workspace: expect.objectContaining({
        id: "/repo/.paseo/worktrees/eager-squid",
      }),
      placeholderWorkspaceId: "__creating_worktree__:eager-squid",
    });
    expect(toast.error).not.toHaveBeenCalled();
    expect(result?.id).toBe("/repo/.paseo/worktrees/eager-squid");
  });

  it("forwards an explicit base ref when creating a worktree", async () => {
    const mergeWorkspaces = vi.fn();
    const removeWorkspace = vi.fn();
    const toast = { error: vi.fn() };
    const client = {
      createPaseoWorktree: vi.fn(async () => ({
        requestId: "req-1",
        setupTerminalId: null,
        error: null,
        workspace: {
          id: "/repo/.paseo/worktrees/eager-squid",
          projectId: "project-1",
          projectDisplayName: "Repo",
          projectRootPath: "/repo",
          workspaceDirectory: "/repo/.paseo/worktrees/eager-squid",
          projectKind: "git",
          workspaceKind: "worktree",
          name: "eager-squid",
          status: "done",
          diffStat: null,
          scripts: [],
        },
      })),
    } as unknown as Pick<DaemonClient, "createPaseoWorktree">;

    await createWorktreeQuickly({
      client,
      isConnected: true,
      serverId: "server-1",
      sourceDirectory: "/repo",
      refName: "feature/base",
      projectId: "project-1",
      projectDisplayName: "Repo",
      projectRootPath: "/repo",
      mergeWorkspaces,
      removeWorkspace,
      toast,
    });

    expect(client.createPaseoWorktree).toHaveBeenCalledWith({
      cwd: "/repo",
      worktreeSlug: "eager-squid",
      refName: "feature/base",
    });
  });

  it("removes the placeholder and shows an error when creation fails", async () => {
    const mergeWorkspaces = vi.fn();
    const removeWorkspace = vi.fn();
    const toast = { error: vi.fn() };
    const client = {
      createPaseoWorktree: vi.fn(async () => ({
        requestId: "req-2",
        setupTerminalId: null,
        error: "boom",
        workspace: null,
      })),
    } as unknown as Pick<DaemonClient, "createPaseoWorktree">;

    const result = await createWorktreeQuickly({
      client,
      isConnected: true,
      serverId: "server-1",
      sourceDirectory: "/repo",
      projectId: "project-1",
      projectDisplayName: "Repo",
      projectRootPath: "/repo",
      mergeWorkspaces,
      removeWorkspace,
      toast,
    });

    expect(removeWorkspace).toHaveBeenCalledWith("server-1", "__creating_worktree__:eager-squid");
    expect(toast.error).toHaveBeenCalledWith("boom");
    expect(result).toBeNull();
  });

  it("cleans up duplicate creating placeholders when the real workspace arrives", async () => {
    const mergeWorkspaces = vi.fn();
    const removeWorkspace = vi.fn();
    const toast = { error: vi.fn() };
    const setWorkspaces = vi.fn();
    const client = {
      createPaseoWorktree: vi.fn(async () => ({
        requestId: "req-3",
        setupTerminalId: null,
        error: null,
        workspace: {
          id: "/repo/.paseo/worktrees/eager-squid",
          projectId: "project-1",
          projectDisplayName: "Repo",
          projectRootPath: "/repo",
          workspaceDirectory: "/repo/.paseo/worktrees/eager-squid",
          projectKind: "git",
          workspaceKind: "worktree",
          name: "eager-squid",
          status: "done",
          diffStat: null,
          scripts: [],
        },
      })),
    } as unknown as Pick<DaemonClient, "createPaseoWorktree">;

    const current = new Map<string, WorkspaceDescriptor>([
      [
        "__creating_worktree__:eager-squid",
        {
          id: "__creating_worktree__:eager-squid",
          projectId: "project-1",
          projectDisplayName: "Repo",
          projectRootPath: "/repo",
          workspaceDirectory: "/repo/.paseo/pending/eager-squid",
          projectKind: "git",
          workspaceKind: "worktree",
          name: "eager-squid",
          status: "running",
          diffStat: null,
          scripts: [],
        },
      ],
      [
        "__creating_worktree__:stale-eager-squid",
        {
          id: "__creating_worktree__:stale-eager-squid",
          projectId: "project-1",
          projectDisplayName: "Repo",
          projectRootPath: "/repo",
          workspaceDirectory: "/repo/.paseo/pending/eager-squid",
          projectKind: "git",
          workspaceKind: "worktree",
          name: "eager-squid",
          status: "running",
          diffStat: null,
          scripts: [],
        },
      ],
    ]);

    setWorkspaces.mockImplementation(
      (
        _serverId: string,
        update:
          | Map<string, WorkspaceDescriptor>
          | ((prev: Map<string, WorkspaceDescriptor>) => Map<string, WorkspaceDescriptor>),
      ) => {
        const next = typeof update === "function" ? update(current) : update;
        current.clear();
        for (const [key, value] of next.entries()) {
          current.set(key, value);
        }
      },
    );

    await createWorktreeQuickly({
      client,
      isConnected: true,
      serverId: "server-1",
      sourceDirectory: "/repo",
      projectId: "project-1",
      projectDisplayName: "Repo",
      projectRootPath: "/repo",
      mergeWorkspaces,
      removeWorkspace,
      setWorkspaces,
      toast,
    });

    expect(Array.from(current.keys())).toEqual(["/repo/.paseo/worktrees/eager-squid"]);
    expect(current.get("/repo/.paseo/worktrees/eager-squid")?.name).toBe("eager-squid");
    expect(removeWorkspace).not.toHaveBeenCalled();
  });
});

import { createNameId } from "mnemonic-id";
import type { DaemonClient } from "@server/client/daemon-client";
import type { ToastApi } from "@/components/toast-host";
import { normalizeWorkspaceDescriptor, type WorkspaceDescriptor } from "@/stores/session-store";
import { toErrorMessage } from "@/utils/error-messages";

const CREATING_WORKTREE_ID_PREFIX = "__creating_worktree__:";

function buildPlaceholderWorkspaceDirectory(sourceDirectory: string, slug: string): string {
  const separator = sourceDirectory.includes("\\") ? "\\" : "/";
  const trimmedSourceDirectory = sourceDirectory.replace(/[\\/]+$/g, "");
  return `${trimmedSourceDirectory}${separator}.paseo${separator}pending${separator}${slug}`;
}

export function buildCreatingWorktreePlaceholder(input: {
  sourceDirectory: string;
  projectId: string;
  projectDisplayName: string;
  projectRootPath: string;
}): WorkspaceDescriptor {
  const slug = createNameId();
  return {
    id: `${CREATING_WORKTREE_ID_PREFIX}${slug}`,
    projectId: input.projectId,
    projectDisplayName: input.projectDisplayName,
    projectRootPath: input.projectRootPath,
    workspaceDirectory: buildPlaceholderWorkspaceDirectory(input.sourceDirectory, slug),
    projectKind: "git",
    workspaceKind: "worktree",
    name: slug,
    status: "running",
    diffStat: null,
    scripts: [],
  };
}

export function isCreatingWorktreePlaceholderId(workspaceId: string): boolean {
  return workspaceId.startsWith(CREATING_WORKTREE_ID_PREFIX);
}

function extractCreatingSlug(workspaceId: string): string | null {
  if (!isCreatingWorktreePlaceholderId(workspaceId)) {
    return null;
  }
  return workspaceId.slice(CREATING_WORKTREE_ID_PREFIX.length);
}

function workspaceDirectoryHasSlug(workspaceDirectory: string, slug: string): boolean {
  const normalizedDirectory = workspaceDirectory.replace(/\\/g, "/");
  return normalizedDirectory.endsWith(`/${slug}`);
}

export async function createWorktreeQuickly(input: {
  client: Pick<DaemonClient, "createPaseoWorktree"> | null;
  isConnected: boolean;
  serverId: string;
  sourceDirectory: string;
  refName?: string;
  projectId: string;
  projectDisplayName: string;
  projectRootPath: string;
  mergeWorkspaces: (serverId: string, workspaces: Iterable<WorkspaceDescriptor>) => void;
  removeWorkspace: (serverId: string, workspaceId: string) => void;
  setWorkspaces?: (
    serverId: string,
    state:
      | Map<string, WorkspaceDescriptor>
      | ((prev: Map<string, WorkspaceDescriptor>) => Map<string, WorkspaceDescriptor>),
  ) => void;
  toast: Pick<ToastApi, "error">;
  onCreated?: (input: { workspace: WorkspaceDescriptor; placeholderWorkspaceId: string }) => void;
  onPlaceholderCreated?: (workspace: WorkspaceDescriptor) => void;
}): Promise<WorkspaceDescriptor | null> {
  if (!input.client || !input.isConnected) {
    input.toast.error("Host is not connected");
    return null;
  }

  const placeholder = buildCreatingWorktreePlaceholder({
    sourceDirectory: input.sourceDirectory,
    projectId: input.projectId,
    projectDisplayName: input.projectDisplayName,
    projectRootPath: input.projectRootPath,
  });

  input.mergeWorkspaces(input.serverId, [placeholder]);
  input.onPlaceholderCreated?.(placeholder);

  try {
    const payload = await input.client.createPaseoWorktree({
      cwd: input.sourceDirectory,
      worktreeSlug: placeholder.name,
      ...(input.refName ? { refName: input.refName } : {}),
    });

    if (payload.error || !payload.workspace) {
      throw new Error(payload.error ?? "Failed to create worktree");
    }

    const normalizedWorkspace = normalizeWorkspaceDescriptor(payload.workspace);
    if (input.setWorkspaces) {
      input.setWorkspaces(input.serverId, (prev) => {
        const next = new Map(prev);
        next.delete(placeholder.id);
        const placeholderSlug = extractCreatingSlug(placeholder.id);
        for (const [workspaceId, workspace] of next) {
          if (!isCreatingWorktreePlaceholderId(workspaceId)) {
            continue;
          }
          if (workspace.projectId !== normalizedWorkspace.projectId) {
            continue;
          }
          const sameName = workspace.name === normalizedWorkspace.name;
          const sameSlug =
            placeholderSlug !== null &&
            workspaceDirectoryHasSlug(normalizedWorkspace.workspaceDirectory, placeholderSlug);
          if (sameName || sameSlug) {
            next.delete(workspaceId);
          }
        }
        next.set(normalizedWorkspace.id, normalizedWorkspace);
        return next;
      });
    } else {
      input.removeWorkspace(input.serverId, placeholder.id);
      input.mergeWorkspaces(input.serverId, [normalizedWorkspace]);
    }
    input.onCreated?.({
      workspace: normalizedWorkspace,
      placeholderWorkspaceId: placeholder.id,
    });
    return normalizedWorkspace;
  } catch (error) {
    input.removeWorkspace(input.serverId, placeholder.id);
    input.toast.error(toErrorMessage(error));
    return null;
  }
}

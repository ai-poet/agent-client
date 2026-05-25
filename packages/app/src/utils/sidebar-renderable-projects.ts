import type {
  SidebarProjectEntry,
  SidebarWorkspaceEntry,
} from "@/hooks/use-sidebar-workspaces-list";
import { isCreatingWorktreePlaceholderId } from "@/utils/quick-create-worktree";

function normalizeWorkspaceName(name: string): string {
  return name.trim().toLocaleLowerCase();
}

function extractCreatingSlug(workspaceId: string): string | null {
  const separatorIndex = workspaceId.indexOf(":");
  if (separatorIndex < 0 || separatorIndex >= workspaceId.length - 1) {
    return null;
  }
  return workspaceId.slice(separatorIndex + 1);
}

function workspaceDirectoryHasSlug(workspaceDirectory: string | undefined, slug: string): boolean {
  if (!workspaceDirectory) {
    return false;
  }
  const normalizedDirectory = workspaceDirectory.replace(/\\/g, "/");
  return normalizedDirectory.endsWith(`/${slug}`);
}

function findPlaceholderReplacement(
  placeholder: SidebarWorkspaceEntry,
  realWorkspaces: readonly SidebarWorkspaceEntry[],
): SidebarWorkspaceEntry | null {
  const normalizedPlaceholderName = normalizeWorkspaceName(placeholder.name);
  const byName = realWorkspaces.find(
    (workspace) => normalizeWorkspaceName(workspace.name) === normalizedPlaceholderName,
  );
  if (byName) {
    return byName;
  }
  const placeholderSlug = extractCreatingSlug(placeholder.workspaceId);
  if (!placeholderSlug) {
    return null;
  }
  return (
    realWorkspaces.find((workspace) =>
      workspaceDirectoryHasSlug(workspace.workspaceDirectory, placeholderSlug),
    ) ?? null
  );
}

export function getSidebarRenderableProjects(
  projects: SidebarProjectEntry[],
): SidebarProjectEntry[] {
  let didChange = false;
  const nextProjects = projects.map((project) => {
    const realWorkspaces = project.workspaces.filter(
      (workspace) => !isCreatingWorktreePlaceholderId(workspace.workspaceId),
    );
    if (realWorkspaces.length === 0) {
      return project;
    }

    const filteredWorkspaces = project.workspaces.filter((workspace) => {
      if (!isCreatingWorktreePlaceholderId(workspace.workspaceId)) {
        return true;
      }
      return findPlaceholderReplacement(workspace, realWorkspaces) === null;
    });

    if (filteredWorkspaces.length === project.workspaces.length) {
      return project;
    }
    didChange = true;
    return {
      ...project,
      workspaces: filteredWorkspaces,
    };
  });

  return didChange ? nextProjects : projects;
}

export function findPlaceholderToRealWorkspaceReplacements(
  projects: readonly SidebarProjectEntry[],
): Map<string, SidebarWorkspaceEntry> {
  const replacements = new Map<string, SidebarWorkspaceEntry>();
  for (const project of projects) {
    const realWorkspaces = project.workspaces.filter(
      (workspace) => !isCreatingWorktreePlaceholderId(workspace.workspaceId),
    );
    if (realWorkspaces.length === 0) {
      continue;
    }
    for (const workspace of project.workspaces) {
      if (!isCreatingWorktreePlaceholderId(workspace.workspaceId)) {
        continue;
      }
      const replacement = findPlaceholderReplacement(workspace, realWorkspaces);
      if (replacement) {
        replacements.set(workspace.workspaceId, replacement);
      }
    }
  }
  return replacements;
}

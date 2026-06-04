import type { CheckoutStatusPayload } from "@/hooks/use-checkout-status-query";
import {
  parseHostWorkspaceOpenIntentFromPathname,
  buildHostWorkspaceRoute,
  parseHostAgentRouteFromPathname,
  parseHostWorkspaceRouteFromPathname,
} from "@/utils/host-routes";
import { resolveWorkspaceIdByExecutionDirectory } from "@/utils/workspace-execution";

type NewChatAgentLookup = {
  cwd: string;
};

type NewChatWorkspaceLookup = {
  id: string;
  workspaceDirectory: string;
};

export type NewChatTarget =
  | {
      kind: "workspace";
      serverId: string;
      workspaceId: string;
    }
  | {
      kind: "fallback";
      serverId: string | null;
      workingDir: string | null;
    };

type NewChatWorkspaceSelection = {
  serverId: string;
  workspaceId: string;
};

export function parseAgentKey(
  key: string | null | undefined,
): { serverId: string; agentId: string } | null {
  if (!key) {
    return null;
  }
  const sep = key.lastIndexOf(":");
  if (sep <= 0 || sep >= key.length - 1) {
    return null;
  }
  const serverId = key.slice(0, sep).trim();
  const agentId = key.slice(sep + 1).trim();
  if (!serverId || !agentId) {
    return null;
  }
  return { serverId, agentId };
}

export function resolveSelectedAgentForNewAgent(input: {
  pathname: string;
  selectedAgentId?: string;
}): { serverId: string; agentId: string } | null {
  const workspaceRoute = parseHostWorkspaceRouteFromPathname(input.pathname);
  const openIntent = parseHostWorkspaceOpenIntentFromPathname(input.pathname);
  if (workspaceRoute && openIntent?.kind === "agent") {
    const agentId = openIntent.agentId.trim();
    if (agentId) {
      return { serverId: workspaceRoute.serverId, agentId };
    }
  }
  return parseHostAgentRouteFromPathname(input.pathname) ?? parseAgentKey(input.selectedAgentId);
}

function inferMainRepoRootFromPaseoWorktreePath(cwd: string): string | null {
  const normalizedPath = cwd.replace(/\\/g, "/");
  const marker = "/.paseo/worktrees";
  const markerIndex = normalizedPath.indexOf(marker);
  if (markerIndex <= 0) {
    return null;
  }
  const markerEnd = markerIndex + marker.length;
  const nextChar = normalizedPath[markerEnd];
  if (nextChar && nextChar !== "/") {
    return null;
  }
  const inferred = cwd.slice(0, markerIndex).replace(/[\\/]+$/, "");
  return inferred.trim() ? inferred : null;
}

export function resolveNewAgentWorkingDir(
  cwd: string,
  checkout: CheckoutStatusPayload | null,
): string {
  const explicitMainRepoRoot = checkout?.isPaseoOwnedWorktree
    ? checkout.mainRepoRoot?.trim() || null
    : null;
  if (explicitMainRepoRoot) {
    return explicitMainRepoRoot;
  }

  return inferMainRepoRootFromPaseoWorktreePath(cwd) ?? cwd;
}

export function buildNewAgentRoute(serverId: string, workingDir?: string | null) {
  const trimmedWorkingDir = workingDir?.trim();
  if (!trimmedWorkingDir) {
    return buildHostWorkspaceRoute(serverId, "__new__");
  }
  return buildHostWorkspaceRoute(serverId, trimmedWorkingDir);
}

export function resolveNewChatTarget(input: {
  pathname: string;
  selectedAgentId?: string;
  activeServerId?: string | null;
  recentWorkspace?: NewChatWorkspaceSelection | null;
  fallbackWorkspace?: NewChatWorkspaceSelection | null;
  getAgent: (serverId: string, agentId: string) => NewChatAgentLookup | null | undefined;
  getWorkspaces: (serverId: string) => Iterable<NewChatWorkspaceLookup> | null | undefined;
}): NewChatTarget {
  const workspaceRoute = parseHostWorkspaceRouteFromPathname(input.pathname);
  if (workspaceRoute) {
    return {
      kind: "workspace",
      serverId: workspaceRoute.serverId,
      workspaceId: workspaceRoute.workspaceId,
    };
  }

  const selectedAgent = resolveSelectedAgentForNewAgent({
    pathname: input.pathname,
    selectedAgentId: input.selectedAgentId,
  });
  if (selectedAgent) {
    const agent = input.getAgent(selectedAgent.serverId, selectedAgent.agentId);
    const workingDir = agent?.cwd.trim() || null;
    if (!workingDir) {
      return {
        kind: "fallback",
        serverId: selectedAgent.serverId,
        workingDir: null,
      };
    }

    const workspaceId = resolveWorkspaceIdByExecutionDirectory({
      workspaces: input.getWorkspaces(selectedAgent.serverId),
      workspaceDirectory: workingDir,
    });
    if (workspaceId) {
      return {
        kind: "workspace",
        serverId: selectedAgent.serverId,
        workspaceId,
      };
    }

    return {
      kind: "fallback",
      serverId: selectedAgent.serverId,
      workingDir,
    };
  }

  const recentWorkspace = input.recentWorkspace;
  if (
    recentWorkspace?.serverId &&
    recentWorkspace.workspaceId &&
    (!input.activeServerId || recentWorkspace.serverId === input.activeServerId)
  ) {
    return {
      kind: "workspace",
      serverId: recentWorkspace.serverId,
      workspaceId: recentWorkspace.workspaceId,
    };
  }

  const fallbackWorkspace = input.fallbackWorkspace;
  if (
    fallbackWorkspace?.serverId &&
    fallbackWorkspace.workspaceId &&
    (!input.activeServerId || fallbackWorkspace.serverId === input.activeServerId)
  ) {
    return {
      kind: "workspace",
      serverId: fallbackWorkspace.serverId,
      workspaceId: fallbackWorkspace.workspaceId,
    };
  }

  return {
    kind: "fallback",
    serverId: input.activeServerId?.trim() || null,
    workingDir: null,
  };
}

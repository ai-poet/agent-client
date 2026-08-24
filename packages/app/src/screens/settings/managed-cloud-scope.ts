import type { Sub2APIGroup, Sub2APIKey } from "@/lib/sub2api-client";

/**
 * Cloud availability scope, one per gateway platform: Claude Code (anthropic) and
 * Codex (openai). Key/group availability is tracked per platform, so this stays a pair.
 */
export type ManagedCloudDesktopScope = "claude" | "codex";

/**
 * A CLI whose config we can write. Grok is not its own platform — it rides the openai
 * route — so it is a write target rather than an availability scope.
 */
export type ManagedRouteTarget = ManagedCloudDesktopScope | "grok";

export const MANAGED_CLOUD_SCOPE_META = {
  claude: {
    scope: "claude" as const,
    cliLabel: "Claude Code",
    platform: "anthropic",
    configTarget: "~/.claude/settings.json",
  },
  codex: {
    scope: "codex" as const,
    cliLabel: "Codex",
    platform: "openai",
    configTarget: "~/.codex/config.toml + ~/.codex/auth.json",
  },
  grok: {
    scope: "grok" as const,
    cliLabel: "Grok",
    platform: "openai",
    configTarget: "~/.agent-client/agent-runtimes/grok/config.toml",
  },
};

/**
 * Scopes an `openai` key can serve. Codex is the inferred default because Grok additionally
 * requires `grok-*` models to exist on the route, so it stays an explicit choice.
 */
export function getRouteTargetsForPlatform(platform: string): ManagedRouteTarget[] {
  const normalized = platform.trim().toLowerCase();
  if (normalized === MANAGED_CLOUD_SCOPE_META.claude.platform) {
    return ["claude"];
  }
  if (normalized === MANAGED_CLOUD_SCOPE_META.codex.platform) {
    return ["codex", "grok"];
  }
  return [];
}

export type ManagedCloudRouteResolution =
  | { ok: true; scope: ManagedCloudDesktopScope; cliLabel: string }
  | { ok: false; reason: string };

export function getManagedCloudMetaForScope(scope: ManagedCloudDesktopScope) {
  return MANAGED_CLOUD_SCOPE_META[scope];
}

/**
 * Map Sub2API group `platform` to the local CLI we configure.
 * Backend uses `anthropic` / `openai` (see domain.PlatformAnthropic / PlatformOpenAI).
 */
export function resolveManagedCloudRouteFromPlatform(
  platform: string,
): ManagedCloudRouteResolution {
  const p = platform.trim().toLowerCase();
  if (p === MANAGED_CLOUD_SCOPE_META.claude.platform) {
    return {
      ok: true,
      scope: MANAGED_CLOUD_SCOPE_META.claude.scope,
      cliLabel: MANAGED_CLOUD_SCOPE_META.claude.cliLabel,
    };
  }
  if (p === MANAGED_CLOUD_SCOPE_META.codex.platform) {
    return {
      ok: true,
      scope: MANAGED_CLOUD_SCOPE_META.codex.scope,
      cliLabel: MANAGED_CLOUD_SCOPE_META.codex.cliLabel,
    };
  }
  return {
    ok: false,
    reason: `This group uses platform "${platform}". Only anthropic (Claude Code) and openai (Codex) can be configured automatically on this device.`,
  };
}

export function resolveManagedCloudRouteForKey(
  key: Sub2APIKey,
  groups: Sub2APIGroup[],
): ManagedCloudRouteResolution {
  const platform = key.group?.platform ?? groups.find((g) => g.id === key.group_id)?.platform ?? "";
  if (!platform) {
    return {
      ok: false,
      reason: "This key has no group or platform; cannot choose Claude Code vs Codex.",
    };
  }
  return resolveManagedCloudRouteFromPlatform(platform);
}

export function resolveManagedCloudRouteForGroup(group: Sub2APIGroup): ManagedCloudRouteResolution {
  return resolveManagedCloudRouteFromPlatform(group.platform);
}

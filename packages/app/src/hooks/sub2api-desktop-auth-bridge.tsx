/**
 * Listens for Electron sub2api OAuth return (pending URL on cold start + auth-callback events).
 * Must mount at app root: when only / (startup) is shown, /login is not mounted and
 * useSub2APILoginFlow would never register the listeners otherwise.
 */
import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef } from "react";
import { Alert } from "react-native";
import { getIsElectron } from "@/constants/platform";
import { invokeDesktopCommand } from "@/desktop/electron/invoke";
import { getDesktopHost } from "@/desktop/host";
import { useSub2APIAuth } from "@/hooks/use-sub2api-auth";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { cloudServiceQueryKeys } from "@/hooks/use-sub2api-api";
import { getSub2APIMessages } from "@/i18n/sub2api";
import { createSub2APIClient, type Sub2APIGroup, type Sub2APIKey } from "@/lib/sub2api-client";
import { parseSub2APIAuthCallback } from "@/screens/settings/sub2api-auth-bridge";
import { resolveScopedActiveProviderIds } from "@/screens/settings/desktop-providers-context";
import {
  resolveManagedCloudRouteForGroup,
  resolveManagedCloudRouteForKey,
  type ManagedCloudDesktopScope,
} from "@/screens/settings/managed-cloud-scope";
import { findReusableKey } from "@/screens/settings/managed-provider-settings-shared";
import type {
  DesktopProviderPayload,
  ManagedProviderTarget,
  ProviderStore,
} from "@/screens/settings/sub2api-provider-types";
import {
  defaultModelsForTarget,
  fetchGatewayModelIds,
  filterGatewayModelsForTarget,
} from "@/screens/settings/gateway-models";
import { CLOUD_NAME } from "@/config/branding";

let lastHandledCallbackUrl: string | null = null;

interface AuthCallbackPayload {
  url: string;
}

type SetupDefaultProviderPayload = {
  endpoint: string;
  apiKey: string;
  scope: ManagedProviderTarget | "both";
  name: string;
  /** Gateway model ids; only Grok and Pi embed an explicit list in their config. */
  models?: string[];
};

function getErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function extractAuthCallbackUrl(payload: unknown): string | null {
  if (typeof payload !== "object" || payload === null) {
    return null;
  }
  const url = (payload as AuthCallbackPayload).url;
  return typeof url === "string" && url.length > 0 ? url : null;
}

function normalizeProviderEndpoint(value: string | null | undefined): string {
  const trimmed = value?.trim().replace(/\/+$/, "") ?? "";
  return trimmed.toLowerCase().endsWith("/v1") ? trimmed.slice(0, -3) : trimmed;
}

function providerUsesSignedInCloudKey(input: {
  provider: DesktopProviderPayload | null | undefined;
  scope: ManagedCloudDesktopScope;
  endpoint: string;
  keys: Sub2APIKey[];
  groups: Sub2APIGroup[];
}): boolean {
  const provider = input.provider;
  if (!provider || (provider.target && provider.target !== input.scope)) {
    return false;
  }

  const providerEndpoint = normalizeProviderEndpoint(provider.endpoint);
  const signedInEndpoint = normalizeProviderEndpoint(input.endpoint);
  const providerKey = provider.apiKey.trim();
  if (
    !providerEndpoint ||
    !signedInEndpoint ||
    providerEndpoint !== signedInEndpoint ||
    !providerKey
  ) {
    return false;
  }

  const cloudKey = input.keys.find((key) => key.key.trim() === providerKey);
  if (!cloudKey) {
    return false;
  }

  const route = resolveManagedCloudRouteForKey(cloudKey, input.groups);
  return route.ok && route.scope === input.scope;
}

function getActiveProviderForScope(
  store: ProviderStore,
  scope: ManagedCloudDesktopScope,
): DesktopProviderPayload | null {
  const active = resolveScopedActiveProviderIds(store);
  const id = active[scope];
  return id ? (store.providers.find((provider) => provider.id === id) ?? null) : null;
}

function chooseGroupForScope(input: {
  scope: ManagedCloudDesktopScope;
  groups: Sub2APIGroup[];
  keys: Sub2APIKey[];
}): Sub2APIGroup | null {
  const candidates = input.groups.filter((group) => {
    const route = resolveManagedCloudRouteForGroup(group);
    return route.ok && route.scope === input.scope;
  });
  if (candidates.length === 0) {
    return null;
  }

  return candidates.sort((a, b) => {
    const aHasKey = findReusableKey(input.keys, a.id) ? 1 : 0;
    const bHasKey = findReusableKey(input.keys, b.id) ? 1 : 0;
    if (aHasKey !== bHasKey) {
      return bHasKey - aHasKey;
    }

    const aActive = a.status === "active" ? 1 : 0;
    const bActive = b.status === "active" ? 1 : 0;
    if (aActive !== bActive) {
      return bActive - aActive;
    }

    const priceDelta = a.rate_multiplier - b.rate_multiplier;
    if (priceDelta !== 0) {
      return priceDelta;
    }

    return a.name.localeCompare(b.name) || a.id - b.id;
  })[0];
}

async function resolveCloudKeyForScope(input: {
  scope: ManagedCloudDesktopScope;
  groups: Sub2APIGroup[];
  keys: Sub2APIKey[];
  createKey: (group: Sub2APIGroup) => Promise<Sub2APIKey>;
}): Promise<{ key: Sub2APIKey; group: Sub2APIGroup } | null> {
  const group = chooseGroupForScope({
    scope: input.scope,
    groups: input.groups,
    keys: input.keys,
  });
  if (!group) {
    return null;
  }

  const reusable = findReusableKey(input.keys, group.id);
  return {
    group,
    key: reusable ?? (await input.createKey(group)),
  };
}

/**
 * Targets that ride the OpenAI-compatible route rather than owning a gateway platform.
 * Pi is omitted while it is hidden — see MANAGED_PROVIDER_TARGETS.
 */
const GATEWAY_AGENT_TARGETS = ["grok"] as const;

function resolveOpenAiRoute(
  commands: SetupDefaultProviderPayload[],
  store: ProviderStore,
): { endpoint: string; apiKey: string; name: string } | null {
  const queued = commands.find((command) => command.scope === "codex" || command.scope === "both");
  if (queued) {
    return { endpoint: queued.endpoint, apiKey: queued.apiKey, name: queued.name };
  }
  // Codex may already be correctly configured from an earlier login that predates Grok/Pi
  // support; reuse its route so those two still get set up.
  const active = getActiveProviderForScope(store, "codex");
  return active ? { endpoint: active.endpoint, apiKey: active.apiKey, name: active.name } : null;
}

/**
 * Grok and Pi share Codex's OpenAI-compatible route and key. Their configs embed an explicit
 * model list, so the gateway catalog has to be read before the desktop can write them.
 */
async function withGatewayAgentCommands(
  commands: SetupDefaultProviderPayload[],
  store: ProviderStore,
): Promise<SetupDefaultProviderPayload[]> {
  const route = resolveOpenAiRoute(commands, store);
  if (!route) {
    return commands;
  }

  const activeIds = resolveScopedActiveProviderIds(store);
  const needsSetup = GATEWAY_AGENT_TARGETS.filter((target) => {
    const activeId = activeIds[target];
    if (!activeId) {
      return true;
    }
    // Already configured — only rewrite when the key behind it has rotated.
    const row = store.providers.find((provider) => provider.id === activeId);
    return row?.apiKey?.trim() !== route.apiKey.trim();
  });
  if (needsSetup.length === 0) {
    return commands;
  }

  let models: string[] = [];
  try {
    models = await fetchGatewayModelIds({ endpoint: route.endpoint, apiKey: route.apiKey });
  } catch (error) {
    // Still write the configs with defaults below: the endpoint and key are known good here,
    // and a file on disk is what the user can edit if the defaults are wrong.
    console.warn("[sub2api] gateway catalog unavailable, using default models", error);
  }

  const extra: SetupDefaultProviderPayload[] = [];
  for (const scope of needsSetup) {
    const usable = filterGatewayModelsForTarget(scope, models);
    extra.push({
      endpoint: route.endpoint,
      apiKey: route.apiKey,
      scope,
      name: route.name,
      models: usable.length > 0 ? usable : defaultModelsForTarget(scope),
    });
  }
  return [...commands, ...extra];
}

async function configureMissingManagedRoutes(
  session: ReturnType<typeof parseSub2APIAuthCallback>,
  getAccessToken: () => Promise<string | null>,
) {
  const store = await invokeDesktopCommand<ProviderStore>("get_providers");
  const active = resolveScopedActiveProviderIds(store);

  const commands: SetupDefaultProviderPayload[] = [];
  const pendingScopes = new Set<ManagedCloudDesktopScope>(["claude", "codex"]);

  if (session.claudeApiKey) {
    commands.push({
      endpoint: session.endpoint,
      apiKey: session.claudeApiKey,
      scope: "claude",
      name: CLOUD_NAME,
    });
    pendingScopes.delete("claude");
  }
  if (session.codexApiKey) {
    commands.push({
      endpoint: session.endpoint,
      apiKey: session.codexApiKey,
      scope: "codex",
      name: CLOUD_NAME,
    });
    pendingScopes.delete("codex");
  }

  if (pendingScopes.size > 0) {
    const client = createSub2APIClient({ endpoint: session.endpoint, getAccessToken });
    const [keyResult, groups] = await Promise.all([
      client.listKeys(1, 200),
      client.getAvailableGroups(),
    ]);
    const keys = [...keyResult.items];

    for (const scope of [...pendingScopes]) {
      const provider = getActiveProviderForScope(store, scope);
      if (
        active[scope] &&
        providerUsesSignedInCloudKey({
          provider,
          scope,
          endpoint: session.endpoint,
          keys,
          groups,
        })
      ) {
        pendingScopes.delete(scope);
        continue;
      }

      const resolved = await resolveCloudKeyForScope({
        scope,
        groups,
        keys,
        createKey: async (group) => {
          const created = await client.createKey({
            name: `${group.name} Key`,
            group_id: group.id,
          });
          keys.push(created);
          return created;
        },
      });
      if (!resolved) {
        continue;
      }

      commands.push({
        endpoint: session.endpoint,
        apiKey: resolved.key.key,
        scope,
        name: resolved.group.name,
      });
      pendingScopes.delete(scope);
    }
  }

  if (commands.length === 0 && !active.claude && !active.codex && session.apiKey) {
    commands.push({
      endpoint: session.endpoint,
      apiKey: session.apiKey,
      scope: "both",
      name: CLOUD_NAME,
    });
  }

  const allCommands = await withGatewayAgentCommands(commands, store);
  if (allCommands.length === 0) {
    return;
  }

  for (const command of allCommands) {
    await invokeDesktopCommand("setup_default_provider", command);
  }
}

export function Sub2apiDesktopAuthBridge(): null {
  const queryClient = useQueryClient();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).authAlerts, [locale]);
  const { login, getAccessToken } = useSub2APIAuth();
  const loginRef = useRef(login);
  loginRef.current = login;
  const getAccessTokenRef = useRef(getAccessToken);
  getAccessTokenRef.current = getAccessToken;
  const queryClientRef = useRef(queryClient);
  queryClientRef.current = queryClient;

  const applyCallback = useCallback(
    async (payload: unknown) => {
      const url = extractAuthCallbackUrl(payload);
      if (!url) {
        return;
      }
      if (lastHandledCallbackUrl === url) {
        return;
      }
      lastHandledCallbackUrl = url;

      try {
        const session = parseSub2APIAuthCallback(url);
        await loginRef.current({
          accessToken: session.accessToken,
          refreshToken: session.refreshToken,
          expiresIn: session.expiresIn,
          endpoint: session.endpoint,
        });
        try {
          await configureMissingManagedRoutes(session, () => getAccessTokenRef.current());
        } catch (error) {
          console.error("[sub2api-desktop-auth-bridge] auto-route setup failed:", error);
          Alert.alert(text.signedIn, text.autoRouteSetupFailed(getErrorMessage(error)));
        }
        void queryClientRef.current.invalidateQueries({ queryKey: cloudServiceQueryKeys.root });
      } catch (error) {
        lastHandledCallbackUrl = null;
        console.error("[sub2api-desktop-auth-bridge] callback failed:", error);
        Alert.alert(text.loginFailed, getErrorMessage(error));
      }
    },
    [text],
  );

  const applyCallbackRef = useRef(applyCallback);
  applyCallbackRef.current = applyCallback;

  useEffect(() => {
    if (!getIsElectron()) {
      return;
    }

    const run = (payload: unknown) => {
      void applyCallbackRef.current(payload);
    };

    const subscribe = getDesktopHost()?.events?.on;
    if (typeof subscribe !== "function") {
      return;
    }

    let disposed = false;
    let cleanup: (() => void) | null = null;

    void Promise.resolve(subscribe("auth-callback", run))
      .then((unsubscribe) => {
        if (disposed) {
          unsubscribe();
          return;
        }
        cleanup = unsubscribe;
      })
      .catch((error) => {
        console.error("[sub2api-desktop-auth-bridge] subscribe failed:", error);
      });

    return () => {
      disposed = true;
      cleanup?.();
    };
  }, []);

  useEffect(() => {
    if (!getIsElectron()) {
      return;
    }
    const getPendingAuthCallback = getDesktopHost()?.getPendingAuthCallback;
    if (typeof getPendingAuthCallback !== "function") {
      return;
    }

    void getPendingAuthCallback()
      .then((url) => {
        if (!url) {
          return;
        }
        void applyCallbackRef.current({ url });
      })
      .catch((error) => {
        console.error("[sub2api-desktop-auth-bridge] pending callback failed:", error);
      });
  }, []);

  return null;
}

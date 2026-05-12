import { useCallback, useMemo, useRef, useState } from "react";
import { useRouter } from "expo-router";
import { getIsElectron } from "@/constants/platform";
import { useSub2APIAuth } from "@/hooks/use-sub2api-auth";
import { useAppSettings, type AccessMode } from "@/hooks/use-settings";
import { getManagedServiceUrlFromEnv } from "@/config/managed-service-env";
import { createSub2APIClient, type Sub2APIGroup, type Sub2APIKey } from "@/lib/sub2api-client";
import {
  getModelCliRuntimeStatus,
  installClaudeCodeCli,
  installAllModelClis,
  installCodexCli,
  installGitBashRuntime,
  installNode22Runtime,
  type ModelCliRuntimeStatus,
} from "@/desktop/daemon/desktop-daemon";
import { invokeDesktopCommand } from "@/desktop/electron/invoke";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";
import type { ProviderStore } from "@/screens/settings/sub2api-provider-types";
import { resolveScopedActiveProviderIds } from "@/screens/settings/desktop-providers-context";
import {
  resolveManagedCloudRouteForGroup,
  resolveManagedCloudRouteForKey,
} from "@/screens/settings/managed-cloud-scope";
import { CLOUD_NAME } from "@/config/branding";

export type CheckStatus = "pending" | "checking" | "passed" | "failed" | "skipped";

export interface CheckItem {
  id: "health" | "apiKeys" | "cliConfig";
  label: string;
  description: string;
  status: CheckStatus;
  error: string | null;
  fixLabel: string | null;
}

export interface UseSetupChecksReturn {
  checks: CheckItem[];
  allPassed: boolean;
  isRunning: boolean;
  runAllChecks: () => Promise<void>;
  fixCheck: (id: CheckItem["id"]) => Promise<void>;
}

const HEALTH_TIMEOUT_MS = 5000;
const MANAGED_CLOUD_ROUTES_UNAVAILABLE = "Managed cloud routes unavailable";
type SetupCheckText = ReturnType<typeof getSub2APIMessages>["setupCheck"];

export interface ManagedCloudScopeAvailability {
  groups: number;
  activeKeys: number;
}

export interface ManagedCloudAvailabilitySummary {
  claude: ManagedCloudScopeAvailability;
  codex: ManagedCloudScopeAvailability;
}

async function checkHealth(endpoint: string, text: SetupCheckText): Promise<void> {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), HEALTH_TIMEOUT_MS);
  try {
    const resp = await fetch(`${endpoint}/health`, { signal: controller.signal });
    const data = (await resp.json()) as { status?: string };
    if (data?.status !== "ok") {
      throw new Error(text.health.unexpectedStatus);
    }
  } finally {
    clearTimeout(timeout);
  }
}

function makeInitialChecks(accessMode: AccessMode | null, text: SetupCheckText): CheckItem[] {
  const isElectron = getIsElectron();
  return [
    {
      id: "health",
      label: text.health.label,
      description: text.health.checking,
      status: "pending",
      error: null,
      fixLabel: text.actions.retry,
    },
    {
      id: "apiKeys",
      label: accessMode === "byok" ? text.routes.apiKeyLabel : text.routes.cloudRoutesLabel,
      description: accessMode === "byok" ? text.routes.byokSkipped : text.routes.checking,
      status: accessMode === "byok" ? "skipped" : "pending",
      error: null,
      fixLabel: text.actions.manageRoutes,
    },
    {
      id: "cliConfig",
      label: text.cli.label,
      description: isElectron ? text.cli.checking : text.cli.desktopOnlySkipped,
      status: isElectron ? "pending" : "skipped",
      error: null,
      fixLabel: text.actions.install,
    },
  ];
}

export function summarizeManagedCloudAvailability(
  keys: Sub2APIKey[],
  groups: Sub2APIGroup[],
): ManagedCloudAvailabilitySummary {
  const summary: ManagedCloudAvailabilitySummary = {
    claude: { groups: 0, activeKeys: 0 },
    codex: { groups: 0, activeKeys: 0 },
  };

  for (const group of groups) {
    const route = resolveManagedCloudRouteForGroup(group);
    if (!route.ok) {
      continue;
    }
    summary[route.scope].groups += 1;
  }

  for (const key of keys) {
    if (key.status !== "active") {
      continue;
    }
    const route = resolveManagedCloudRouteForKey(key, groups);
    if (!route.ok) {
      continue;
    }
    summary[route.scope].activeKeys += 1;
  }

  return summary;
}

export function describeManagedCloudAvailability(
  summary: ManagedCloudAvailabilitySummary,
  totalKeys: number,
  totalGroups: number,
  text: SetupCheckText = getSub2APIMessages("en").setupCheck,
): Pick<CheckItem, "status" | "description" | "error" | "fixLabel"> {
  const claudeReady = summary.claude.activeKeys > 0 || summary.claude.groups > 0;
  const codexReady = summary.codex.activeKeys > 0 || summary.codex.groups > 0;

  if (claudeReady && codexReady) {
    return {
      status: "passed",
      description: text.routes.readyBoth,
      error: null,
      fixLabel: text.actions.manageRoutes,
    };
  }

  if (claudeReady) {
    return {
      status: "passed",
      description: text.routes.claudeOnly,
      error: null,
      fixLabel: text.actions.manageRoutes,
    };
  }

  if (codexReady) {
    return {
      status: "passed",
      description: text.routes.codexOnly,
      error: null,
      fixLabel: text.actions.manageRoutes,
    };
  }

  if (totalKeys > 0) {
    return {
      status: "failed",
      description: text.routes.keysNotBound,
      error: text.routes.assignGroup(CLOUD_NAME),
      fixLabel: text.actions.manageRoutes,
    };
  }

  if (totalGroups > 0) {
    return {
      status: "failed",
      description: text.routes.noCompatibleGroups,
      error: text.routes.askGroupOrByok,
      fixLabel: text.actions.manageRoutes,
    };
  }

  return {
    status: "failed",
    description: text.routes.noManagedRoutes,
    error: text.routes.createRouteOrByok,
    fixLabel: text.actions.manageRoutes,
  };
}

export function getMissingCliDependencyNames(status: ModelCliRuntimeStatus): string[] {
  const missing: string[] = [];
  if (!status.git.installed) missing.push("Git Bash");
  if (!status.node.installed || !status.node.satisfies) missing.push("Node.js 22");
  if (!status.claude.installed) missing.push("Claude Code");
  if (!status.codex.installed) missing.push("Codex");
  return missing;
}

function getErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function stripDesktopIpcPrefix(message: string): string {
  return message
    .replace(/^Error invoking remote method 'paseo:invoke':\s*/i, "")
    .replace(/^Error:\s*/i, "")
    .trim();
}

export function formatCliInstallFailureMessage(
  error: unknown,
  status: ModelCliRuntimeStatus | null,
  text: SetupCheckText = getSub2APIMessages("en").setupCheck,
): string {
  const missing = status ? getMissingCliDependencyNames(status) : [];
  const cleanedMessage = stripDesktopIpcPrefix(getErrorMessage(error));
  const baseMessage = cleanedMessage.startsWith("Install failed")
    ? cleanedMessage
    : text.cli.installFailedFallback;

  if (missing.length === 0) {
    return baseMessage;
  }

  if (/\bMissing:/i.test(baseMessage) || baseMessage.includes("缺少：")) {
    return baseMessage;
  }

  return text.cli.installFailureWithMissing(baseMessage, missing.join(", "));
}

interface CliInstallStep {
  id: "git" | "node" | "codex" | "claude";
  label: string;
  installingDescription: string;
  run: () => Promise<{ status: ModelCliRuntimeStatus }>;
}

export function getCliInstallSteps(
  text: SetupCheckText = getSub2APIMessages("en").setupCheck,
): CliInstallStep[] {
  return [
    {
      id: "git",
      label: "Git Bash",
      installingDescription: text.cli.installing.git,
      run: installGitBashRuntime,
    },
    {
      id: "node",
      label: "Node.js 22",
      installingDescription: text.cli.installing.node,
      run: installNode22Runtime,
    },
    {
      id: "codex",
      label: "Codex",
      installingDescription: text.cli.installing.codex,
      run: installCodexCli,
    },
    {
      id: "claude",
      label: "Claude Code",
      installingDescription: text.cli.installing.claude,
      run: installClaudeCodeCli,
    },
  ];
}

export function useSetupChecks(): UseSetupChecksReturn {
  const router = useRouter();
  const { settings } = useAppSettings();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).setupCheck, [locale]);
  const { auth, getAccessToken } = useSub2APIAuth();
  const isElectron = getIsElectron();

  const clientRef = useRef(
    settings.accessMode === "builtin" && auth?.endpoint
      ? createSub2APIClient({ endpoint: auth.endpoint, getAccessToken })
      : null,
  );

  const [checks, setChecks] = useState<CheckItem[]>(() =>
    makeInitialChecks(settings.accessMode, text),
  );
  const [isRunning, setIsRunning] = useState(false);
  const cliStatusRef = useRef<ModelCliRuntimeStatus | null>(null);

  const updateCheck = useCallback((id: CheckItem["id"], patch: Partial<CheckItem>) => {
    setChecks((prev) => prev.map((c) => (c.id === id ? { ...c, ...patch } : c)));
  }, []);

  const getEndpoint = useCallback(() => {
    return settings.accessMode === "builtin" && auth?.endpoint
      ? auth.endpoint
      : getManagedServiceUrlFromEnv();
  }, [auth?.endpoint, settings.accessMode]);

  const runHealthCheck = useCallback(async () => {
    updateCheck("health", { status: "checking", error: null });
    try {
      await checkHealth(getEndpoint(), text);
      updateCheck("health", { status: "passed", description: text.health.reachable });
    } catch (err) {
      updateCheck("health", {
        status: "failed",
        error: err instanceof Error ? err.message : text.health.connectionFailed,
        description: text.health.unreachable,
        fixLabel: text.actions.retry,
      });
      throw err;
    }
  }, [getEndpoint, text, updateCheck]);

  const runApiKeyCheck = useCallback(async () => {
    if (settings.accessMode === "byok") {
      updateCheck("apiKeys", { status: "skipped", description: text.routes.byokSkipped });
      return;
    }
    updateCheck("apiKeys", { status: "checking", error: null });
    try {
      const c = clientRef.current;
      if (!c) throw new Error(text.routes.notAuthenticated);
      const [keyResult, groups] = await Promise.all([c.listKeys(1, 200), c.getAvailableGroups()]);
      const availability = summarizeManagedCloudAvailability(keyResult.items, groups);
      const next = describeManagedCloudAvailability(
        availability,
        keyResult.total,
        groups.length,
        text,
      );
      updateCheck("apiKeys", next);
      if (next.status === "failed") {
        throw new Error(MANAGED_CLOUD_ROUTES_UNAVAILABLE);
      }
    } catch (err) {
      if ((err as Error).message !== MANAGED_CLOUD_ROUTES_UNAVAILABLE) {
        updateCheck("apiKeys", {
          status: "failed",
          error: err instanceof Error ? err.message : text.routes.checkFailed,
          description: text.routes.failedCheck,
          fixLabel: text.actions.manageRoutes,
        });
      }
      throw err;
    }
  }, [settings.accessMode, text, updateCheck]);

  const runCliConfigCheck = useCallback(async () => {
    if (!isElectron) {
      updateCheck("cliConfig", { status: "skipped", description: text.cli.desktopOnlySkipped });
      return;
    }
    updateCheck("cliConfig", { status: "checking", error: null });
    try {
      const status = await getModelCliRuntimeStatus();
      cliStatusRef.current = status;
      const missing = getMissingCliDependencyNames(status);

      if (missing.length > 0) {
        updateCheck("cliConfig", {
          status: "failed",
          error: text.cli.missing(missing.join(", ")),
          description: text.cli.toolsNotFullyInstalled,
          fixLabel: text.actions.installAll,
        });
        throw new Error(text.cli.notInstalledError);
      }

      // Check provider configuration
      const store = await invokeDesktopCommand<ProviderStore>("get_providers");
      const { claude, codex } = resolveScopedActiveProviderIds(store);
      if (!claude && !codex) {
        updateCheck("cliConfig", {
          status: "failed",
          error: text.cli.providersNotConfigured,
          description: text.cli.toolsInstalledButNotConfigured,
          fixLabel: text.actions.configure,
        });
        throw new Error(text.cli.providersNotConfiguredError);
      }
      const partial = !claude || !codex;
      if (partial) {
        const configured = claude ? "Claude Code" : "Codex";
        const unconfigured = !claude ? "Claude Code" : "Codex";
        updateCheck("cliConfig", {
          status: "failed",
          description: text.cli.onlyConfigured(configured),
          error: text.cli.providerNotConfigured(unconfigured),
          fixLabel: text.actions.configure,
        });
        throw new Error(text.cli.partialConfigError);
      }
      updateCheck("cliConfig", {
        status: "passed",
        description: text.cli.installedAndConfigured,
      });
    } catch (err) {
      const msg = (err as Error).message;
      if (
        msg !== text.cli.notInstalledError &&
        msg !== text.cli.providersNotConfiguredError &&
        msg !== text.cli.partialConfigError
      ) {
        updateCheck("cliConfig", {
          status: "failed",
          error: err instanceof Error ? err.message : text.cli.checkFailed,
          description: text.cli.failedCheckConfiguration,
          fixLabel: text.actions.retry,
        });
      }
      throw err;
    }
  }, [isElectron, text, updateCheck]);

  const runAllChecks = useCallback(async () => {
    setIsRunning(true);
    setChecks(makeInitialChecks(settings.accessMode, text));

    // Rebuild client ref in case auth changed
    if (settings.accessMode === "builtin" && auth?.endpoint) {
      clientRef.current = createSub2APIClient({ endpoint: auth.endpoint, getAccessToken });
    }

    try {
      await runHealthCheck();
    } catch {
      setIsRunning(false);
      return;
    }
    try {
      await runApiKeyCheck();
    } catch {
      // continue to CLI check
    }
    try {
      await runCliConfigCheck();
    } catch {
      // done
    }
    setIsRunning(false);
  }, [
    auth?.endpoint,
    getAccessToken,
    runApiKeyCheck,
    runCliConfigCheck,
    runHealthCheck,
    settings.accessMode,
    text,
  ]);

  const fixCheck = useCallback(
    async (id: CheckItem["id"]) => {
      switch (id) {
        case "health":
          updateCheck("health", { status: "checking", error: null });
          try {
            await checkHealth(getEndpoint(), text);
            updateCheck("health", { status: "passed", description: text.health.reachable });
          } catch (err) {
            updateCheck("health", {
              status: "failed",
              error: err instanceof Error ? err.message : text.health.connectionFailed,
            });
          }
          break;

        case "apiKeys":
          router.push("/settings/paseo-cloud");
          break;

        case "cliConfig": {
          const status = cliStatusRef.current;
          const needsInstall = status && getMissingCliDependencyNames(status).length > 0;
          if (needsInstall) {
            updateCheck("cliConfig", {
              status: "checking",
              error: null,
              description: text.cli.preparingInstall,
            });
            try {
              const installSteps = getCliInstallSteps(text);
              let result: { status: ModelCliRuntimeStatus } | null = null;
              for (const step of installSteps) {
                updateCheck("cliConfig", {
                  status: "checking",
                  error: null,
                  description: step.installingDescription,
                });
                result = await step.run();
                cliStatusRef.current = result.status;
                updateCheck("cliConfig", {
                  status: "checking",
                  error: null,
                  description: text.cli.stepReadyContinuing(step.label),
                });
              }
              result = result ?? (await installAllModelClis());
              cliStatusRef.current = result.status;
              const allInstalled = getMissingCliDependencyNames(result.status).length === 0;
              if (allInstalled) {
                await runCliConfigCheck();
              } else {
                updateCheck("cliConfig", {
                  status: "failed",
                  error: text.cli.installationIncomplete(
                    getMissingCliDependencyNames(result.status).join(", "),
                  ),
                  description: text.cli.someToolsFailed,
                  fixLabel: text.actions.retry,
                });
              }
            } catch (err) {
              updateCheck("cliConfig", {
                status: "failed",
                error: formatCliInstallFailureMessage(err, cliStatusRef.current, text),
                fixLabel: text.actions.retry,
              });
            }
          } else {
            router.push("/settings/managed-provider");
          }
          break;
        }
      }
    },
    [getEndpoint, router, runCliConfigCheck, text, updateCheck],
  );

  const allPassed = checks.every((c) => c.status === "passed" || c.status === "skipped");

  return { checks, allPassed, isRunning, runAllChecks, fixCheck };
}

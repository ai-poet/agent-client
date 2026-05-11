import { useCallback, useRef, useState } from "react";
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

export interface ManagedCloudScopeAvailability {
  groups: number;
  activeKeys: number;
}

export interface ManagedCloudAvailabilitySummary {
  claude: ManagedCloudScopeAvailability;
  codex: ManagedCloudScopeAvailability;
}

async function checkHealth(endpoint: string): Promise<void> {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), HEALTH_TIMEOUT_MS);
  try {
    const resp = await fetch(`${endpoint}/health`, { signal: controller.signal });
    const data = (await resp.json()) as { status?: string };
    if (data?.status !== "ok") {
      throw new Error("Service returned unexpected status");
    }
  } finally {
    clearTimeout(timeout);
  }
}

function makeInitialChecks(accessMode: AccessMode | null): CheckItem[] {
  const isElectron = getIsElectron();
  return [
    {
      id: "health",
      label: "Backend Service",
      description: "Checking service reachability...",
      status: "pending",
      error: null,
      fixLabel: "Retry",
    },
    {
      id: "apiKeys",
      label: accessMode === "byok" ? "API Key" : "Cloud routes",
      description:
        accessMode === "byok" ? "BYOK mode — skipped" : "Checking Claude/Codex routes...",
      status: accessMode === "byok" ? "skipped" : "pending",
      error: null,
      fixLabel: "Manage Routes",
    },
    {
      id: "cliConfig",
      label: "Claude Code & Codex",
      description: isElectron ? "Checking CLI tools..." : "Desktop only — skipped",
      status: isElectron ? "pending" : "skipped",
      error: null,
      fixLabel: "Install",
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
): Pick<CheckItem, "status" | "description" | "error" | "fixLabel"> {
  const claudeReady = summary.claude.activeKeys > 0 || summary.claude.groups > 0;
  const codexReady = summary.codex.activeKeys > 0 || summary.codex.groups > 0;

  if (claudeReady && codexReady) {
    return {
      status: "passed",
      description: "Claude Code and Codex routes can be created from your current cloud account",
      error: null,
      fixLabel: "Manage Routes",
    };
  }

  if (claudeReady) {
    return {
      status: "passed",
      description:
        "Claude Code routing is available. Add an OpenAI group or key if you also want Codex.",
      error: null,
      fixLabel: "Manage Routes",
    };
  }

  if (codexReady) {
    return {
      status: "passed",
      description:
        "Codex routing is available. Add an Anthropic group or key if you also want Claude Code.",
      error: null,
      fixLabel: "Manage Routes",
    };
  }

  if (totalKeys > 0) {
    return {
      status: "failed",
      description: "Your current API keys are not bound to Claude Code or Codex compatible routes",
      error: `Assign an anthropic or openai group in ${CLOUD_NAME} before continuing.`,
      fixLabel: "Manage Routes",
    };
  }

  if (totalGroups > 0) {
    return {
      status: "failed",
      description: "No Claude Code or Codex compatible groups are available for this account",
      error: "Ask for an anthropic/openai group or switch to BYOK.",
      fixLabel: "Manage Routes",
    };
  }

  return {
    status: "failed",
    description: "No managed Claude Code or Codex routes are available yet",
    error: "Create a compatible cloud route or use BYOK instead.",
    fixLabel: "Manage Routes",
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
): string {
  const missing = status ? getMissingCliDependencyNames(status) : [];
  const cleanedMessage = stripDesktopIpcPrefix(getErrorMessage(error));
  const baseMessage = cleanedMessage.startsWith("Install failed")
    ? cleanedMessage
    : "Install failed. Please retry or install manually.";

  if (missing.length === 0) {
    return baseMessage;
  }

  if (/\bMissing:/i.test(baseMessage)) {
    return baseMessage;
  }

  return `${baseMessage.replace(/\.$/, "")}. Missing: ${missing.join(", ")}`;
}

interface CliInstallStep {
  id: "git" | "node" | "codex" | "claude";
  label: string;
  installingDescription: string;
  run: () => Promise<{ status: ModelCliRuntimeStatus }>;
}

export function useSetupChecks(): UseSetupChecksReturn {
  const router = useRouter();
  const { settings } = useAppSettings();
  const { auth, getAccessToken } = useSub2APIAuth();
  const isElectron = getIsElectron();

  const clientRef = useRef(
    settings.accessMode === "builtin" && auth?.endpoint
      ? createSub2APIClient({ endpoint: auth.endpoint, getAccessToken })
      : null,
  );

  const [checks, setChecks] = useState<CheckItem[]>(() => makeInitialChecks(settings.accessMode));
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
      await checkHealth(getEndpoint());
      updateCheck("health", { status: "passed", description: "Service is reachable" });
    } catch (err) {
      updateCheck("health", {
        status: "failed",
        error: err instanceof Error ? err.message : "Connection failed",
        description: "Cannot reach backend service",
        fixLabel: "Retry",
      });
      throw err;
    }
  }, [getEndpoint, updateCheck]);

  const runApiKeyCheck = useCallback(async () => {
    if (settings.accessMode === "byok") {
      updateCheck("apiKeys", { status: "skipped", description: "BYOK mode — skipped" });
      return;
    }
    updateCheck("apiKeys", { status: "checking", error: null });
    try {
      const c = clientRef.current;
      if (!c) throw new Error("Not authenticated");
      const [keyResult, groups] = await Promise.all([c.listKeys(1, 200), c.getAvailableGroups()]);
      const availability = summarizeManagedCloudAvailability(keyResult.items, groups);
      const next = describeManagedCloudAvailability(availability, keyResult.total, groups.length);
      updateCheck("apiKeys", next);
      if (next.status === "failed") {
        throw new Error(MANAGED_CLOUD_ROUTES_UNAVAILABLE);
      }
    } catch (err) {
      if ((err as Error).message !== MANAGED_CLOUD_ROUTES_UNAVAILABLE) {
        updateCheck("apiKeys", {
          status: "failed",
          error: err instanceof Error ? err.message : "Check failed",
          description: "Failed to check Claude/Codex routes",
          fixLabel: "Manage Routes",
        });
      }
      throw err;
    }
  }, [settings.accessMode, updateCheck]);

  const runCliConfigCheck = useCallback(async () => {
    if (!isElectron) {
      updateCheck("cliConfig", { status: "skipped", description: "Desktop only — skipped" });
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
          error: `Missing: ${missing.join(", ")}`,
          description: "CLI tools not fully installed",
          fixLabel: "Install All",
        });
        throw new Error("CLI not installed");
      }

      // Check provider configuration
      const store = await invokeDesktopCommand<ProviderStore>("get_providers");
      const { claude, codex } = resolveScopedActiveProviderIds(store);
      if (!claude && !codex) {
        updateCheck("cliConfig", {
          status: "failed",
          error: "No providers configured for Claude Code or Codex",
          description: "CLI tools installed but not configured",
          fixLabel: "Configure",
        });
        throw new Error("Providers not configured");
      }
      const partial = !claude || !codex;
      if (partial) {
        const configured = claude ? "Claude Code" : "Codex";
        const unconfigured = !claude ? "Claude Code" : "Codex";
        updateCheck("cliConfig", {
          status: "failed",
          description: `Only ${configured} configured`,
          error: `${unconfigured} provider not configured`,
          fixLabel: "Configure",
        });
        throw new Error("Partial config");
      }
      updateCheck("cliConfig", {
        status: "passed",
        description: "Claude Code & Codex installed and configured",
      });
    } catch (err) {
      const msg = (err as Error).message;
      if (
        msg !== "CLI not installed" &&
        msg !== "Providers not configured" &&
        msg !== "Partial config"
      ) {
        updateCheck("cliConfig", {
          status: "failed",
          error: err instanceof Error ? err.message : "Check failed",
          description: "Failed to check CLI configuration",
          fixLabel: "Retry",
        });
      }
      throw err;
    }
  }, [isElectron, updateCheck]);

  const runAllChecks = useCallback(async () => {
    setIsRunning(true);
    setChecks(makeInitialChecks(settings.accessMode));

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
  ]);

  const fixCheck = useCallback(
    async (id: CheckItem["id"]) => {
      switch (id) {
        case "health":
          updateCheck("health", { status: "checking", error: null });
          try {
            await checkHealth(getEndpoint());
            updateCheck("health", { status: "passed", description: "Service is reachable" });
          } catch (err) {
            updateCheck("health", {
              status: "failed",
              error: err instanceof Error ? err.message : "Connection failed",
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
              description: "Preparing CLI installation...",
            });
            try {
              const installSteps: CliInstallStep[] = [
                {
                  id: "git",
                  label: "Git Bash",
                  installingDescription: "Installing Git Bash...",
                  run: installGitBashRuntime,
                },
                {
                  id: "node",
                  label: "Node.js 22",
                  installingDescription: "Installing Node.js 22...",
                  run: installNode22Runtime,
                },
                {
                  id: "codex",
                  label: "Codex",
                  installingDescription: "Installing Codex CLI...",
                  run: installCodexCli,
                },
                {
                  id: "claude",
                  label: "Claude Code",
                  installingDescription: "Installing Claude Code CLI...",
                  run: installClaudeCodeCli,
                },
              ];
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
                  description: `${step.label} ready. Continuing...`,
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
                  error: `Installation incomplete: ${getMissingCliDependencyNames(result.status).join(", ")}`,
                  description: "Some tools failed to install",
                  fixLabel: "Retry",
                });
              }
            } catch (err) {
              updateCheck("cliConfig", {
                status: "failed",
                error: formatCliInstallFailureMessage(err, cliStatusRef.current),
                fixLabel: "Retry",
              });
            }
          } else {
            router.push("/settings/managed-provider");
          }
          break;
        }
      }
    },
    [getEndpoint, router, runCliConfigCheck, updateCheck],
  );

  const allPassed = checks.every((c) => c.status === "passed" || c.status === "skipped");

  return { checks, allPassed, isRunning, runAllChecks, fixCheck };
}

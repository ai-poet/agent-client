import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import type { Logger } from "pino";
import { z } from "zod";

import type { AgentModelDefinition } from "./agent-sdk-types.js";
import { normalizeClaudeRuntimeModelId } from "./providers/claude/claude-models.js";

export type ManagedModelProvider = "claude" | "codex";

export interface ManagedProviderModelCatalogLike {
  getModels(
    provider: ManagedModelProvider,
    fallbackModels: AgentModelDefinition[],
  ): Promise<AgentModelDefinition[]>;
}

const MANAGED_PROVIDER_IDS: Record<ManagedModelProvider, string> = {
  claude: "paseo-managed-claude",
  codex: "paseo-managed-codex",
};

const storedProviderSchema = z
  .object({
    id: z.string(),
    endpoint: z.string(),
    apiKey: z.string(),
    target: z.enum(["claude", "codex"]).optional(),
  })
  .passthrough();

const providerStoreSchema = z
  .object({
    providers: z.array(storedProviderSchema),
    activeClaudeProviderId: z.string().nullable().optional(),
    activeCodexProviderId: z.string().nullable().optional(),
  })
  .passthrough();

const remoteCatalogSchema = z
  .object({
    object: z.string().optional(),
    data: z.array(
      z
        .object({
          id: z.string(),
          display_name: z.string().optional(),
        })
        .passthrough(),
    ),
  })
  .passthrough();

type RemoteModel = {
  id: string;
  displayName?: string;
};

type ManagedRoute = {
  endpoint: string;
  apiKey: string;
};

type RemoteResult =
  | { ok: true; models: RemoteModel[] }
  | { ok: false; reason: "timeout" | "network" | "http" | "malformed" | "empty"; status?: number };

export type ManagedProviderModelCatalogOptions = {
  providersFile?: string;
  fetchImpl?: typeof globalThis.fetch;
  timeoutMs?: number;
};

const DEFAULT_TIMEOUT_MS = 5_000;

function cloneModels(models: AgentModelDefinition[]): AgentModelDefinition[] {
  return models.map((model) => ({
    ...model,
    metadata: model.metadata ? { ...model.metadata } : undefined,
    thinkingOptions: model.thinkingOptions?.map((option) => ({
      ...option,
      metadata: option.metadata ? { ...option.metadata } : undefined,
    })),
  }));
}

export function normalizeManagedProviderEndpoint(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) {
    return null;
  }

  try {
    const url = new URL(trimmed);
    if ((url.protocol !== "http:" && url.protocol !== "https:") || url.username || url.password) {
      return null;
    }

    url.search = "";
    url.hash = "";
    let pathname = url.pathname.replace(/\/+$/, "");
    if (pathname.toLowerCase().endsWith("/v1")) {
      pathname = pathname.slice(0, -3).replace(/\/+$/, "");
    }
    url.pathname = pathname || "/";
    return url.toString().replace(/\/+$/, "");
  } catch {
    return null;
  }
}

function apiKeyFingerprint(apiKey: string): string {
  return createHash("sha256").update(apiKey).digest("hex");
}

function mergeRemoteModels(
  provider: ManagedModelProvider,
  remoteModels: RemoteModel[],
  fallbackModels: AgentModelDefinition[],
): AgentModelDefinition[] {
  const fallbackById = new Map(fallbackModels.map((model) => [model.id, model]));
  const merged = remoteModels.map((remote) => {
    const canonicalId =
      provider === "claude" ? normalizeClaudeRuntimeModelId(remote.id) : remote.id;
    const known =
      fallbackById.get(remote.id) ?? (canonicalId ? fallbackById.get(canonicalId) : null);
    if (known) {
      return cloneModels([{ ...known, provider, id: remote.id }])[0]!;
    }

    return {
      provider,
      id: remote.id,
      label: remote.displayName || remote.id,
    } satisfies AgentModelDefinition;
  });

  if (!merged.some((model) => model.isDefault) && merged[0]) {
    merged[0] = { ...merged[0], isDefault: true };
  }
  return merged;
}

export class ManagedProviderModelCatalog implements ManagedProviderModelCatalogLike {
  private readonly providersFile: string;
  private readonly fetchImpl: typeof globalThis.fetch;
  private readonly timeoutMs: number;
  private readonly cache = new Map<string, AgentModelDefinition[]>();

  constructor(
    private readonly logger: Logger,
    options: ManagedProviderModelCatalogOptions = {},
  ) {
    this.providersFile =
      options.providersFile ?? join(homedir(), ".agent-client", "providers.json");
    this.fetchImpl = options.fetchImpl ?? globalThis.fetch;
    this.timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  }

  async getModels(
    provider: ManagedModelProvider,
    fallbackModels: AgentModelDefinition[],
  ): Promise<AgentModelDefinition[]> {
    const route = await this.readManagedRoute(provider);
    if (!route) {
      return cloneModels(fallbackModels);
    }

    const cacheKey = `${provider}:${route.endpoint}:${apiKeyFingerprint(route.apiKey)}`;
    const result = await this.fetchRemoteModels(route);
    if (result.ok) {
      const models = mergeRemoteModels(provider, result.models, fallbackModels);
      this.cache.set(cacheKey, cloneModels(models));
      return models;
    }

    this.logger.debug(
      {
        provider,
        endpoint: route.endpoint,
        reason: result.reason,
        ...(result.status === undefined ? {} : { status: result.status }),
      },
      "Managed model catalog request failed; using fallback",
    );
    const cached = this.cache.get(cacheKey);
    return cached ? cloneModels(cached) : cloneModels(fallbackModels);
  }

  private async readManagedRoute(provider: ManagedModelProvider): Promise<ManagedRoute | null> {
    let raw: string;
    try {
      raw = await readFile(this.providersFile, "utf8");
    } catch {
      return null;
    }

    let parsedJson: unknown;
    try {
      parsedJson = JSON.parse(raw);
    } catch {
      return null;
    }

    const parsed = providerStoreSchema.safeParse(parsedJson);
    if (!parsed.success) {
      return null;
    }

    const activeProviderId =
      provider === "claude"
        ? parsed.data.activeClaudeProviderId
        : parsed.data.activeCodexProviderId;
    const managedProviderId = MANAGED_PROVIDER_IDS[provider];
    if (activeProviderId !== managedProviderId) {
      return null;
    }

    const storedProvider = parsed.data.providers.find(
      (candidate) => candidate.id === managedProviderId,
    );
    if (!storedProvider || (storedProvider.target && storedProvider.target !== provider)) {
      return null;
    }

    const endpoint = normalizeManagedProviderEndpoint(storedProvider.endpoint);
    const apiKey = storedProvider.apiKey.trim();
    if (!endpoint || !apiKey) {
      return null;
    }
    return { endpoint, apiKey };
  }

  private async fetchRemoteModels(route: ManagedRoute): Promise<RemoteResult> {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), this.timeoutMs);
    try {
      let response: Response;
      try {
        response = await this.fetchImpl(`${route.endpoint}/v1/models`, {
          headers: {
            Authorization: `Bearer ${route.apiKey}`,
          },
          signal: controller.signal,
        });
      } catch {
        return { ok: false, reason: controller.signal.aborted ? "timeout" : "network" };
      }

      if (!response.ok) {
        return { ok: false, reason: "http", status: response.status };
      }

      let body: unknown;
      try {
        body = await response.json();
      } catch {
        return {
          ok: false,
          reason: controller.signal.aborted ? "timeout" : "malformed",
        };
      }
      const parsed = remoteCatalogSchema.safeParse(body);
      if (!parsed.success) {
        return { ok: false, reason: "malformed" };
      }

      const seen = new Set<string>();
      const models: RemoteModel[] = [];
      for (const item of parsed.data.data) {
        const id = item.id.trim();
        if (!id || seen.has(id)) {
          continue;
        }
        seen.add(id);
        const displayName = item.display_name?.trim();
        models.push({ id, ...(displayName ? { displayName } : {}) });
      }
      return models.length > 0 ? { ok: true, models } : { ok: false, reason: "empty" };
    } finally {
      clearTimeout(timeout);
    }
  }
}

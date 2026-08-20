import type { ManagedProviderTarget } from "@/screens/settings/sub2api-provider-types";

/**
 * Targets whose config embeds an explicit model list, so switching to them needs the
 * gateway catalog first. Claude and Codex discover models themselves at runtime.
 */
const TARGETS_NEEDING_MODEL_LIST = new Set<ManagedProviderTarget>(["grok", "pi"]);

/**
 * Model families a target can actually run. Grok only speaks to the xAI family even though
 * the gateway lists everything; Pi is model-agnostic and takes the whole catalog.
 */
const TARGET_MODEL_PREFIXES: Partial<Record<ManagedProviderTarget, string>> = {
  grok: "grok-",
};

/**
 * Written when the catalog cannot be read — a BYOK endpoint need not serve `/v1/models`, and
 * refusing to write the config would leave no file for the user to edit by hand.
 */
const FALLBACK_MODELS: Partial<Record<ManagedProviderTarget, string[]>> = {
  grok: ["grok-4.6"],
};

export function targetNeedsModelList(target: ManagedProviderTarget): boolean {
  return TARGETS_NEEDING_MODEL_LIST.has(target);
}

export function defaultModelsForTarget(target: ManagedProviderTarget): string[] {
  return FALLBACK_MODELS[target] ?? [];
}

export function filterGatewayModelsForTarget(
  target: ManagedProviderTarget,
  modelIds: string[],
): string[] {
  const prefix = TARGET_MODEL_PREFIXES[target];
  if (!prefix) {
    return modelIds;
  }
  return modelIds.filter((id) => id.toLowerCase().startsWith(prefix));
}

/** Saved rows store the bare origin; the OpenAI-compatible catalog lives under /v1. */
export function gatewayModelsUrl(endpoint: string): string {
  const trimmed = endpoint.trim().replace(/\/+$/, "");
  const base = trimmed.toLowerCase().endsWith("/v1") ? trimmed.slice(0, -3) : trimmed;
  return `${base.replace(/\/+$/, "")}/v1/models`;
}

export function parseGatewayModelIds(body: unknown): string[] {
  if (typeof body !== "object" || body === null || !("data" in body)) {
    return [];
  }
  const data = (body as { data: unknown }).data;
  if (!Array.isArray(data)) {
    return [];
  }

  const seen = new Set<string>();
  for (const entry of data) {
    if (typeof entry !== "object" || entry === null || !("id" in entry)) {
      continue;
    }
    const id = (entry as { id: unknown }).id;
    if (typeof id !== "string") {
      continue;
    }
    const trimmed = id.trim();
    if (trimmed) {
      seen.add(trimmed);
    }
  }
  return [...seen];
}

/**
 * Reads the gateway's model catalog. Throws on failure rather than returning an empty list —
 * writing a Grok or Pi config with no models would leave a valid-looking but unusable setup.
 */
export async function fetchGatewayModelIds(options: {
  endpoint: string;
  apiKey: string;
  fetchImpl?: typeof globalThis.fetch;
}): Promise<string[]> {
  const doFetch = options.fetchImpl ?? globalThis.fetch;
  const response = await doFetch(gatewayModelsUrl(options.endpoint), {
    headers: { Authorization: `Bearer ${options.apiKey}` },
  });
  if (!response.ok) {
    throw new Error(`Model catalog request failed with HTTP ${response.status}`);
  }
  return parseGatewayModelIds(await response.json());
}

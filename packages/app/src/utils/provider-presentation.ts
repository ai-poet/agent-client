import type { ProviderSnapshotEntry, ProviderStatus } from "@server/server/agent/agent-sdk-types";

export type ProviderStatusTone = "success" | "error" | "warning" | "pending" | "muted";

/**
 * `loading` and `unavailable` must not share a tone — "checking" and "not installed"
 * are different situations and the user acts on them differently.
 */
export function resolveProviderStatusTone(status: ProviderStatus): ProviderStatusTone {
  switch (status) {
    case "ready":
      return "success";
    case "error":
      return "error";
    case "loading":
      return "pending";
    case "unavailable":
      return "warning";
  }
}

/**
 * The secondary line on a provider row: version and, when it adds information, the
 * model count. Returns null when there is nothing worth a second line.
 */
export function formatProviderMeta(
  entry: ProviderSnapshotEntry | undefined,
  modelCountLabel: (count: number) => string,
): string | null {
  if (!entry) {
    return null;
  }
  const parts: string[] = [];
  if (entry.version) {
    parts.push(entry.version);
  }
  const modelCount = entry.models?.length ?? 0;
  if (entry.status === "ready" && modelCount > 0) {
    parts.push(modelCountLabel(modelCount));
  }
  return parts.length > 0 ? parts.join(" · ") : null;
}

/**
 * A provider is installable from the app only when the daemon reported an install command
 * for it. Everything else (including a provider that ships inside the daemon) shows facts
 * without an action.
 */
export function canOfferInstall(entry: ProviderSnapshotEntry | undefined): boolean {
  return Boolean(entry?.installCommand) && entry?.status === "unavailable";
}

export type ProviderInstallOutcome =
  | { kind: "installed"; version?: string }
  /** The command finished but the binary still is not detectable. */
  | { kind: "not-detected" }
  /** An update ran but the reported version did not move. */
  | { kind: "unchanged"; version: string };

/**
 * Classifies what actually happened after an install so the UI can report it honestly
 * instead of claiming success whenever the command exited zero.
 */
export function classifyInstallOutcome(options: {
  previousVersion?: string;
  nextEntry: ProviderSnapshotEntry | undefined;
}): ProviderInstallOutcome {
  const { previousVersion, nextEntry } = options;
  const installed = nextEntry?.status === "ready";
  if (!installed) {
    return { kind: "not-detected" };
  }
  const version = nextEntry?.version;
  if (previousVersion && version && previousVersion === version) {
    return { kind: "unchanged", version };
  }
  return { kind: "installed", version };
}

import type { SegmentedControlOption } from "@/components/ui/segmented-control";
import type { getSub2APIMessages } from "@/i18n/sub2api";
import type {
  DesktopProviderPayload,
  ManagedProviderTarget,
} from "@/screens/settings/sub2api-provider-types";

type DesktopProviderText = ReturnType<typeof getSub2APIMessages>["settings"]["desktopProviders"];

/**
 * CLIs the managed UI offers, in display order. Claude and Codex come first because an
 * untargeted legacy row still implicitly means those two.
 *
 * Pi is deliberately absent: its write path is implemented and tested, but the provider is
 * hidden until its gateway routing is verified. Add it back here and drop `hidden` from the
 * server manifest to re-enable it.
 */
export const MANAGED_PROVIDER_TARGETS = ["claude", "codex", "grok"] as const;

export function providerWritesTarget(
  p: { target?: ManagedProviderTarget },
  target: ManagedProviderTarget,
): boolean {
  return p.target === target;
}

export function providerTargetHint(p: DesktopProviderPayload, text?: DesktopProviderText): string {
  const hints = text?.providerTargetHints;
  switch (p.target) {
    case "claude":
      return hints?.claude ?? "Claude Code · Anthropic";
    case "codex":
      return hints?.codex ?? "Codex · Responses";
    case "grok":
      return hints?.grok ?? "Grok · Responses · grok- models only";
    case "pi":
      return hints?.pi ?? "Pi · all gateway models";
    default:
      return hints?.legacy ?? "Legacy unscoped endpoint";
  }
}

/**
 * Labels only — icons are attached by the panel. Keeping this module free of component
 * imports is what lets it stay a plain unit-testable module.
 */
export function getCustomTargetSegmentOptions(text: {
  claude: string;
  codex: string;
  grok: string;
  pi: string;
}): SegmentedControlOption<ManagedProviderTarget>[] {
  return MANAGED_PROVIDER_TARGETS.map((target) => ({
    value: target,
    label: text[target],
  }));
}

export const ENDPOINT_PLACEHOLDER =
  "https://api.example.com — omit /v1 (a trailing /v1 is stripped if present)";

export function getErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function formatUsd(value: number | null | undefined): string {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return "--";
  }
  return `$${value.toFixed(2)}`;
}

export function maskApiKey(value: string): string {
  const trimmed = value.trim();
  if (trimmed.length <= 12) {
    return trimmed;
  }
  return `${trimmed.slice(0, 8)}...${trimmed.slice(-4)}`;
}

export function findReusableKey<T extends { group_id: number | null; status?: string }>(
  keys: T[],
  groupId: number,
): T | null {
  return (
    keys.find((entry) => entry.group_id === groupId && entry.status === "active") ??
    keys.find((entry) => entry.group_id === groupId) ??
    null
  );
}

export function normalizeFilter(s: string): string {
  return s.trim().toLowerCase();
}

import type { SegmentedControlOption } from "@/components/ui/segmented-control";
import type { getSub2APIMessages } from "@/i18n/sub2api";
import type {
  DesktopProviderPayload,
  ManagedProviderTarget,
} from "@/screens/settings/sub2api-provider-types";

type DesktopProviderText = ReturnType<typeof getSub2APIMessages>["settings"]["desktopProviders"];

export function providerWritesClaude(p: { target?: ManagedProviderTarget }): boolean {
  return p.target === "claude";
}

export function providerWritesCodex(p: { target?: ManagedProviderTarget }): boolean {
  return p.target === "codex";
}

export function providerTargetHint(p: DesktopProviderPayload, text?: DesktopProviderText): string {
  if (p.target === "claude") {
    return text?.providerTargetHints.claude ?? "Claude Code · Anthropic";
  }
  if (p.target === "codex") {
    return text?.providerTargetHints.codex ?? "Codex · Responses";
  }
  return text?.providerTargetHints.legacy ?? "Legacy unscoped endpoint";
}

export function getCustomTargetSegmentOptions(text: {
  claude: string;
  codex: string;
}): SegmentedControlOption<ManagedProviderTarget>[] {
  return [
    { value: "claude", label: text.claude },
    { value: "codex", label: text.codex },
  ];
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

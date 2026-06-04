import type { AgentMode } from "@server/server/agent/agent-sdk-types";
import { getAppMessages, normalizeSub2APILocale, type Sub2APILocale } from "@/i18n/sub2api";

type AgentModeText = ReturnType<typeof getAppMessages>["agentModes"];
type AgentModeTextKey = keyof AgentModeText;

const MODE_TEXT_BY_ID: Record<string, AgentModeTextKey> = {
  default: "alwaysAsk",
  "always-ask": "alwaysAsk",
  acceptEdits: "acceptEdits",
  plan: "plan",
  bypassPermissions: "bypassPermissions",
  auto: "defaultPermissions",
  "full-access": "fullAccess",
  "read-only": "readOnly",
  build: "build",
  "https://agentclientprotocol.com/protocol/session-modes#agent": "agent",
  "https://agentclientprotocol.com/protocol/session-modes#plan": "plan",
  "https://agentclientprotocol.com/protocol/session-modes#autopilot": "autopilot",
  "load-test": "loadTest",
};

const MODE_TEXT_BY_LABEL: Record<string, AgentModeTextKey> = {
  "always ask": "alwaysAsk",
  default: "alwaysAsk",
  "accept file edits": "acceptEdits",
  "plan mode": "plan",
  plan: "plan",
  bypass: "bypassPermissions",
  "bypass permissions": "bypassPermissions",
  "default permissions": "defaultPermissions",
  auto: "defaultPermissions",
  "full access": "fullAccess",
  "read only": "readOnly",
  "read-only": "readOnly",
  build: "build",
  agent: "agent",
  autopilot: "autopilot",
  "load test": "loadTest",
};

function normalizeModeLabel(value: string | null | undefined): string {
  return value?.trim().toLowerCase().replace(/\s+/g, " ") ?? "";
}

function resolveModeTextKey(mode: Pick<AgentMode, "id" | "label">): AgentModeTextKey | null {
  return MODE_TEXT_BY_ID[mode.id] ?? MODE_TEXT_BY_LABEL[normalizeModeLabel(mode.label)] ?? null;
}

export function localizeAgentModeLabel(
  mode: Pick<AgentMode, "id" | "label">,
  locale: Sub2APILocale | string | null | undefined,
): string {
  const textKey = resolveModeTextKey(mode);
  if (!textKey) {
    return mode.label;
  }
  return getAppMessages(normalizeSub2APILocale(locale)).agentModes[textKey].label;
}

export function localizeAgentModeDescription(
  mode: Pick<AgentMode, "id" | "label" | "description">,
  locale: Sub2APILocale | string | null | undefined,
): string | undefined {
  const textKey = resolveModeTextKey(mode);
  if (!textKey) {
    return mode.description;
  }
  return getAppMessages(normalizeSub2APILocale(locale)).agentModes[textKey].description;
}

export function localizeAgentMode(
  mode: AgentMode,
  locale: Sub2APILocale | string | null | undefined,
): AgentMode {
  return {
    ...mode,
    label: localizeAgentModeLabel(mode, locale),
    ...(mode.description || resolveModeTextKey(mode)
      ? { description: localizeAgentModeDescription(mode, locale) }
      : {}),
  };
}

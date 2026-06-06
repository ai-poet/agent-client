import {
  buildToolCallDisplayModel as buildSharedToolCallDisplayModel,
  type ToolCallDisplayInput,
  type ToolCallDisplayModel,
} from "@server/shared/tool-call-display";
import { getAppMessages } from "@/i18n/sub2api";

export type { ToolCallDisplayInput, ToolCallDisplayModel };

type AgentToolLabels = ReturnType<typeof getAppMessages>["agentTools"]["labels"];

export type LocalizedToolCallDisplayOptions = {
  locale?: string | null;
};

function readString(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function hasLocaleOption(
  options: LocalizedToolCallDisplayOptions | undefined,
): options is LocalizedToolCallDisplayOptions {
  return Boolean(options && Object.prototype.hasOwnProperty.call(options, "locale"));
}

function getCanonicalDisplayName(
  input: ToolCallDisplayInput,
  labels: AgentToolLabels,
): string | undefined {
  switch (input.detail.type) {
    case "shell":
      return labels.shell;
    case "read":
      return labels.read;
    case "edit":
      return labels.edit;
    case "write":
      return labels.write;
    case "search":
      return labels.search;
    case "fetch":
      return labels.fetch;
    case "worktree_setup":
      return labels.worktreeSetup;
    case "sub_agent":
      return readString(input.detail.subAgentType) ?? labels.task;
    case "plan":
      return labels.plan;
    case "plain_text":
    case "unknown":
      return undefined;
  }
}

function getUnknownDetailOverrideDisplayName(
  input: ToolCallDisplayInput,
  labels: AgentToolLabels,
): string | undefined {
  const lowerName = input.name.trim().toLowerCase();
  if (input.detail.type === "unknown" && lowerName === "task") {
    return labels.task;
  }
  if (input.detail.type === "unknown" && lowerName === "thinking") {
    return labels.thinking;
  }
  if (lowerName === "terminal") {
    return labels.terminal;
  }
  return undefined;
}

export function buildToolCallDisplayModel(
  input: ToolCallDisplayInput,
  options?: LocalizedToolCallDisplayOptions,
): ToolCallDisplayModel {
  const display = buildSharedToolCallDisplayModel(input);
  if (!hasLocaleOption(options)) {
    return display;
  }

  const labels = getAppMessages(options.locale).agentTools.labels;
  const displayName =
    getUnknownDetailOverrideDisplayName(input, labels) ??
    getCanonicalDisplayName(input, labels) ??
    display.displayName;

  return {
    ...display,
    displayName,
  };
}

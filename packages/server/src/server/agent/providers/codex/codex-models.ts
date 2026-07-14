import type { AgentModelDefinition, AgentSelectOption } from "../../agent-sdk-types.js";

const STANDARD_REASONING_OPTIONS = [
  { id: "low", label: "Low", description: "Fast responses with lighter reasoning" },
  {
    id: "medium",
    label: "Medium",
    description: "Balances speed and reasoning depth for everyday tasks",
  },
  { id: "high", label: "High", description: "Greater reasoning depth for complex problems" },
  {
    id: "xhigh",
    label: "Extra High",
    description: "Extra high reasoning depth for complex problems",
  },
  { id: "max", label: "Max", description: "Maximum reasoning depth for the hardest problems" },
] satisfies AgentSelectOption[];

const ULTRA_REASONING_OPTION = {
  id: "ultra",
  label: "Ultra",
  description: "Maximum reasoning with automatic task delegation",
} satisfies AgentSelectOption;

function reasoningOptions(defaultId: string, includeUltra: boolean): AgentSelectOption[] {
  const options = includeUltra
    ? [...STANDARD_REASONING_OPTIONS, ULTRA_REASONING_OPTION]
    : STANDARD_REASONING_OPTIONS;
  return options.map((option) => ({ ...option, isDefault: option.id === defaultId }));
}

const CODEX_MODELS: AgentModelDefinition[] = [
  {
    provider: "codex",
    id: "gpt-5.6-sol",
    label: "GPT-5.6-Sol",
    description: "Latest frontier agentic coding model.",
    thinkingOptions: reasoningOptions("low", true),
    defaultThinkingOptionId: "low",
  },
  {
    provider: "codex",
    id: "gpt-5.6-terra",
    label: "GPT-5.6-Terra",
    description: "Balanced agentic coding model for everyday work.",
    thinkingOptions: reasoningOptions("medium", true),
    defaultThinkingOptionId: "medium",
  },
  {
    provider: "codex",
    id: "gpt-5.6-luna",
    label: "GPT-5.6-Luna",
    description: "Fast and affordable agentic coding model.",
    thinkingOptions: reasoningOptions("medium", false),
    defaultThinkingOptionId: "medium",
  },
  {
    provider: "codex",
    id: "gpt-5.5",
    label: "GPT-5.5",
    description: "Frontier model for complex coding, research, and real-world work.",
  },
  {
    provider: "codex",
    id: "gpt-5.4",
    label: "GPT-5.4",
    description: "Strong model for everyday coding.",
    isDefault: true,
  },
  {
    provider: "codex",
    id: "gpt-5.4-mini",
    label: "GPT-5.4 Mini",
    description: "Small, fast, and cost-efficient model for simpler coding tasks.",
  },
];

export function getCodexModels(): AgentModelDefinition[] {
  return CODEX_MODELS.map((model) => ({
    ...model,
    thinkingOptions: model.thinkingOptions?.map((option) => ({ ...option })),
  }));
}

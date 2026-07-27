export type ModelContextWindow = "256k" | "1m";

export type ModelContextEntry = {
  id: string;
  label: string;
  description?: string;
  isDefault?: boolean;
};

const CLAUDE_PROVIDER = "claude";
const ONE_MILLION_SUFFIX = "[1m]";

export function supportsModelContextWindow(provider: string): boolean {
  return provider === CLAUDE_PROVIDER;
}

export function getBaseModelId(provider: string, modelId: string): string {
  if (!supportsModelContextWindow(provider)) {
    return modelId;
  }
  return modelId.endsWith(ONE_MILLION_SUFFIX)
    ? modelId.slice(0, -ONE_MILLION_SUFFIX.length)
    : modelId;
}

export function getModelContextWindow(provider: string, modelId: string): ModelContextWindow {
  return supportsModelContextWindow(provider) && modelId.endsWith(ONE_MILLION_SUFFIX)
    ? "1m"
    : "256k";
}

function removeContextSuffix(label: string): string {
  return label.replace(/\s+1m$/i, "");
}

export function collapseModelContextVariants<T extends ModelContextEntry>(
  provider: string,
  models: T[],
): T[] {
  if (!supportsModelContextWindow(provider)) {
    return models;
  }

  const collapsed: T[] = [];
  const indexesByBaseId = new Map<string, number>();

  for (const model of models) {
    const baseModelId = getBaseModelId(provider, model.id);
    const existingIndex = indexesByBaseId.get(baseModelId);
    const normalizedModel = {
      ...model,
      id: baseModelId,
      label: removeContextSuffix(model.label),
    } as T;

    if (existingIndex === undefined) {
      indexesByBaseId.set(baseModelId, collapsed.length);
      collapsed.push(normalizedModel);
      continue;
    }

    const existing = collapsed[existingIndex]!;
    const currentIsStandard = getModelContextWindow(provider, model.id) === "256k";
    if (currentIsStandard) {
      collapsed[existingIndex] = {
        ...normalizedModel,
        ...(existing.isDefault ? { isDefault: true } : {}),
      };
    }
  }

  return collapsed;
}

export function getAvailableModelContextWindows<T extends Pick<ModelContextEntry, "id">>(input: {
  provider: string;
  modelId: string;
  models: T[];
}): ModelContextWindow[] {
  if (!supportsModelContextWindow(input.provider) || !input.modelId) {
    return [];
  }

  const baseModelId = getBaseModelId(input.provider, input.modelId);
  const available = new Set<ModelContextWindow>();
  for (const model of input.models) {
    if (getBaseModelId(input.provider, model.id) === baseModelId) {
      available.add(getModelContextWindow(input.provider, model.id));
    }
  }

  if (available.size === 0) {
    available.add(getModelContextWindow(input.provider, input.modelId));
  }

  return (["256k", "1m"] as const).filter((context) => available.has(context));
}

export function resolveModelIdForContext<T extends Pick<ModelContextEntry, "id">>(input: {
  provider: string;
  modelId: string;
  contextWindow: ModelContextWindow;
  models: T[];
}): string {
  if (!supportsModelContextWindow(input.provider)) {
    return input.modelId;
  }

  const baseModelId = getBaseModelId(input.provider, input.modelId);
  const requestedId =
    input.contextWindow === "1m" ? `${baseModelId}${ONE_MILLION_SUFFIX}` : baseModelId;
  if (input.models.some((model) => model.id === requestedId)) {
    return requestedId;
  }

  const fallback = input.models.find(
    (model) => getBaseModelId(input.provider, model.id) === baseModelId,
  );
  return fallback?.id ?? requestedId;
}

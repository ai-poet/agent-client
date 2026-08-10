import { memo, useCallback, useMemo, useRef, useState } from "react";
import { View, Text, Pressable, Keyboard } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { useShallow } from "zustand/shallow";
import { useStoreWithEqualityFn } from "zustand/traditional";
import {
  Brain,
  ChevronDown,
  Cloud,
  ListTodo,
  Maximize2,
  Settings2,
  ShieldAlert,
  ShieldCheck,
  ShieldOff,
  Zap,
} from "lucide-react-native";
import { getProviderIcon } from "@/components/provider-icons";
import { CombinedModelSelector } from "@/components/combined-model-selector";
import type { SelectorCloudGroup } from "@/components/combined-model-selector.utils";
import { useSessionStore } from "@/stores/session-store";
import { useProvidersSnapshot } from "@/hooks/use-providers-snapshot";
import { useCloudModelRouting } from "@/hooks/use-cloud-model-routing";
import { resolveProviderDefinition } from "@/utils/provider-definitions";
import {
  buildFavoriteModelKey,
  mergeProviderPreferences,
  toggleFavoriteModel,
  useFormPreferences,
} from "@/hooks/use-form-preferences";
import { useAppLocale } from "@/hooks/use-app-locale";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Combobox, ComboboxItem, type ComboboxOption } from "@/components/ui/combobox";
import { AdaptiveModalSheet } from "@/components/adaptive-modal-sheet";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import type {
  AgentFeature,
  AgentMode,
  AgentModelDefinition,
  AgentProvider,
} from "@server/server/agent/agent-sdk-types";
import type { AgentProviderDefinition } from "@server/server/agent/provider-manifest";
import {
  getModeVisuals,
  type AgentModeColorTier,
  type AgentModeIcon,
} from "@server/server/agent/provider-manifest";
import {
  cloudGroupsForStatusProvider,
  filterSelectableProviderDefinitions,
  resolveCloudGroupDisplayLabel,
  getFeatureHighlightColor,
  getFeatureTooltip,
  getStatusSelectorHint,
  resolveAgentModelSelection,
  scopeModelsToProvider,
} from "@/components/agent-status-bar.utils";
import { isWeb as platformIsWeb } from "@/constants/platform";
import { useToast } from "@/contexts/toast-context";
import { toErrorMessage } from "@/utils/error-messages";
import { getAppMessages } from "@/i18n/sub2api";
import { localizeAgentMode, localizeAgentModeLabel } from "@/utils/agent-mode-localization";
import {
  collapseModelContextVariants,
  getAvailableModelContextWindows,
  getBaseModelId,
  getModelContextWindow,
  resolveModelIdForContext,
  supportsModelContextWindow,
} from "@/utils/model-context-window";

type StatusOption = {
  id: string;
  label: string;
};

type StatusSelector = "provider" | "mode" | "model" | "context" | "thinking" | `feature-${string}`;

type ControlledAgentStatusBarProps = {
  provider: string;
  providerOptions?: StatusOption[];
  selectedProviderId?: string;
  onSelectProvider?: (providerId: string) => void;
  modeOptions?: StatusOption[];
  selectedModeId?: string;
  onSelectMode?: (modeId: string) => void;
  modelOptions?: StatusOption[];
  selectedModelId?: string;
  onSelectModel?: (modelId: string) => void;
  onSelectProviderAndModel?: (provider: string, modelId: string) => void;
  thinkingOptions?: StatusOption[];
  selectedThinkingOptionId?: string;
  onSelectThinkingOption?: (thinkingOptionId: string) => void;
  disabled?: boolean;
  isModelLoading?: boolean;
  providerDefinitions: AgentProviderDefinition[];
  allProviderModels?: Map<string, AgentModelDefinition[]>;
  canSelectModelProvider?: (providerId: string) => boolean;
  favoriteKeys?: Set<string>;
  onToggleFavoriteModel?: (provider: string, modelId: string) => void;
  features?: AgentFeature[];
  onSetFeature?: (featureId: string, value: unknown) => void;
  onDropdownClose?: () => void;
  onModelSelectorOpen?: () => void;
  cloudGroups?: SelectorCloudGroup[];
};

export interface DraftAgentStatusBarProps {
  serverId?: string;
  cwd?: string;
  providerDefinitions: AgentProviderDefinition[];
  selectableProviderIds?: AgentProvider[];
  selectedProvider: AgentProvider | null;
  onSelectProvider: (provider: AgentProvider) => void;
  modeOptions: AgentMode[];
  selectedMode: string;
  onSelectMode: (modeId: string) => void;
  models: AgentModelDefinition[];
  selectedModel: string;
  onSelectModel: (modelId: string) => void;
  isModelLoading: boolean;
  allProviderModels: Map<string, AgentModelDefinition[]>;
  isAllModelsLoading: boolean;
  onSelectProviderAndModel: (provider: AgentProvider, modelId: string) => void;
  thinkingOptions: NonNullable<AgentModelDefinition["thinkingOptions"]>;
  selectedThinkingOptionId: string;
  onSelectThinkingOption: (thinkingOptionId: string) => void;
  features?: AgentFeature[];
  onSetFeature?: (featureId: string, value: unknown) => void;
  onDropdownClose?: () => void;
  onModelSelectorOpen?: () => void;
  disabled?: boolean;
}

interface AgentStatusBarProps {
  agentId: string;
  serverId: string;
  onDropdownClose?: () => void;
}

function findOptionLabel(
  options: StatusOption[] | undefined,
  selectedId: string | undefined,
  fallback: string,
) {
  if (!options || options.length === 0) {
    return fallback;
  }
  const selected = options.find((option) => option.id === selectedId);
  return selected?.label ?? fallback;
}

type WorkspaceText = ReturnType<typeof getAppMessages>["workspace"];

function normalizeThinkingEffortKey(
  option: Pick<StatusOption, "id" | "label">,
): "low" | "medium" | "high" | "extraHigh" | null {
  const normalized = `${option.id} ${option.label}`
    .trim()
    .toLowerCase()
    .replace(/[\s_-]+/g, "");
  if (!normalized) {
    return null;
  }
  if (normalized.includes("extrahigh") || normalized.includes("xhigh")) {
    return "extraHigh";
  }
  if (normalized.includes("medium") || normalized === "med") {
    return "medium";
  }
  if (normalized.includes("high")) {
    return "high";
  }
  if (normalized.includes("low")) {
    return "low";
  }
  return null;
}

function localizeThinkingOption(option: StatusOption, text: WorkspaceText): StatusOption {
  const key = normalizeThinkingEffortKey(option);
  return {
    ...option,
    label: key ? text.thinkingEffortLabels[key] : option.label,
  };
}

const FEATURE_ICONS: Record<string, typeof Zap> = {
  "list-todo": ListTodo,
  zap: Zap,
};

function getFeatureIcon(icon?: string) {
  return (icon && FEATURE_ICONS[icon]) || Settings2;
}

function getFeatureIconColor(
  featureId: string,
  enabled: boolean,
  palette: {
    blue: { 400: string };
    yellow: { 400: string };
  },
  foregroundMuted: string,
): string {
  if (!enabled) {
    return foregroundMuted;
  }

  switch (getFeatureHighlightColor(featureId)) {
    case "blue":
      return palette.blue[400];
    case "yellow":
      return palette.yellow[400];
    default:
      return foregroundMuted;
  }
}

const MODE_ICONS = {
  ShieldCheck,
  ShieldAlert,
  ShieldOff,
} as const;

function getModeIconColor(
  colorTier: AgentModeColorTier | undefined,
  palette: {
    blue: { 500: string };
    green: { 500: string };
    red: { 500: string };
    purple: { 500: string };
  },
): string {
  switch (colorTier) {
    case "safe":
      return palette.green[500];
    case "moderate":
      return palette.blue[500];
    case "dangerous":
      return palette.red[500];
    case "planning":
      return palette.purple[500];
    default:
      return palette.blue[500];
  }
}

function ControlledStatusBar({
  provider,
  providerOptions,
  selectedProviderId,
  onSelectProvider,
  modeOptions,
  selectedModeId,
  onSelectMode,
  modelOptions,
  selectedModelId,
  onSelectModel,
  onSelectProviderAndModel,
  thinkingOptions,
  selectedThinkingOptionId,
  onSelectThinkingOption,
  disabled = false,
  isModelLoading = false,
  providerDefinitions,
  allProviderModels,
  canSelectModelProvider,
  favoriteKeys = new Set<string>(),
  onToggleFavoriteModel,
  features,
  onSetFeature,
  onDropdownClose,
  onModelSelectorOpen,
  cloudGroups = [],
}: ControlledAgentStatusBarProps) {
  const { theme } = useUnistyles();
  const locale = useAppLocale();
  const appText = useMemo(() => getAppMessages(locale), [locale]);
  const text = appText.workspace;
  const [prefsOpen, setPrefsOpen] = useState(false);
  const [openSelector, setOpenSelector] = useState<StatusSelector | null>(null);

  const providerAnchorRef = useRef<View>(null);
  const modeAnchorRef = useRef<View>(null);
  const modelAnchorRef = useRef<View>(null);
  const contextAnchorRef = useRef<View>(null);
  const thinkingAnchorRef = useRef<View>(null);

  const canSelectProvider = Boolean(
    onSelectProvider && providerOptions && providerOptions.length > 0,
  );
  const canSelectMode = Boolean(onSelectMode && modeOptions && modeOptions.length > 0);
  const canSelectModel = Boolean(onSelectModel);
  const canSelectThinking = Boolean(
    onSelectThinkingOption && thinkingOptions && thinkingOptions.length > 0,
  );

  const displayProvider = findOptionLabel(
    providerOptions,
    selectedProviderId,
    providerDefinitions.find((definition) => definition.id === selectedProviderId)?.label ??
      text.providerFallback,
  );
  const displayCloudGroup = resolveCloudGroupDisplayLabel(cloudGroups, provider);
  const displayMode = findOptionLabel(modeOptions, selectedModeId, text.defaultMode);
  const selectedDisplayModelId = getBaseModelId(provider, selectedModelId ?? "");
  const displayModelOptions = useMemo(
    () => collapseModelContextVariants(provider, modelOptions ?? []),
    [modelOptions, provider],
  );
  const displayModel =
    isModelLoading && displayModelOptions.length === 0
      ? text.loadingModels
      : findOptionLabel(
          displayModelOptions,
          selectedDisplayModelId,
          appText.modelSelector.selectModel,
        );
  const localizedThinkingOptions = useMemo(
    () => (thinkingOptions ?? []).map((option) => localizeThinkingOption(option, text)),
    [text, thinkingOptions],
  );
  const displayThinking = findOptionLabel(
    localizedThinkingOptions,
    selectedThinkingOptionId,
    localizedThinkingOptions[0]?.label ?? text.unknown,
  );

  const modeVisuals = selectedModeId
    ? getModeVisuals(provider, selectedModeId, providerDefinitions)
    : undefined;
  const ModeIconComponent = modeVisuals?.icon ? MODE_ICONS[modeVisuals.icon] : null;
  const modeIconColor = getModeIconColor(modeVisuals?.colorTier, theme.colors.palette);
  const hasSelectedProvider = provider.trim().length > 0;
  const ProviderIcon = hasSelectedProvider ? getProviderIcon(provider) : null;

  const hasAnyControl =
    Boolean(providerOptions?.length) ||
    Boolean(displayCloudGroup) ||
    Boolean(modeOptions?.length) ||
    canSelectModel ||
    Boolean(thinkingOptions?.length) ||
    Boolean(features?.length);

  if (!hasAnyControl) {
    return null;
  }

  const modelDisabled = disabled || !hasSelectedProvider;

  const SEARCH_THRESHOLD = 6;

  const comboboxProviderOptions = useMemo<ComboboxOption[]>(
    () => (providerOptions ?? []).map((o) => ({ id: o.id, label: o.label })),
    [providerOptions],
  );
  const comboboxModeOptions = useMemo<ComboboxOption[]>(
    () => (modeOptions ?? []).map((o) => ({ id: o.id, label: o.label })),
    [modeOptions],
  );
  const comboboxModelOptions = useMemo<ComboboxOption[]>(
    () => (modelOptions ?? []).map((o) => ({ id: o.id, label: o.label })),
    [modelOptions],
  );
  const fallbackAllProviderModels = useMemo(() => {
    const map = new Map<string, AgentModelDefinition[]>();
    if (!modelOptions || modelOptions.length === 0) {
      return map;
    }

    map.set(
      provider,
      modelOptions.map((option) => ({
        provider: provider as AgentProvider,
        id: option.id,
        label: option.label,
      })),
    );
    return map;
  }, [modelOptions, provider]);
  const effectiveProviderDefinitions = providerDefinitions;
  const effectiveAllProviderModels = allProviderModels ?? fallbackAllProviderModels;
  const displayAllProviderModels = useMemo(() => {
    return new Map(
      Array.from(effectiveAllProviderModels.entries()).map(([providerId, models]) => [
        providerId,
        collapseModelContextVariants(providerId, models),
      ]),
    );
  }, [effectiveAllProviderModels]);
  const displayCloudGroups = useMemo(
    () =>
      cloudGroups.map((group) => ({
        ...group,
        models: collapseModelContextVariants(group.provider, group.models),
      })),
    [cloudGroups],
  );
  const contextModelCandidates = useMemo(() => {
    if (!selectedDisplayModelId) {
      return [];
    }
    const containsSelectedModel = (models: Array<{ id: string }>) =>
      models.some((model) => getBaseModelId(provider, model.id) === selectedDisplayModelId);
    const activeCloudGroup = cloudGroups.find(
      (group) =>
        group.provider === provider &&
        (group.isActiveForGlobalKey || group.isActiveForWorkspace) &&
        containsSelectedModel(group.models),
    );
    if (activeCloudGroup) {
      return activeCloudGroup.models;
    }
    const providerModels = effectiveAllProviderModels.get(provider) ?? [];
    if (containsSelectedModel(providerModels)) {
      return providerModels;
    }
    return (
      cloudGroups.find(
        (group) => group.provider === provider && containsSelectedModel(group.models),
      )?.models ?? providerModels
    );
  }, [cloudGroups, effectiveAllProviderModels, provider, selectedDisplayModelId]);
  const contextWindows = useMemo(
    () =>
      getAvailableModelContextWindows({
        provider,
        modelId: selectedModelId ?? "",
        models: contextModelCandidates,
      }),
    [contextModelCandidates, provider, selectedModelId],
  );
  const selectedContextWindow = getModelContextWindow(provider, selectedModelId ?? "");
  const contextOptions = useMemo<StatusOption[]>(
    () =>
      contextWindows.map((contextWindow) => ({
        id: contextWindow,
        label: contextWindow === "1m" ? "1M" : "256K",
      })),
    [contextWindows],
  );
  const showContextSelector =
    supportsModelContextWindow(provider) &&
    Boolean(selectedDisplayModelId) &&
    contextOptions.length > 0;
  const canSelectContext =
    showContextSelector &&
    contextOptions.length > 1 &&
    Boolean(onSelectModel || onSelectProviderAndModel);
  const canSelectProviderInModelMenu = canSelectModelProvider ?? (() => true);
  const comboboxThinkingOptions = useMemo<ComboboxOption[]>(
    () => localizedThinkingOptions.map((o) => ({ id: o.id, label: o.label })),
    [localizedThinkingOptions],
  );

  const renderModeOption = useCallback(
    ({
      option,
      selected,
      active,
      onPress,
    }: {
      option: ComboboxOption;
      selected: boolean;
      active: boolean;
      onPress: () => void;
    }) => {
      const visuals = getModeVisuals(provider, option.id, providerDefinitions);
      const IconComponent = visuals?.icon ? MODE_ICONS[visuals.icon] : ShieldCheck;
      return (
        <ComboboxItem
          label={option.label}
          selected={selected}
          active={active}
          onPress={onPress}
          leadingSlot={<IconComponent size={16} color={theme.colors.foreground} />}
        />
      );
    },
    [provider, providerDefinitions, theme.colors.foreground],
  );

  const handleOpenChange = useCallback(
    (selector: StatusSelector) => (nextOpen: boolean) => {
      setOpenSelector(nextOpen ? selector : null);
      if (!nextOpen) {
        onDropdownClose?.();
      }
    },
    [onDropdownClose],
  );

  const handleSelectorPress = useCallback(
    (selector: StatusSelector) => {
      handleOpenChange(selector)(openSelector !== selector);
    },
    [handleOpenChange, openSelector],
  );

  const commitModelSelection = useCallback(
    (
      selectedProvider: AgentProvider,
      displayModelId: string,
      candidateModels?: Array<{ id: string }>,
    ) => {
      const contextWindow =
        selectedProvider === provider ? selectedContextWindow : ("256k" as const);
      const models = candidateModels ?? effectiveAllProviderModels.get(selectedProvider) ?? [];
      const modelId = resolveModelIdForContext({
        provider: selectedProvider,
        modelId: displayModelId,
        contextWindow,
        models,
      });
      if (onSelectProviderAndModel) {
        onSelectProviderAndModel(selectedProvider, modelId);
      } else if (selectedProvider === provider) {
        onSelectModel?.(modelId);
      }
    },
    [
      effectiveAllProviderModels,
      onSelectModel,
      onSelectProviderAndModel,
      provider,
      selectedContextWindow,
    ],
  );

  const handleSelectCloudModel = useCallback(
    (selectedProvider: AgentProvider, displayModelId: string, group: SelectorCloudGroup) => {
      const sourceGroup = cloudGroups.find(
        (candidate) => candidate.provider === group.provider && candidate.groupId === group.groupId,
      );
      commitModelSelection(selectedProvider, displayModelId, sourceGroup?.models ?? group.models);
    },
    [cloudGroups, commitModelSelection],
  );

  const handleSelectContextWindow = useCallback(
    (contextWindowId: string) => {
      const contextWindow = contextWindows.find((entry) => entry === contextWindowId);
      if (!contextWindow || !selectedDisplayModelId) {
        return;
      }
      const modelId = resolveModelIdForContext({
        provider,
        modelId: selectedDisplayModelId,
        contextWindow,
        models: contextModelCandidates,
      });
      if (onSelectProviderAndModel) {
        onSelectProviderAndModel(provider, modelId);
      } else {
        onSelectModel?.(modelId);
      }
    },
    [
      contextModelCandidates,
      contextWindows,
      onSelectModel,
      onSelectProviderAndModel,
      provider,
      selectedDisplayModelId,
    ],
  );

  return (
    <View style={styles.container}>
      {platformIsWeb ? (
        <>
          {providerOptions && providerOptions.length > 0 ? (
            <>
              <Pressable
                ref={providerAnchorRef}
                collapsable={false}
                disabled={disabled || !canSelectProvider}
                onPress={() => handleSelectorPress("provider")}
                style={({ pressed, hovered }) => [
                  styles.modeBadge,
                  hovered && styles.modeBadgeHovered,
                  (pressed || openSelector === "provider") && styles.modeBadgePressed,
                  (disabled || !canSelectProvider) && styles.disabledBadge,
                ]}
                accessibilityRole="button"
                accessibilityLabel={text.selectAgentProvider}
                testID="agent-provider-selector"
              >
                <Text style={styles.modeBadgeText} numberOfLines={1} ellipsizeMode="tail">
                  {displayProvider}
                </Text>
                <ChevronDown size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
              </Pressable>
              <Combobox
                options={comboboxProviderOptions}
                value={selectedProviderId ?? ""}
                onSelect={(id) => onSelectProvider?.(id)}
                searchable={comboboxProviderOptions.length > SEARCH_THRESHOLD}
                open={openSelector === "provider"}
                onOpenChange={handleOpenChange("provider")}
                anchorRef={providerAnchorRef}
                desktopPlacement="top-start"
              />
            </>
          ) : null}

          {displayCloudGroup ? (
            <Tooltip delayDuration={0} enabledOnDesktop enabledOnMobile={false}>
              <TooltipTrigger asChild triggerRefProp="ref">
                <View
                  style={[styles.modeBadge, styles.cloudGroupBadge]}
                  testID="agent-cloud-group-status"
                >
                  <Cloud size={theme.iconSize.md} color={theme.colors.foregroundMuted} />
                  <Text
                    style={[styles.modeBadgeText, styles.cloudGroupBadgeText]}
                    numberOfLines={1}
                    ellipsizeMode="tail"
                  >
                    {displayCloudGroup}
                  </Text>
                </View>
              </TooltipTrigger>
              <TooltipContent side="top" align="center" offset={8}>
                <Text style={styles.tooltipText}>{getStatusSelectorHint("cloud-group")}</Text>
              </TooltipContent>
            </Tooltip>
          ) : null}

          {canSelectModel ? (
            <Tooltip
              key={`model-${displayModel}`}
              delayDuration={0}
              enabledOnDesktop
              enabledOnMobile={false}
            >
              <TooltipTrigger asChild triggerRefProp="ref">
                <View style={styles.modelSelectorSlot}>
                  <CombinedModelSelector
                    providerDefinitions={effectiveProviderDefinitions}
                    allProviderModels={displayAllProviderModels}
                    selectedProvider={provider}
                    selectedModel={selectedDisplayModelId}
                    cloudGroups={displayCloudGroups}
                    canSelectProvider={canSelectProviderInModelMenu}
                    onSelect={commitModelSelection}
                    onSelectCloudModel={handleSelectCloudModel}
                    favoriteKeys={favoriteKeys}
                    onToggleFavorite={onToggleFavoriteModel}
                    isLoading={isModelLoading}
                    disabled={modelDisabled}
                    onOpen={onModelSelectorOpen}
                    onClose={onDropdownClose}
                  />
                </View>
              </TooltipTrigger>
              <TooltipContent side="top" align="center" offset={8}>
                <Text style={styles.tooltipText}>{getStatusSelectorHint("model")}</Text>
              </TooltipContent>
            </Tooltip>
          ) : null}

          {showContextSelector ? (
            <>
              <Tooltip delayDuration={0} enabledOnDesktop enabledOnMobile={false}>
                <TooltipTrigger asChild triggerRefProp="ref">
                  <Pressable
                    ref={contextAnchorRef}
                    collapsable={false}
                    disabled={disabled || !canSelectContext}
                    onPress={() => handleSelectorPress("context")}
                    style={({ pressed, hovered }) => [
                      styles.modeBadge,
                      hovered && styles.modeBadgeHovered,
                      (pressed || openSelector === "context") && styles.modeBadgePressed,
                      (disabled || !canSelectContext) && styles.disabledBadge,
                    ]}
                    accessibilityRole="button"
                    accessibilityLabel={text.selectContextWindow(
                      selectedContextWindow === "1m" ? "1M" : "256K",
                    )}
                    testID="agent-context-selector"
                  >
                    <Maximize2 size={theme.iconSize.md} color={theme.colors.foregroundMuted} />
                    <Text style={styles.modeBadgeText} numberOfLines={1}>
                      {selectedContextWindow === "1m" ? "1M" : "256K"}
                    </Text>
                    <ChevronDown size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
                  </Pressable>
                </TooltipTrigger>
                <TooltipContent side="top" align="center" offset={8}>
                  <Text style={styles.tooltipText}>{getStatusSelectorHint("context")}</Text>
                </TooltipContent>
              </Tooltip>
              <Combobox
                options={contextOptions}
                value={selectedContextWindow}
                onSelect={handleSelectContextWindow}
                searchable={false}
                open={openSelector === "context"}
                onOpenChange={handleOpenChange("context")}
                anchorRef={contextAnchorRef}
                desktopPlacement="top-start"
              />
            </>
          ) : null}

          {thinkingOptions && thinkingOptions.length > 0 ? (
            <>
              <Tooltip delayDuration={0} enabledOnDesktop enabledOnMobile={false}>
                <TooltipTrigger asChild triggerRefProp="ref">
                  <Pressable
                    ref={thinkingAnchorRef}
                    collapsable={false}
                    disabled={disabled || !canSelectThinking}
                    onPress={() => handleSelectorPress("thinking")}
                    style={({ pressed, hovered }) => [
                      styles.modeBadge,
                      hovered && styles.modeBadgeHovered,
                      (pressed || openSelector === "thinking") && styles.modeBadgePressed,
                      (disabled || !canSelectThinking) && styles.disabledBadge,
                    ]}
                    accessibilityRole="button"
                    accessibilityLabel={text.selectThinkingOption(displayThinking)}
                    testID="agent-thinking-selector"
                  >
                    <Brain size={theme.iconSize.md} color={theme.colors.foregroundMuted} />
                    <Text style={styles.modeBadgeText} numberOfLines={1} ellipsizeMode="tail">
                      {displayThinking}
                    </Text>
                    <ChevronDown size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
                  </Pressable>
                </TooltipTrigger>
                <TooltipContent side="top" align="center" offset={8}>
                  <Text style={styles.tooltipText}>{getStatusSelectorHint("thinking")}</Text>
                </TooltipContent>
              </Tooltip>
              <Combobox
                options={comboboxThinkingOptions}
                value={selectedThinkingOptionId ?? ""}
                onSelect={(id) => onSelectThinkingOption?.(id)}
                searchable={comboboxThinkingOptions.length > SEARCH_THRESHOLD}
                open={openSelector === "thinking"}
                onOpenChange={handleOpenChange("thinking")}
                anchorRef={thinkingAnchorRef}
                desktopPlacement="top-start"
              />
            </>
          ) : null}

          {modeOptions && modeOptions.length > 0 ? (
            <>
              <Tooltip delayDuration={0} enabledOnDesktop enabledOnMobile={false}>
                <TooltipTrigger asChild triggerRefProp="ref">
                  <Pressable
                    ref={modeAnchorRef}
                    collapsable={false}
                    disabled={disabled || !canSelectMode}
                    onPress={() => handleSelectorPress("mode")}
                    style={({ pressed, hovered }) => [
                      styles.modeBadge,
                      hovered && styles.modeBadgeHovered,
                      (pressed || openSelector === "mode") && styles.modeBadgePressed,
                      (disabled || !canSelectMode) && styles.disabledBadge,
                    ]}
                    accessibilityRole="button"
                    accessibilityLabel={text.selectAgentMode(displayMode)}
                    testID="agent-mode-selector"
                  >
                    {ModeIconComponent ? (
                      <ModeIconComponent size={theme.iconSize.md} color={modeIconColor} />
                    ) : (
                      <ShieldCheck size={theme.iconSize.md} color={theme.colors.foregroundMuted} />
                    )}
                    <Text style={styles.modeBadgeText} numberOfLines={1}>
                      {displayMode}
                    </Text>
                    <ChevronDown size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
                  </Pressable>
                </TooltipTrigger>
                <TooltipContent side="top" align="center" offset={8}>
                  <Text style={styles.tooltipText}>{getStatusSelectorHint("mode")}</Text>
                </TooltipContent>
              </Tooltip>
              <Combobox
                options={comboboxModeOptions}
                value={selectedModeId ?? ""}
                onSelect={(id) => onSelectMode?.(id)}
                searchable={comboboxModeOptions.length > SEARCH_THRESHOLD}
                open={openSelector === "mode"}
                onOpenChange={handleOpenChange("mode")}
                anchorRef={modeAnchorRef}
                desktopPlacement="top-start"
                renderOption={renderModeOption}
              />
            </>
          ) : null}

          {features?.map((feature) => {
            if (feature.type === "toggle") {
              const FeatureIcon = getFeatureIcon(feature.icon);
              return (
                <Tooltip
                  key={`feature-${feature.id}`}
                  delayDuration={0}
                  enabledOnDesktop
                  enabledOnMobile={false}
                >
                  <TooltipTrigger asChild triggerRefProp="ref">
                    <Pressable
                      disabled={disabled}
                      onPress={() => onSetFeature?.(feature.id, !feature.value)}
                      style={({ pressed, hovered }) => [
                        styles.modeIconBadge,
                        hovered && styles.modeBadgeHovered,
                        pressed && styles.modeBadgePressed,
                        disabled && styles.disabledBadge,
                      ]}
                      accessibilityRole="button"
                      accessibilityLabel={getFeatureTooltip(feature)}
                      testID={`agent-feature-${feature.id}`}
                    >
                      <FeatureIcon
                        size={theme.iconSize.md}
                        color={getFeatureIconColor(
                          feature.id,
                          feature.value,
                          theme.colors.palette,
                          theme.colors.foregroundMuted,
                        )}
                      />
                    </Pressable>
                  </TooltipTrigger>
                  <TooltipContent side="top" align="center" offset={8}>
                    <Text style={styles.tooltipText}>{getFeatureTooltip(feature)}</Text>
                  </TooltipContent>
                </Tooltip>
              );
            }
            if (feature.type === "select") {
              const FeatureIcon = getFeatureIcon(feature.icon);
              const selectedOption = feature.options.find((o) => o.id === feature.value);
              return (
                <DropdownMenu
                  key={`feature-${feature.id}`}
                  open={openSelector === `feature-${feature.id}`}
                  onOpenChange={handleOpenChange(`feature-${feature.id}`)}
                >
                  <Tooltip delayDuration={0} enabledOnDesktop enabledOnMobile={false}>
                    <TooltipTrigger asChild triggerRefProp="ref">
                      <DropdownMenuTrigger
                        disabled={disabled}
                        style={({ pressed, hovered }) => [
                          styles.modeBadge,
                          hovered && styles.modeBadgeHovered,
                          (pressed || openSelector === `feature-${feature.id}`) &&
                            styles.modeBadgePressed,
                          disabled && styles.disabledBadge,
                        ]}
                        accessibilityRole="button"
                        accessibilityLabel={getFeatureTooltip(feature)}
                        testID={`agent-feature-${feature.id}`}
                      >
                        <FeatureIcon
                          size={theme.iconSize.md}
                          color={theme.colors.foregroundMuted}
                        />
                        <Text style={styles.modeBadgeText} numberOfLines={1} ellipsizeMode="tail">
                          {selectedOption?.label ?? feature.label}
                        </Text>
                        <ChevronDown
                          size={theme.iconSize.sm}
                          color={theme.colors.foregroundMuted}
                        />
                      </DropdownMenuTrigger>
                    </TooltipTrigger>
                    <TooltipContent side="top" align="center" offset={8}>
                      <Text style={styles.tooltipText}>{getFeatureTooltip(feature)}</Text>
                    </TooltipContent>
                  </Tooltip>
                  <DropdownMenuContent side="top" align="start">
                    {feature.options.map((option) => (
                      <DropdownMenuItem
                        key={option.id}
                        selected={option.id === feature.value}
                        onSelect={() => onSetFeature?.(feature.id, option.id)}
                      >
                        {option.label}
                      </DropdownMenuItem>
                    ))}
                  </DropdownMenuContent>
                </DropdownMenu>
              );
            }
            return null;
          })}
        </>
      ) : (
        <>
          <Pressable
            onPress={() => {
              Keyboard.dismiss();
              setPrefsOpen(true);
            }}
            style={({ pressed }) => [styles.prefsButton, pressed && styles.prefsButtonPressed]}
            accessibilityRole="button"
            accessibilityLabel={text.agentPreferences}
            testID="agent-preferences-button"
          >
            {ProviderIcon ? (
              <ProviderIcon size={theme.iconSize.lg} color={theme.colors.foregroundMuted} />
            ) : null}
            <Text style={styles.prefsButtonText} numberOfLines={1}>
              {displayModel}
            </Text>
          </Pressable>

          <AdaptiveModalSheet
            title={text.preferences}
            visible={prefsOpen}
            onClose={() => setPrefsOpen(false)}
            testID="agent-preferences-sheet"
          >
            {providerOptions && providerOptions.length > 0 ? (
              <View style={styles.sheetSection}>
                <Pressable
                  ref={providerAnchorRef}
                  collapsable={false}
                  disabled={disabled || !canSelectProvider}
                  onPress={() => handleSelectorPress("provider")}
                  style={({ pressed }) => [
                    styles.sheetSelect,
                    pressed && styles.sheetSelectPressed,
                    (disabled || !canSelectProvider) && styles.disabledSheetSelect,
                  ]}
                  accessibilityRole="button"
                  accessibilityLabel={text.selectAgentProvider}
                  testID="agent-preferences-provider"
                >
                  {ProviderIcon ? (
                    <ProviderIcon size={theme.iconSize.md} color={theme.colors.foregroundMuted} />
                  ) : null}
                  <Text style={styles.sheetSelectText}>{displayProvider}</Text>
                  <ChevronDown size={theme.iconSize.md} color={theme.colors.foregroundMuted} />
                </Pressable>
                <Combobox
                  options={comboboxProviderOptions}
                  value={selectedProviderId ?? ""}
                  onSelect={(id) => onSelectProvider?.(id)}
                  searchable={comboboxProviderOptions.length > SEARCH_THRESHOLD}
                  title={text.selectAgentProvider}
                  open={openSelector === "provider"}
                  onOpenChange={handleOpenChange("provider")}
                  anchorRef={providerAnchorRef}
                />
              </View>
            ) : null}

            {displayCloudGroup ? (
              <View style={styles.sheetSection}>
                <View
                  style={styles.sheetSelect}
                  accessibilityLabel={text.currentCloudGroup}
                  testID="agent-preferences-cloud-group"
                >
                  <Cloud size={theme.iconSize.md} color={theme.colors.foregroundMuted} />
                  <Text style={styles.sheetSelectText}>{displayCloudGroup}</Text>
                </View>
              </View>
            ) : null}

            {canSelectModel ? (
              <View style={styles.sheetSection}>
                <CombinedModelSelector
                  providerDefinitions={effectiveProviderDefinitions}
                  allProviderModels={displayAllProviderModels}
                  selectedProvider={provider}
                  selectedModel={selectedDisplayModelId}
                  cloudGroups={displayCloudGroups}
                  canSelectProvider={canSelectProviderInModelMenu}
                  onSelect={commitModelSelection}
                  onSelectCloudModel={handleSelectCloudModel}
                  favoriteKeys={favoriteKeys}
                  onToggleFavorite={onToggleFavoriteModel}
                  isLoading={isModelLoading}
                  disabled={modelDisabled}
                  onOpen={onModelSelectorOpen}
                  onClose={onDropdownClose}
                  renderTrigger={({ selectedModelLabel }) => (
                    <View
                      style={[styles.sheetSelect, modelDisabled && styles.disabledSheetSelect]}
                      pointerEvents="none"
                      testID="agent-preferences-model"
                    >
                      {ProviderIcon ? (
                        <ProviderIcon
                          size={theme.iconSize.md}
                          color={theme.colors.foregroundMuted}
                        />
                      ) : null}
                      <Text style={styles.sheetSelectText}>{selectedModelLabel}</Text>
                      <ChevronDown size={theme.iconSize.md} color={theme.colors.foregroundMuted} />
                    </View>
                  )}
                />
              </View>
            ) : null}

            {showContextSelector ? (
              <View style={styles.sheetSection}>
                <DropdownMenu
                  open={openSelector === "context"}
                  onOpenChange={handleOpenChange("context")}
                >
                  <DropdownMenuTrigger
                    disabled={disabled || !canSelectContext}
                    style={({ pressed }) => [
                      styles.sheetSelect,
                      pressed && styles.sheetSelectPressed,
                      (disabled || !canSelectContext) && styles.disabledSheetSelect,
                    ]}
                    accessibilityRole="button"
                    accessibilityLabel={text.selectContextWindow(
                      selectedContextWindow === "1m" ? "1M" : "256K",
                    )}
                    testID="agent-preferences-context"
                  >
                    <Maximize2 size={theme.iconSize.md} color={theme.colors.foregroundMuted} />
                    <Text style={styles.sheetSelectText}>{text.contextWindow}</Text>
                    <Text style={styles.modeBadgeText}>
                      {selectedContextWindow === "1m" ? "1M" : "256K"}
                    </Text>
                    <ChevronDown size={theme.iconSize.md} color={theme.colors.foregroundMuted} />
                  </DropdownMenuTrigger>
                  <DropdownMenuContent side="top" align="start">
                    {contextOptions.map((contextOption) => (
                      <DropdownMenuItem
                        key={contextOption.id}
                        selected={contextOption.id === selectedContextWindow}
                        onSelect={() => handleSelectContextWindow(contextOption.id)}
                      >
                        {contextOption.label}
                      </DropdownMenuItem>
                    ))}
                  </DropdownMenuContent>
                </DropdownMenu>
              </View>
            ) : null}

            {thinkingOptions && thinkingOptions.length > 0 ? (
              <View style={styles.sheetSection}>
                <DropdownMenu
                  open={openSelector === "thinking"}
                  onOpenChange={handleOpenChange("thinking")}
                >
                  <DropdownMenuTrigger
                    disabled={disabled || !canSelectThinking}
                    style={({ pressed }) => [
                      styles.sheetSelect,
                      pressed && styles.sheetSelectPressed,
                      (disabled || !canSelectThinking) && styles.disabledSheetSelect,
                    ]}
                    accessibilityRole="button"
                    accessibilityLabel={text.selectThinkingOption(displayThinking)}
                    testID="agent-preferences-thinking"
                  >
                    <Brain size={theme.iconSize.md} color={theme.colors.foregroundMuted} />
                    <Text style={styles.sheetSelectText}>{displayThinking}</Text>
                    <ChevronDown size={theme.iconSize.md} color={theme.colors.foregroundMuted} />
                  </DropdownMenuTrigger>
                  <DropdownMenuContent side="top" align="start">
                    {localizedThinkingOptions.map((thinking) => (
                      <DropdownMenuItem
                        key={thinking.id}
                        selected={thinking.id === selectedThinkingOptionId}
                        onSelect={() => onSelectThinkingOption?.(thinking.id)}
                      >
                        {thinking.label}
                      </DropdownMenuItem>
                    ))}
                  </DropdownMenuContent>
                </DropdownMenu>
              </View>
            ) : null}

            {modeOptions && modeOptions.length > 0 ? (
              <View style={styles.sheetSection}>
                <DropdownMenu
                  open={openSelector === "mode"}
                  onOpenChange={handleOpenChange("mode")}
                >
                  <DropdownMenuTrigger
                    disabled={disabled || !canSelectMode}
                    style={({ pressed }) => [
                      styles.sheetSelect,
                      pressed && styles.sheetSelectPressed,
                      (disabled || !canSelectMode) && styles.disabledSheetSelect,
                    ]}
                    accessibilityRole="button"
                    accessibilityLabel={text.selectAgentMode(displayMode)}
                    testID="agent-preferences-mode"
                  >
                    {ModeIconComponent ? (
                      <ModeIconComponent size={theme.iconSize.md} color={modeIconColor} />
                    ) : null}
                    <Text style={styles.sheetSelectText}>{displayMode}</Text>
                    <ChevronDown size={theme.iconSize.md} color={theme.colors.foregroundMuted} />
                  </DropdownMenuTrigger>
                  <DropdownMenuContent side="top" align="start">
                    {modeOptions.map((mode) => {
                      const visuals = getModeVisuals(provider, mode.id, providerDefinitions);
                      const Icon = visuals?.icon ? MODE_ICONS[visuals.icon] : ShieldCheck;
                      return (
                        <DropdownMenuItem
                          key={mode.id}
                          selected={mode.id === selectedModeId}
                          onSelect={() => onSelectMode?.(mode.id)}
                          leading={<Icon size={16} color={theme.colors.foreground} />}
                        >
                          {mode.label}
                        </DropdownMenuItem>
                      );
                    })}
                  </DropdownMenuContent>
                </DropdownMenu>
              </View>
            ) : null}

            {features?.map((feature) => {
              if (feature.type === "toggle") {
                const FeatureIcon = getFeatureIcon(feature.icon);
                return (
                  <View key={`feature-${feature.id}`} style={styles.sheetSection}>
                    <Pressable
                      disabled={disabled}
                      onPress={() => onSetFeature?.(feature.id, !feature.value)}
                      style={({ pressed }) => [
                        styles.sheetSelect,
                        pressed && styles.sheetSelectPressed,
                        disabled && styles.disabledSheetSelect,
                      ]}
                      accessibilityRole="button"
                      accessibilityLabel={getFeatureTooltip(feature)}
                      testID={`agent-feature-${feature.id}`}
                    >
                      <FeatureIcon
                        size={theme.iconSize.md}
                        color={getFeatureIconColor(
                          feature.id,
                          feature.value,
                          theme.colors.palette,
                          theme.colors.foregroundMuted,
                        )}
                      />
                      <Text style={styles.sheetSelectText}>{feature.label}</Text>
                      <Text style={styles.modeBadgeText}>{feature.value ? text.on : text.off}</Text>
                    </Pressable>
                  </View>
                );
              }
              if (feature.type === "select") {
                const selectedOption = feature.options.find((o) => o.id === feature.value);
                return (
                  <View key={`feature-${feature.id}`} style={styles.sheetSection}>
                    <DropdownMenu
                      open={openSelector === `feature-${feature.id}`}
                      onOpenChange={handleOpenChange(`feature-${feature.id}`)}
                    >
                      <DropdownMenuTrigger
                        disabled={disabled}
                        style={({ pressed }) => [
                          styles.sheetSelect,
                          pressed && styles.sheetSelectPressed,
                          disabled && styles.disabledSheetSelect,
                        ]}
                        accessibilityRole="button"
                        accessibilityLabel={getFeatureTooltip(feature)}
                        testID={`agent-feature-${feature.id}`}
                      >
                        <Text style={styles.sheetSelectText}>
                          {selectedOption?.label ?? feature.label}
                        </Text>
                        <ChevronDown
                          size={theme.iconSize.md}
                          color={theme.colors.foregroundMuted}
                        />
                      </DropdownMenuTrigger>
                      <DropdownMenuContent side="top" align="start">
                        {feature.options.map((option) => (
                          <DropdownMenuItem
                            key={option.id}
                            selected={option.id === feature.value}
                            onSelect={() => onSetFeature?.(feature.id, option.id)}
                          >
                            {option.label}
                          </DropdownMenuItem>
                        ))}
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </View>
                );
              }
              return null;
            })}
          </AdaptiveModalSheet>
        </>
      )}
    </View>
  );
}

const EMPTY_MODES: AgentMode[] = [];

export const AgentStatusBar = memo(function AgentStatusBar({
  agentId,
  serverId,
  onDropdownClose,
}: AgentStatusBarProps) {
  const { preferences, updatePreferences } = useFormPreferences();
  const agent = useSessionStore(
    useShallow((state) => {
      const currentAgent = state.sessions[serverId]?.agents?.get(agentId) ?? null;
      return currentAgent
        ? {
            provider: currentAgent.provider,
            cwd: currentAgent.cwd,
            currentModeId: currentAgent.currentModeId,
            runtimeModelId: currentAgent.runtimeInfo?.model ?? null,
            model: currentAgent.model,
            features: currentAgent.features,
            thinkingOptionId: currentAgent.thinkingOptionId,
            lastUsage: currentAgent.lastUsage,
          }
        : null;
    }),
  );
  const availableModes = useStoreWithEqualityFn(
    useSessionStore,
    (state) => state.sessions[serverId]?.agents?.get(agentId)?.availableModes ?? EMPTY_MODES,
    (a, b) => a === b || JSON.stringify(a) === JSON.stringify(b),
  );
  const client = useSessionStore((state) => state.sessions[serverId]?.client ?? null);
  const toast = useToast();
  const locale = useAppLocale();

  const {
    entries: snapshotEntries,
    isLoading: snapshotIsLoading,
    refetchIfStale: refetchSnapshotIfStale,
  } = useProvidersSnapshot(serverId, agent?.cwd);

  const snapshotSelectedEntry = useMemo(() => {
    if (!snapshotEntries || !agent?.provider) {
      return null;
    }
    return snapshotEntries.find((e) => e.provider === agent.provider) ?? null;
  }, [snapshotEntries, agent?.provider]);

  const models = snapshotSelectedEntry?.models ?? null;
  const selectedProviderIsLoading = snapshotSelectedEntry?.status === "loading";

  const agentProviderDefinitions = useMemo(() => {
    const definition = agent?.provider
      ? resolveProviderDefinition(agent.provider, snapshotEntries)
      : undefined;
    return definition ? [definition] : [];
  }, [agent?.provider, snapshotEntries]);
  const agentProviderModels = useMemo(() => {
    const map = new Map<string, AgentModelDefinition[]>();
    if (agent?.provider && models) {
      map.set(agent.provider, models);
    }
    return map;
  }, [agent?.provider, models]);
  const { cloudGroups } = useCloudModelRouting({
    serverId,
    cwd: agent?.cwd,
    providerDefinitions: agentProviderDefinitions,
    allProviderModels: agentProviderModels,
  });

  const displayMode = agent?.currentModeId
    ? localizeAgentModeLabel(
        {
          id: agent.currentModeId,
          label:
            availableModes.find((mode) => mode.id === agent.currentModeId)?.label ||
            agent.currentModeId,
        },
        locale,
      )
    : localizeAgentModeLabel({ id: "default", label: "default" }, locale);

  const modelSelection = resolveAgentModelSelection({
    models,
    runtimeModelId: agent?.runtimeModelId,
    configuredModelId: agent?.model,
    explicitThinkingOptionId: agent?.thinkingOptionId,
  });

  const modeOptions = useMemo<StatusOption[]>(() => {
    return availableModes.map((mode) => {
      const localizedMode = localizeAgentMode(mode, locale);
      return {
        id: localizedMode.id,
        label: localizedMode.label,
      };
    });
  }, [availableModes, locale]);

  const modelOptions = useMemo<StatusOption[]>(() => {
    return (models ?? []).map((model) => ({
      id: model.id,
      label: model.label,
    }));
  }, [models]);
  const favoriteKeys = useMemo(
    () =>
      new Set(
        (preferences.favoriteModels ?? []).map((favorite) => buildFavoriteModelKey(favorite)),
      ),
    [preferences.favoriteModels],
  );

  const thinkingOptions = useMemo<StatusOption[]>(() => {
    return (modelSelection.thinkingOptions ?? []).map((option) => ({
      id: option.id,
      label: localizeThinkingOption(option, getAppMessages(locale).workspace).label,
    }));
  }, [locale, modelSelection.thinkingOptions]);

  const handleSelectGlobalModel = useCallback(
    (modelId: string) => {
      if (!client || !agent?.provider) {
        return;
      }

      void (async () => {
        await updatePreferences((current) =>
          mergeProviderPreferences({
            preferences: current,
            provider: agent.provider,
            updates: {
              model: modelId,
            },
          }),
        );
        await client.setAgentModel(agentId, modelId);
      })().catch((error) => {
        console.warn("[AgentStatusBar] setAgentModel failed", error);
        toast.error(toErrorMessage(error));
      });
    },
    [agent?.provider, agentId, client, toast, updatePreferences],
  );

  if (!agent) {
    return null;
  }

  return (
    <ControlledStatusBar
      provider={agent.provider}
      modeOptions={
        modeOptions.length > 0
          ? modeOptions
          : [{ id: agent.currentModeId ?? "", label: displayMode }]
      }
      selectedModeId={agent.currentModeId ?? undefined}
      providerDefinitions={agentProviderDefinitions}
      allProviderModels={agentProviderModels}
      onSelectMode={(modeId) => {
        if (!client) {
          return;
        }
        void client.setAgentMode(agentId, modeId).catch((error) => {
          console.warn("[AgentStatusBar] setAgentMode failed", error);
          toast.error(toErrorMessage(error));
        });
      }}
      modelOptions={modelOptions}
      selectedModelId={modelSelection.activeModelId ?? undefined}
      onSelectModel={handleSelectGlobalModel}
      favoriteKeys={favoriteKeys}
      onToggleFavoriteModel={(provider, modelId) => {
        void updatePreferences((current) =>
          toggleFavoriteModel({ preferences: current, provider, modelId }),
        ).catch((error) => {
          console.warn("[AgentStatusBar] toggle favorite model failed", error);
        });
      }}
      cloudGroups={cloudGroups}
      thinkingOptions={thinkingOptions.length > 1 ? thinkingOptions : undefined}
      selectedThinkingOptionId={modelSelection.selectedThinkingId ?? undefined}
      onSelectThinkingOption={(thinkingOptionId) => {
        if (!client) {
          return;
        }
        const activeModelId = modelSelection.activeModelId;
        if (activeModelId) {
          void updatePreferences((current) =>
            mergeProviderPreferences({
              preferences: current,
              provider: agent.provider,
              updates: {
                model: activeModelId,
                thinkingByModel: {
                  [activeModelId]: thinkingOptionId,
                },
              },
            }),
          ).catch((error) => {
            console.warn("[AgentStatusBar] persist thinking preference failed", error);
          });
        }
        void client.setAgentThinkingOption(agentId, thinkingOptionId).catch((error) => {
          console.warn("[AgentStatusBar] setAgentThinkingOption failed", error);
          toast.error(toErrorMessage(error));
        });
      }}
      features={agent.features}
      onSetFeature={(featureId, value) => {
        if (!client) {
          return;
        }
        void updatePreferences((current) =>
          mergeProviderPreferences({
            preferences: current,
            provider: agent.provider,
            updates: {
              featureValues: {
                [featureId]: value,
              },
            },
          }),
        ).catch((error) => {
          console.warn("[AgentStatusBar] persist feature preference failed", error);
        });
        void client.setAgentFeature(agentId, featureId, value).catch((error) => {
          console.warn("[AgentStatusBar] setAgentFeature failed", error);
          toast.error(toErrorMessage(error));
        });
      }}
      isModelLoading={snapshotIsLoading || selectedProviderIsLoading}
      onModelSelectorOpen={() => refetchSnapshotIfStale(agent?.provider)}
      onDropdownClose={onDropdownClose}
      disabled={!client}
    />
  );
});

export function DraftAgentStatusBar({
  serverId,
  cwd,
  providerDefinitions,
  selectableProviderIds,
  selectedProvider,
  onSelectProvider,
  modeOptions,
  selectedMode,
  onSelectMode,
  models,
  selectedModel,
  onSelectModel,
  isModelLoading,
  allProviderModels,
  onSelectProviderAndModel,
  thinkingOptions,
  selectedThinkingOptionId,
  onSelectThinkingOption,
  features,
  onSetFeature,
  onDropdownClose,
  onModelSelectorOpen,
  disabled = false,
}: DraftAgentStatusBarProps) {
  const locale = useAppLocale();
  const text = useMemo(() => getAppMessages(locale).workspace, [locale]);
  const { preferences, updatePreferences } = useFormPreferences();
  const { cloudGroups } = useCloudModelRouting({
    serverId,
    cwd,
    providerDefinitions,
    allProviderModels,
  });

  const selectableProviderDefinitions = useMemo(
    () => filterSelectableProviderDefinitions(providerDefinitions, selectableProviderIds),
    [providerDefinitions, selectableProviderIds],
  );
  const providerOptions = useMemo<StatusOption[]>(
    () =>
      selectableProviderDefinitions.map((definition) => ({
        id: definition.id,
        label: definition.label,
      })),
    [selectableProviderDefinitions],
  );
  const selectedProviderModels = useMemo(
    () => scopeModelsToProvider(selectedProvider, allProviderModels, models),
    [allProviderModels, models, selectedProvider],
  );
  const selectedProviderCloudGroups = useMemo(
    () => cloudGroupsForStatusProvider(cloudGroups, selectedProvider ?? ""),
    [cloudGroups, selectedProvider],
  );

  const mappedModeOptions = useMemo<StatusOption[]>(() => {
    if (modeOptions.length === 0) {
      return [{ id: "", label: text.defaultMode }];
    }
    return modeOptions.map((mode) => {
      const localizedMode = localizeAgentMode(mode, locale);
      return {
        id: localizedMode.id,
        label: localizedMode.label,
      };
    });
  }, [locale, modeOptions, text.defaultMode]);

  const mappedThinkingOptions = useMemo<StatusOption[]>(() => {
    return thinkingOptions.map((option) => ({
      id: option.id,
      label: localizeThinkingOption(option, text).label,
    }));
  }, [text, thinkingOptions]);
  const favoriteKeys = useMemo(
    () =>
      new Set(
        (preferences.favoriteModels ?? []).map((favorite) => buildFavoriteModelKey(favorite)),
      ),
    [preferences.favoriteModels],
  );

  const handleSelectGlobalModel = useCallback(
    (provider: AgentProvider, modelId: string) => {
      onSelectProviderAndModel(provider, modelId);
    },
    [onSelectProviderAndModel],
  );

  const effectiveSelectedMode = selectedMode || mappedModeOptions[0]?.id || "";
  const effectiveSelectedThinkingOption =
    selectedThinkingOptionId || mappedThinkingOptions[0]?.id || undefined;
  const hasSelectedProvider = selectedProvider !== null;

  const modelOptions: StatusOption[] = models.map((model) => ({
    id: model.id,
    label: model.label,
  }));

  return (
    <>
      <ControlledStatusBar
        provider={selectedProvider ?? ""}
        providerOptions={providerOptions}
        selectedProviderId={selectedProvider ?? undefined}
        onSelectProvider={(providerId) => onSelectProvider(providerId as AgentProvider)}
        providerDefinitions={providerDefinitions}
        allProviderModels={selectedProviderModels}
        modeOptions={hasSelectedProvider ? mappedModeOptions : undefined}
        selectedModeId={effectiveSelectedMode}
        onSelectMode={onSelectMode}
        modelOptions={modelOptions}
        selectedModelId={selectedModel}
        onSelectModel={(modelId) => {
          if (selectedProvider) {
            handleSelectGlobalModel(selectedProvider, modelId);
          } else {
            onSelectModel(modelId);
          }
        }}
        onSelectProviderAndModel={handleSelectGlobalModel}
        cloudGroups={selectedProviderCloudGroups}
        isModelLoading={isModelLoading}
        favoriteKeys={favoriteKeys}
        onToggleFavoriteModel={(provider, modelId) => {
          void updatePreferences((current) =>
            toggleFavoriteModel({ preferences: current, provider, modelId }),
          ).catch((error) => {
            console.warn("[DraftAgentStatusBar] toggle favorite model failed", error);
          });
        }}
        thinkingOptions={mappedThinkingOptions.length > 0 ? mappedThinkingOptions : undefined}
        selectedThinkingOptionId={effectiveSelectedThinkingOption}
        onSelectThinkingOption={onSelectThinkingOption}
        features={features}
        onSetFeature={onSetFeature}
        onModelSelectorOpen={onModelSelectorOpen}
        disabled={disabled}
      />
    </>
  );
}

const styles = StyleSheet.create((theme) => ({
  container: {
    flexDirection: "row",
    alignItems: "flex-end",
    gap: theme.spacing[1],
    minWidth: 0,
    maxWidth: "100%",
    flexShrink: 1,
    overflow: "hidden",
  },
  modeBadge: {
    height: 28,
    minWidth: 0,
    maxWidth: 220,
    flexShrink: 1,
    flexDirection: "row",
    alignItems: "center",
    backgroundColor: "transparent",
    gap: theme.spacing[1],
    paddingHorizontal: theme.spacing[2],
    borderRadius: theme.borderRadius["2xl"],
  },
  cloudGroupBadge: {
    maxWidth: 260,
    flexShrink: 2,
  },
  cloudGroupBadgeText: {
    minWidth: 0,
    flexShrink: 1,
  },
  modelSelectorSlot: {
    minWidth: 0,
    flexShrink: 1,
  },
  modeIconBadge: {
    width: 28,
    height: 28,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: "transparent",
    borderRadius: theme.borderRadius.full,
  },
  modeBadgeHovered: {
    backgroundColor: theme.colors.surface2,
  },
  modeBadgePressed: {
    backgroundColor: theme.colors.surface0,
  },
  disabledBadge: {
    opacity: 0.5,
  },
  modeBadgeText: {
    minWidth: 0,
    flexShrink: 1,
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.normal,
  },
  tooltipText: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
    lineHeight: theme.fontSize.sm * 1.4,
  },
  prefsButton: {
    height: 28,
    minWidth: 0,
    flexShrink: 1,
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[1],
    paddingHorizontal: theme.spacing[2],
    borderRadius: theme.borderRadius["2xl"],
  },
  prefsButtonPressed: {
    backgroundColor: theme.colors.surface0,
  },
  prefsButtonText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.normal,
    flexShrink: 1,
  },
  sheetSection: {
    gap: theme.spacing[2],
  },
  sheetSelect: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: theme.spacing[3],
    paddingHorizontal: theme.spacing[4],
    paddingVertical: theme.spacing[3],
    borderRadius: theme.borderRadius.lg,
    borderWidth: 1,
    borderColor: theme.colors.surface2,
    backgroundColor: theme.colors.surface0,
  },
  sheetSelectPressed: {
    backgroundColor: theme.colors.surface2,
  },
  disabledSheetSelect: {
    opacity: 0.5,
  },
  sheetSelectText: {
    flex: 1,
    color: theme.colors.foreground,
    fontSize: theme.fontSize.base,
    fontWeight: theme.fontWeight.semibold,
  },
}));

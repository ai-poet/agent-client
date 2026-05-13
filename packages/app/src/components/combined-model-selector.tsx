import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  View,
  Text,
  TextInput,
  Pressable,
  ActivityIndicator,
  type GestureResponderEvent,
} from "react-native";
import { BottomSheetTextInput } from "@gorhom/bottom-sheet";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { useIsCompactFormFactor } from "@/constants/layout";
import { isWeb as platformIsWeb } from "@/constants/platform";
import { ArrowLeft, ChevronDown, ChevronRight, Cloud, Search, Star } from "lucide-react-native";
import type { AgentModelDefinition, AgentProvider } from "@server/server/agent/agent-sdk-types";
import type { AgentProviderDefinition } from "@server/server/agent/provider-manifest";
const IS_WEB = platformIsWeb;

import { Combobox, ComboboxItem } from "@/components/ui/combobox";
import { getProviderIcon } from "@/components/provider-icons";
import type { FavoriteModelRow } from "@/hooks/use-form-preferences";
import { useAppLocale } from "@/hooks/use-app-locale";
import { getAppMessages } from "@/i18n/sub2api";
import {
  buildModelRows,
  buildCloudGroupModelRows,
  buildOtherAvailableModelRows,
  buildSelectedTriggerLabel,
  cloudGroupsForProvider,
  matchesSearch,
  resolveProviderLabel,
  type SelectorCloudGroup,
  type SelectorModelRow,
} from "./combined-model-selector.utils";

// TODO: this should be configured per provider in the provider manifest
const PROVIDERS_WITH_MODEL_DESCRIPTIONS = new Set(["opencode", "pi"]);

type SelectorView =
  | { kind: "all" }
  | { kind: "provider"; providerId: string; providerLabel: string }
  | {
      kind: "cloudGroup";
      providerId: string;
      providerLabel: string;
      groupId: number;
      groupLabel: string;
    };

interface CombinedModelSelectorProps {
  providerDefinitions: AgentProviderDefinition[];
  allProviderModels: Map<string, AgentModelDefinition[]>;
  selectedProvider: string;
  selectedModel: string;
  onSelect: (provider: AgentProvider, modelId: string) => void;
  cloudGroups?: SelectorCloudGroup[];
  onSelectCloudModel?: (
    provider: AgentProvider,
    modelId: string,
    group: SelectorCloudGroup,
  ) => void;
  isLoading: boolean;
  canSelectProvider?: (provider: string) => boolean;
  favoriteKeys?: Set<string>;
  onToggleFavorite?: (provider: string, modelId: string) => void;
  renderTrigger?: (input: {
    selectedModelLabel: string;
    onPress: () => void;
    disabled: boolean;
    isOpen: boolean;
  }) => React.ReactNode;
  onOpen?: () => void;
  onClose?: () => void;
  disabled?: boolean;
}

interface SelectorContentProps {
  view: SelectorView;
  providerDefinitions: AgentProviderDefinition[];
  allProviderModels: Map<string, AgentModelDefinition[]>;
  selectedProvider: string;
  selectedModel: string;
  searchQuery: string;
  onSearchChange: (query: string) => void;
  favoriteKeys: Set<string>;
  onSelect: (provider: string, modelId: string) => void;
  cloudGroups: SelectorCloudGroup[];
  onSelectCloudModel?: (provider: string, modelId: string, group: SelectorCloudGroup) => void;
  canSelectProvider: (provider: string) => boolean;
  onToggleFavorite?: (provider: string, modelId: string) => void;
  onDrillDown: (providerId: string, providerLabel: string) => void;
  onDrillDownCloudGroup: (
    providerId: string,
    providerLabel: string,
    groupId: number,
    groupLabel: string,
  ) => void;
}

function resolveDefaultModelLabel(
  models: AgentModelDefinition[] | undefined,
  fallbackLabel: string,
): string {
  if (!models || models.length === 0) {
    return fallbackLabel;
  }
  return (models.find((model) => model.isDefault) ?? models[0])?.label ?? fallbackLabel;
}

function normalizeSearchQuery(value: string): string {
  return value.trim().toLowerCase();
}

function partitionRows(
  rows: SelectorModelRow[],
  favoriteKeys: Set<string>,
): { favoriteRows: SelectorModelRow[]; regularRows: SelectorModelRow[] } {
  const favoriteRows: SelectorModelRow[] = [];
  const regularRows: SelectorModelRow[] = [];

  for (const row of rows) {
    if (favoriteKeys.has(row.favoriteKey)) {
      favoriteRows.push(row);
      continue;
    }
    regularRows.push(row);
  }

  return { favoriteRows, regularRows };
}

function sortFavoritesFirst(
  rows: SelectorModelRow[],
  favoriteKeys: Set<string>,
): SelectorModelRow[] {
  const favorites: SelectorModelRow[] = [];
  const rest: SelectorModelRow[] = [];
  for (const row of rows) {
    if (favoriteKeys.has(row.favoriteKey)) {
      favorites.push(row);
    } else {
      rest.push(row);
    }
  }
  return [...favorites, ...rest];
}

function groupRowsByProvider(rows: SelectorModelRow[]): Array<{
  providerId: string;
  providerLabel: string;
  rows: SelectorModelRow[];
}> {
  const grouped = new Map<
    string,
    { providerId: string; providerLabel: string; rows: SelectorModelRow[] }
  >();

  for (const row of rows) {
    const existing = grouped.get(row.provider);
    if (existing) {
      existing.rows.push(row);
      continue;
    }

    grouped.set(row.provider, {
      providerId: row.provider,
      providerLabel: row.providerLabel,
      rows: [row],
    });
  }

  return Array.from(grouped.values());
}

function ModelRow({
  row,
  isSelected,
  isFavorite,
  disabled = false,
  elevated = false,
  onPress,
  onToggleFavorite,
  text,
}: {
  row: SelectorModelRow;
  isSelected: boolean;
  isFavorite: boolean;
  disabled?: boolean;
  elevated?: boolean;
  onPress: () => void;
  onToggleFavorite?: (provider: string, modelId: string) => void;
  text: ReturnType<typeof getAppMessages>["modelSelector"];
}) {
  const { theme } = useUnistyles();
  const ProviderIcon = getProviderIcon(row.provider);

  const handleToggleFavorite = useCallback(
    (event: GestureResponderEvent) => {
      event.stopPropagation();
      onToggleFavorite?.(row.provider, row.modelId);
    },
    [onToggleFavorite, row.modelId, row.provider],
  );

  const showDescription = row.description && PROVIDERS_WITH_MODEL_DESCRIPTIONS.has(row.provider);

  return (
    <ComboboxItem
      label={row.modelLabel}
      description={showDescription ? row.description : undefined}
      selected={isSelected}
      disabled={disabled}
      elevated={elevated}
      onPress={onPress}
      leadingSlot={<ProviderIcon size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />}
      trailingSlot={
        onToggleFavorite && !disabled ? (
          <Pressable
            onPress={handleToggleFavorite}
            hitSlop={8}
            style={({ pressed, hovered }) => [
              styles.favoriteButton,
              hovered && styles.favoriteButtonHovered,
              pressed && styles.favoriteButtonPressed,
            ]}
            accessibilityRole="button"
            accessibilityLabel={isFavorite ? text.unfavoriteModel : text.favoriteModel}
            testID={`favorite-model-${row.provider}-${row.modelId}`}
          >
            {({ hovered }) => (
              <Star
                size={16}
                color={
                  isFavorite
                    ? theme.colors.palette.amber[500]
                    : hovered
                      ? theme.colors.foregroundMuted
                      : theme.colors.border
                }
                fill={isFavorite ? theme.colors.palette.amber[500] : "transparent"}
              />
            )}
          </Pressable>
        ) : null
      }
    />
  );
}

function FavoritesSection({
  favoriteRows,
  selectedProvider,
  selectedModel,
  favoriteKeys,
  onSelect,
  canSelectProvider,
  onToggleFavorite,
  text,
}: {
  favoriteRows: SelectorModelRow[];
  selectedProvider: string;
  selectedModel: string;
  favoriteKeys: Set<string>;
  onSelect: (provider: string, modelId: string) => void;
  canSelectProvider: (provider: string) => boolean;
  onToggleFavorite?: (provider: string, modelId: string) => void;
  text: ReturnType<typeof getAppMessages>["modelSelector"];
}) {
  const { theme } = useUnistyles();

  if (favoriteRows.length === 0) {
    return null;
  }

  return (
    <View style={styles.favoritesContainer}>
      <View style={styles.sectionHeading}>
        <Text style={styles.sectionHeadingText}>{text.favorites}</Text>
      </View>
      {favoriteRows.map((row) => (
        <ModelRow
          key={row.favoriteKey}
          row={row}
          isSelected={row.provider === selectedProvider && row.modelId === selectedModel}
          isFavorite={favoriteKeys.has(row.favoriteKey)}
          disabled={!canSelectProvider(row.provider)}
          elevated
          onPress={() => onSelect(row.provider, row.modelId)}
          onToggleFavorite={onToggleFavorite}
          text={text}
        />
      ))}
    </View>
  );
}

function GroupedProviderRows({
  providerDefinitions,
  groupedRows,
  selectedProvider,
  selectedModel,
  favoriteKeys,
  onSelect,
  cloudGroups,
  canSelectProvider,
  onToggleFavorite,
  onDrillDown,
  onDrillDownCloudGroup,
  viewKind,
  text,
}: {
  providerDefinitions: AgentProviderDefinition[];
  groupedRows: Array<{
    providerId: string;
    providerLabel: string;
    rows: SelectorModelRow[];
  }>;
  selectedProvider: string;
  selectedModel: string;
  favoriteKeys: Set<string>;
  onSelect: (provider: string, modelId: string) => void;
  cloudGroups: SelectorCloudGroup[];
  canSelectProvider: (provider: string) => boolean;
  onToggleFavorite?: (provider: string, modelId: string) => void;
  onDrillDown: (providerId: string, providerLabel: string) => void;
  onDrillDownCloudGroup: (
    providerId: string,
    providerLabel: string,
    groupId: number,
    groupLabel: string,
  ) => void;
  viewKind: SelectorView["kind"];
  text: ReturnType<typeof getAppMessages>["modelSelector"];
}) {
  const { theme } = useUnistyles();

  return (
    <View>
      {groupedRows.map((group, index) => {
        const providerDefinition = providerDefinitions.find(
          (definition) => definition.id === group.providerId,
        );
        const ProvIcon = getProviderIcon(group.providerId);
        const isInline = viewKind === "provider";
        const providerCloudGroups = cloudGroupsForProvider(cloudGroups, group.providerId);
        const otherRows = buildOtherAvailableModelRows(group.rows, providerCloudGroups);
        const providerCanBeSelected = canSelectProvider(group.providerId);

        return (
          <View key={group.providerId}>
            {index > 0 ? <View style={styles.separator} /> : null}
            {isInline && providerCloudGroups.length > 0 ? (
              <>
                <View style={styles.sectionHeading}>
                  <Text style={styles.sectionHeadingText}>{text.cloudGroups}</Text>
                </View>
                {providerCloudGroups.map((cloudGroup) => (
                  <Pressable
                    key={`${cloudGroup.provider}:${cloudGroup.groupId}`}
                    disabled={!providerCanBeSelected}
                    onPress={() =>
                      onDrillDownCloudGroup(
                        group.providerId,
                        group.providerLabel,
                        cloudGroup.groupId,
                        cloudGroup.groupLabel,
                      )
                    }
                    style={({ pressed, hovered }) => [
                      styles.drillDownRow,
                      hovered && styles.drillDownRowHovered,
                      pressed && styles.drillDownRowPressed,
                      !providerCanBeSelected && styles.drillDownRowDisabled,
                    ]}
                  >
                    <Cloud size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
                    <View style={styles.drillDownMain}>
                      <Text style={styles.drillDownText}>{cloudGroup.groupLabel}</Text>
                      {cloudGroup.description ? (
                        <Text style={styles.drillDownDescription} numberOfLines={1}>
                          {cloudGroup.description}
                        </Text>
                      ) : null}
                    </View>
                    <View style={styles.drillDownTrailing}>
                      <Text style={styles.drillDownCount}>
                        {text.modelCount(cloudGroup.models.length)}
                      </Text>
                      <ChevronRight size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
                    </View>
                  </Pressable>
                ))}
                {otherRows.length > 0 ? (
                  <>
                    <View style={styles.sectionHeading}>
                      <Text style={styles.sectionHeadingText}>{text.otherAvailableModels}</Text>
                    </View>
                    {sortFavoritesFirst(otherRows, favoriteKeys).map((row) => (
                      <ModelRow
                        key={row.favoriteKey}
                        row={row}
                        isSelected={
                          row.provider === selectedProvider && row.modelId === selectedModel
                        }
                        isFavorite={favoriteKeys.has(row.favoriteKey)}
                        disabled={!canSelectProvider(row.provider)}
                        onPress={() => onSelect(row.provider, row.modelId)}
                        onToggleFavorite={onToggleFavorite}
                        text={text}
                      />
                    ))}
                  </>
                ) : null}
              </>
            ) : isInline ? (
              <>
                {sortFavoritesFirst(group.rows, favoriteKeys).map((row) => (
                  <ModelRow
                    key={row.favoriteKey}
                    row={row}
                    isSelected={row.provider === selectedProvider && row.modelId === selectedModel}
                    isFavorite={favoriteKeys.has(row.favoriteKey)}
                    disabled={!canSelectProvider(row.provider)}
                    onPress={() => onSelect(row.provider, row.modelId)}
                    onToggleFavorite={onToggleFavorite}
                    text={text}
                  />
                ))}
              </>
            ) : (
              <Pressable
                onPress={() => onDrillDown(group.providerId, group.providerLabel)}
                style={({ pressed, hovered }) => [
                  styles.drillDownRow,
                  hovered && styles.drillDownRowHovered,
                  pressed && styles.drillDownRowPressed,
                ]}
              >
                <ProvIcon size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
                <Text style={styles.drillDownText}>{group.providerLabel}</Text>
                <View style={styles.drillDownTrailing}>
                  <Text style={styles.drillDownCount}>{text.modelCount(group.rows.length)}</Text>
                  <ChevronRight size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
                </View>
              </Pressable>
            )}
          </View>
        );
      })}
    </View>
  );
}

function ProviderSearchInput({
  value,
  onChangeText,
  autoFocus = false,
  text,
}: {
  value: string;
  onChangeText: (text: string) => void;
  autoFocus?: boolean;
  text: ReturnType<typeof getAppMessages>["modelSelector"];
}) {
  const { theme } = useUnistyles();
  const inputRef = useRef<TextInput>(null);
  const isMobile = useIsCompactFormFactor();
  const InputComponent = isMobile ? BottomSheetTextInput : TextInput;
  const webOutlineStyle = platformIsWeb ? ({ outlineStyle: "none" } as any) : null;

  useEffect(() => {
    if (autoFocus && platformIsWeb && inputRef.current) {
      const timer = setTimeout(() => {
        inputRef.current?.focus();
      }, 50);
      return () => clearTimeout(timer);
    }
  }, [autoFocus]);

  return (
    <View style={styles.providerSearchContainer}>
      <Search size={theme.iconSize.md} color={theme.colors.foregroundMuted} />
      <InputComponent
        ref={inputRef as any}
        style={[styles.providerSearchInput, webOutlineStyle]}
        placeholder={text.searchPlaceholder}
        placeholderTextColor={theme.colors.foregroundMuted}
        value={value}
        onChangeText={onChangeText}
        autoCapitalize="none"
        autoCorrect={false}
      />
    </View>
  );
}

function SelectorContent({
  view,
  providerDefinitions,
  allProviderModels,
  selectedProvider,
  selectedModel,
  searchQuery,
  onSearchChange,
  favoriteKeys,
  onSelect,
  cloudGroups,
  onSelectCloudModel,
  canSelectProvider,
  onToggleFavorite,
  onDrillDown,
  onDrillDownCloudGroup,
}: SelectorContentProps) {
  const { theme } = useUnistyles();
  const locale = useAppLocale();
  const text = useMemo(() => getAppMessages(locale).modelSelector, [locale]);
  const allRows = useMemo(
    () => buildModelRows(providerDefinitions, allProviderModels),
    [allProviderModels, providerDefinitions],
  );

  const scopedRows = useMemo(() => {
    if (view.kind === "provider") {
      return allRows.filter((row) => row.provider === view.providerId);
    }
    if (view.kind === "cloudGroup") {
      const group = cloudGroups.find(
        (entry) => entry.provider === view.providerId && entry.groupId === view.groupId,
      );
      return group
        ? buildCloudGroupModelRows({
            providerLabel: view.providerLabel,
            group,
          })
        : [];
    }
    return allRows;
  }, [allRows, cloudGroups, view]);

  const normalizedQuery = useMemo(() => normalizeSearchQuery(searchQuery), [searchQuery]);

  const visibleRows = useMemo(
    () => scopedRows.filter((row) => matchesSearch(row, normalizedQuery)),
    [normalizedQuery, scopedRows],
  );

  const { favoriteRows, regularRows } = useMemo(
    () => partitionRows(visibleRows, favoriteKeys),
    [favoriteKeys, visibleRows],
  );

  // Group ALL visible rows by provider — favorites are a cross-cutting view,
  // not a partition. A model being favorited doesn't remove it from its provider.
  const allGroupedRows = useMemo(() => groupRowsByProvider(visibleRows), [visibleRows]);

  // When searching at Level 1, filter grouped rows to only providers whose name or models match
  const filteredGroupedRows = useMemo(() => {
    if (view.kind === "provider" || view.kind === "cloudGroup" || !normalizedQuery) {
      return allGroupedRows;
    }
    return allGroupedRows.filter(
      (group) =>
        group.providerLabel.toLowerCase().includes(normalizedQuery) || group.rows.length > 0,
    );
  }, [allGroupedRows, normalizedQuery, view.kind]);

  const activeCloudGroup =
    view.kind === "cloudGroup"
      ? (cloudGroups.find(
          (entry) => entry.provider === view.providerId && entry.groupId === view.groupId,
        ) ?? null)
      : null;
  const hasResults = favoriteRows.length > 0 || filteredGroupedRows.length > 0;

  if (view.kind === "cloudGroup") {
    return (
      <View>
        {visibleRows.length > 0 && activeCloudGroup ? (
          <>
            <View style={styles.sectionHeading}>
              <Text style={styles.sectionHeadingText}>{activeCloudGroup.groupLabel}</Text>
            </View>
            {sortFavoritesFirst(visibleRows, favoriteKeys).map((row) => (
              <ModelRow
                key={row.favoriteKey}
                row={row}
                isSelected={row.provider === selectedProvider && row.modelId === selectedModel}
                isFavorite={favoriteKeys.has(row.favoriteKey)}
                disabled={!canSelectProvider(row.provider)}
                onPress={() => {
                  if (onSelectCloudModel) {
                    onSelectCloudModel(row.provider, row.modelId, activeCloudGroup);
                    return;
                  }
                  onSelect(row.provider, row.modelId);
                }}
                onToggleFavorite={onToggleFavorite}
                text={text}
              />
            ))}
          </>
        ) : null}

        {visibleRows.length === 0 ? (
          <View style={styles.emptyState}>
            <Search size={theme.iconSize.md} color={theme.colors.foregroundMuted} />
            <Text style={styles.emptyStateText}>{text.noSearchMatches}</Text>
          </View>
        ) : null}
      </View>
    );
  }

  return (
    <View>
      {view.kind === "all" ? (
        <FavoritesSection
          favoriteRows={favoriteRows}
          selectedProvider={selectedProvider}
          selectedModel={selectedModel}
          favoriteKeys={favoriteKeys}
          onSelect={onSelect}
          canSelectProvider={canSelectProvider}
          onToggleFavorite={onToggleFavorite}
          text={text}
        />
      ) : null}

      {filteredGroupedRows.length > 0 ? (
        <GroupedProviderRows
          providerDefinitions={providerDefinitions}
          groupedRows={filteredGroupedRows}
          selectedProvider={selectedProvider}
          selectedModel={selectedModel}
          favoriteKeys={favoriteKeys}
          onSelect={onSelect}
          cloudGroups={cloudGroups}
          canSelectProvider={canSelectProvider}
          onToggleFavorite={onToggleFavorite}
          onDrillDown={onDrillDown}
          onDrillDownCloudGroup={onDrillDownCloudGroup}
          viewKind={view.kind}
          text={text}
        />
      ) : null}

      {!hasResults ? (
        <View style={styles.emptyState}>
          <Search size={theme.iconSize.md} color={theme.colors.foregroundMuted} />
          <Text style={styles.emptyStateText}>{text.noSearchMatches}</Text>
        </View>
      ) : null}
    </View>
  );
}

function ProviderBackButton({
  providerId,
  providerLabel,
  onBack,
}: {
  providerId: string;
  providerLabel: string;
  onBack?: () => void;
}) {
  const { theme } = useUnistyles();
  const ProviderIcon = getProviderIcon(providerId);

  if (!onBack) {
    return null;
  }

  return (
    <Pressable
      onPress={onBack}
      style={({ pressed, hovered }) => [
        styles.backButton,
        hovered && styles.backButtonHovered,
        pressed && styles.backButtonPressed,
      ]}
    >
      <ArrowLeft size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
      <ProviderIcon size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
      <Text style={styles.backButtonText}>{providerLabel}</Text>
    </Pressable>
  );
}

export function CombinedModelSelector({
  providerDefinitions,
  allProviderModels,
  selectedProvider,
  selectedModel,
  onSelect,
  cloudGroups = [],
  onSelectCloudModel,
  isLoading,
  canSelectProvider = () => true,
  favoriteKeys = new Set<string>(),
  onToggleFavorite,
  renderTrigger,
  onOpen,
  onClose,
  disabled = false,
}: CombinedModelSelectorProps) {
  const { theme } = useUnistyles();
  const locale = useAppLocale();
  const text = useMemo(() => getAppMessages(locale).modelSelector, [locale]);
  const anchorRef = useRef<View>(null);
  const [isOpen, setIsOpen] = useState(false);
  const [isContentReady, setIsContentReady] = useState(platformIsWeb);
  const [view, setView] = useState<SelectorView>({ kind: "all" });
  const [searchQuery, setSearchQuery] = useState("");

  // Single-provider mode: only one provider with models → skip Level 1 entirely
  const singleProviderView = useMemo<SelectorView | null>(() => {
    const providers = Array.from(allProviderModels.keys());
    if (providers.length !== 1) return null;
    const providerId = providers[0]!;
    const label = resolveProviderLabel(providerDefinitions, providerId);
    return { kind: "provider", providerId, providerLabel: label };
  }, [allProviderModels, providerDefinitions]);

  const computeInitialView = useCallback((): SelectorView => {
    if (singleProviderView) return singleProviderView;

    const selectedFavoriteKey = `${selectedProvider}:${selectedModel}`;
    if (selectedProvider && selectedModel && !favoriteKeys.has(selectedFavoriteKey)) {
      const label = resolveProviderLabel(providerDefinitions, selectedProvider);
      return {
        kind: "provider",
        providerId: selectedProvider,
        providerLabel: label,
      };
    }

    return { kind: "all" };
  }, [singleProviderView, selectedProvider, selectedModel, favoriteKeys, providerDefinitions]);

  const handleOpenChange = useCallback(
    (open: boolean) => {
      setIsOpen(open);
      setView(computeInitialView());
      if (open) {
        onOpen?.();
      } else {
        setSearchQuery("");
        onClose?.();
      }
    },
    [onOpen, onClose, computeInitialView],
  );

  const handleSelect = useCallback(
    (provider: string, modelId: string) => {
      onSelect(provider as AgentProvider, modelId);
      setIsOpen(false);
      setSearchQuery("");
    },
    [onSelect],
  );

  const handleSelectCloudModel = useCallback(
    (provider: string, modelId: string, group: SelectorCloudGroup) => {
      if (onSelectCloudModel) {
        onSelectCloudModel(provider as AgentProvider, modelId, group);
      } else {
        onSelect(provider as AgentProvider, modelId);
      }
      setIsOpen(false);
      setSearchQuery("");
    },
    [onSelect, onSelectCloudModel],
  );

  const hasSelectedProvider = selectedProvider.trim().length > 0;
  const ProviderIcon = hasSelectedProvider ? getProviderIcon(selectedProvider) : null;

  const selectedModelLabel = useMemo(() => {
    if (!selectedModel) {
      if (!hasSelectedProvider) {
        return text.selectModel;
      }
      return isLoading ? text.loading : text.selectModel;
    }
    const models = allProviderModels.get(selectedProvider);
    if (!models) {
      const cloudModel = cloudGroups
        .filter((group) => group.provider === selectedProvider)
        .flatMap((group) => group.models)
        .find((entry) => entry.id === selectedModel);
      if (cloudModel) {
        return cloudModel.label;
      }
      return isLoading ? text.loading : text.selectModel;
    }
    const model = models.find((entry) => entry.id === selectedModel);
    if (model) {
      return model.label;
    }
    const cloudModel = cloudGroups
      .filter((group) => group.provider === selectedProvider)
      .flatMap((group) => group.models)
      .find((entry) => entry.id === selectedModel);
    return cloudModel?.label ?? resolveDefaultModelLabel(models, text.selectModel);
  }, [
    allProviderModels,
    cloudGroups,
    hasSelectedProvider,
    isLoading,
    selectedModel,
    selectedProvider,
    text,
  ]);

  const desktopFixedHeight = useMemo(() => {
    if (view.kind === "cloudGroup") {
      const group = cloudGroups.find(
        (entry) => entry.provider === view.providerId && entry.groupId === view.groupId,
      );
      const modelCount = group?.models.length ?? 0;
      return Math.min(80 + modelCount * 40, 400);
    }
    if (view.kind !== "provider") {
      return undefined;
    }
    const models = allProviderModels.get(view.providerId);
    const modelCount = models?.length ?? 0;
    return Math.min(80 + modelCount * 40, 400);
  }, [allProviderModels, cloudGroups, view]);

  const triggerLabel = useMemo(() => {
    if (selectedModelLabel === text.loading || selectedModelLabel === text.selectModel) {
      return selectedModelLabel;
    }

    return buildSelectedTriggerLabel(selectedModelLabel);
  }, [selectedModelLabel, text.loading, text.selectModel]);

  useEffect(() => {
    if (platformIsWeb) {
      return;
    }

    if (!isOpen) {
      setIsContentReady(false);
      return;
    }

    const frame = requestAnimationFrame(() => {
      setIsContentReady(true);
    });

    return () => cancelAnimationFrame(frame);
  }, [isOpen, platformIsWeb]);

  return (
    <>
      <Pressable
        ref={anchorRef}
        collapsable={false}
        disabled={disabled}
        onPress={() => handleOpenChange(!isOpen)}
        style={({ pressed, hovered }) => [
          styles.trigger,
          hovered && styles.triggerHovered,
          (pressed || isOpen) && styles.triggerPressed,
          disabled && styles.triggerDisabled,
          renderTrigger ? styles.customTriggerWrapper : null,
        ]}
        accessibilityRole="button"
        accessibilityLabel={`${text.selectModel} (${selectedModelLabel})`}
        testID="combined-model-selector"
      >
        {renderTrigger ? (
          renderTrigger({
            selectedModelLabel: triggerLabel,
            onPress: () => handleOpenChange(!isOpen),
            disabled,
            isOpen,
          })
        ) : (
          <>
            {ProviderIcon ? (
              <ProviderIcon size={theme.iconSize.md} color={theme.colors.foregroundMuted} />
            ) : null}
            <Text style={styles.triggerText} numberOfLines={1} ellipsizeMode="tail">
              {triggerLabel}
            </Text>
            <ChevronDown size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
          </>
        )}
      </Pressable>
      <Combobox
        options={[]}
        value=""
        onSelect={() => {}}
        open={isOpen}
        onOpenChange={handleOpenChange}
        anchorRef={anchorRef}
        desktopPlacement="top-start"
        desktopMinWidth={360}
        desktopFixedHeight={desktopFixedHeight}
        title={text.title}
        stickyHeader={
          view.kind !== "all" ? (
            <View style={styles.level2Header}>
              {view.kind === "cloudGroup" || !singleProviderView ? (
                <ProviderBackButton
                  providerId={view.providerId}
                  providerLabel={view.kind === "cloudGroup" ? view.groupLabel : view.providerLabel}
                  onBack={() => {
                    if (view.kind === "cloudGroup") {
                      setView({
                        kind: "provider",
                        providerId: view.providerId,
                        providerLabel: view.providerLabel,
                      });
                    } else {
                      setView({ kind: "all" });
                    }
                    setSearchQuery("");
                  }}
                />
              ) : null}
              <ProviderSearchInput
                value={searchQuery}
                onChangeText={setSearchQuery}
                autoFocus={platformIsWeb}
                text={text}
              />
            </View>
          ) : undefined
        }
      >
        {isContentReady ? (
          <SelectorContent
            view={view}
            providerDefinitions={providerDefinitions}
            allProviderModels={allProviderModels}
            selectedProvider={selectedProvider}
            selectedModel={selectedModel}
            searchQuery={searchQuery}
            onSearchChange={setSearchQuery}
            favoriteKeys={favoriteKeys}
            onSelect={handleSelect}
            cloudGroups={cloudGroups}
            onSelectCloudModel={handleSelectCloudModel}
            canSelectProvider={canSelectProvider}
            onToggleFavorite={onToggleFavorite}
            onDrillDown={(providerId, providerLabel) => {
              setView({ kind: "provider", providerId, providerLabel });
            }}
            onDrillDownCloudGroup={(providerId, providerLabel, groupId, groupLabel) => {
              setView({
                kind: "cloudGroup",
                providerId,
                providerLabel,
                groupId,
                groupLabel,
              });
              setSearchQuery("");
            }}
          />
        ) : (
          <View style={styles.sheetLoadingState}>
            <ActivityIndicator size="small" color={theme.colors.foregroundMuted} />
            <Text style={styles.sheetLoadingText}>{text.loadingSelector}</Text>
          </View>
        )}
      </Combobox>
    </>
  );
}

const styles = StyleSheet.create((theme) => ({
  trigger: {
    height: 28,
    minWidth: 0,
    flexShrink: 1,
    flexDirection: "row",
    alignItems: "center",
    backgroundColor: "transparent",
    gap: theme.spacing[1],
    paddingHorizontal: theme.spacing[2],
    borderRadius: theme.borderRadius["2xl"],
  },
  triggerHovered: {
    backgroundColor: theme.colors.surface2,
  },
  triggerPressed: {
    backgroundColor: theme.colors.surface0,
  },
  triggerDisabled: {
    opacity: 0.5,
  },
  triggerText: {
    minWidth: 0,
    flexShrink: 1,
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.normal,
  },
  customTriggerWrapper: {
    paddingHorizontal: 0,
    paddingVertical: 0,
    height: "auto",
  },
  favoritesContainer: {
    backgroundColor: theme.colors.surface1,
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
  },
  separator: {
    height: 1,
    backgroundColor: theme.colors.border,
  },
  sectionHeading: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    paddingHorizontal: theme.spacing[3],
    paddingTop: theme.spacing[2],
    paddingBottom: theme.spacing[1],
    ...(IS_WEB ? {} : { marginHorizontal: theme.spacing[1] }),
  },
  sectionHeadingText: {
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.normal,
    color: theme.colors.foregroundMuted,
  },
  drillDownRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    paddingHorizontal: theme.spacing[3],
    paddingVertical: theme.spacing[2],
    minHeight: 36,
    ...(IS_WEB ? {} : { marginHorizontal: theme.spacing[1] }),
  },
  drillDownRowHovered: {
    backgroundColor: theme.colors.surface1,
  },
  drillDownRowPressed: {
    backgroundColor: theme.colors.surface2,
  },
  drillDownRowDisabled: {
    opacity: 0.5,
  },
  drillDownMain: {
    flex: 1,
    minWidth: 0,
  },
  drillDownText: {
    fontSize: theme.fontSize.sm,
    color: theme.colors.foreground,
  },
  drillDownDescription: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
  },
  drillDownTrailing: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[1],
  },
  drillDownCount: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
  },
  level2Header: {},
  backButton: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    paddingHorizontal: theme.spacing[3],
    paddingVertical: theme.spacing[2],
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
    ...(IS_WEB ? {} : { marginHorizontal: theme.spacing[1] }),
  },
  backButtonHovered: {
    backgroundColor: theme.colors.surface2,
  },
  backButtonPressed: {
    backgroundColor: theme.colors.surface2,
  },
  backButtonText: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
  },
  emptyState: {
    paddingVertical: theme.spacing[4],
    alignItems: "center",
    gap: theme.spacing[2],
  },
  emptyStateText: {
    fontSize: theme.fontSize.sm,
    color: theme.colors.foregroundMuted,
  },
  favoriteButton: {
    width: 24,
    height: 24,
    borderRadius: theme.borderRadius.full,
    alignItems: "center",
    justifyContent: "center",
  },
  favoriteButtonHovered: {
    backgroundColor: theme.colors.surface2,
  },
  favoriteButtonPressed: {
    backgroundColor: theme.colors.surface1,
  },
  sheetLoadingState: {
    minHeight: 160,
    justifyContent: "center",
    alignItems: "center",
    gap: theme.spacing[2],
  },
  sheetLoadingText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
  },
  providerSearchContainer: {
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: theme.spacing[3],
    gap: theme.spacing[2],
    ...(IS_WEB ? {} : { marginHorizontal: theme.spacing[1] }),
  },
  providerSearchInput: {
    flex: 1,
    paddingVertical: theme.spacing[3],
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
  },
}));

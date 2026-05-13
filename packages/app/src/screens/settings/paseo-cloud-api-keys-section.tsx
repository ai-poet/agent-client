import React from "react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Alert, Pressable, Text, TextInput, View } from "react-native";
import { useUnistyles } from "react-native-unistyles";
import { SegmentedControl } from "@/components/ui/segmented-control";
import { ComboSelect } from "@/components/agent-form/agent-form-dropdowns";
import { AdaptiveModalSheet, AdaptiveTextInput } from "@/components/adaptive-modal-sheet";
import { invokeDesktopCommand } from "@/desktop/electron/invoke";
import {
  useCreateSub2APIKeyMutation,
  useDeleteSub2APIKeyMutation,
  useSub2APIAvailableGroups,
  useSub2APIKeys,
  useUpdateSub2APIKeyMutation,
} from "@/hooks/use-sub2api-api";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";
import { CLOUD_NAME, DESKTOP_DEFAULT_KEY_NAME } from "@/config/branding";
import type { Sub2APIKey } from "@/lib/sub2api-client";
import { SettingsSection } from "@/screens/settings/settings-section";
import { useDesktopProvidersStore } from "@/screens/settings/desktop-providers-context";
import {
  formatUsd,
  getErrorMessage,
  maskApiKey,
  normalizeFilter,
} from "@/screens/settings/managed-provider-settings-shared";
import { managedProviderSettingsStyles as styles } from "@/screens/settings/managed-provider-settings-styles";
import {
  getManagedCloudMetaForScope,
  resolveManagedCloudRouteForGroup,
  resolveManagedCloudRouteForKey,
  type ManagedCloudDesktopScope,
} from "@/screens/settings/managed-cloud-scope";
import { settingsStyles } from "@/styles/settings";
import { isValidSub2APIEndpoint } from "./sub2api-auth-bridge";

type PaseoCloudApiKeysSectionProps = {
  authEndpoint?: string | null;
  serviceEndpoint: string;
};

const SCOPE_OPTIONS: Array<{
  value: ManagedCloudDesktopScope;
  label: string;
  testID: string;
}> = [
  {
    value: "claude",
    label: "Claude Code",
    testID: "sub2api-api-keys-tab-claude",
  },
  { value: "codex", label: "Codex", testID: "sub2api-api-keys-tab-codex" },
];

function clampProgress(value: number): number {
  if (!Number.isFinite(value)) {
    return 0;
  }
  return Math.max(0, Math.min(value, 1));
}

function getUsageTone(value: number): "base" | "warning" | "danger" {
  if (value >= 1) {
    return "danger";
  }
  if (value >= 0.8) {
    return "warning";
  }
  return "base";
}

export function PaseoCloudApiKeysSection({
  authEndpoint,
  serviceEndpoint,
}: PaseoCloudApiKeysSectionProps) {
  const { theme } = useUnistyles();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).paseoCloudApiKeys, [locale]);
  const { loadProviders, activeClaudeProvider, activeCodexProvider } = useDesktopProvidersStore();
  const [activeScope, setActiveScope] = useState<ManagedCloudDesktopScope>("claude");
  const [keyFilter, setKeyFilter] = useState("");
  const [groupFilterId, setGroupFilterId] = useState<number | null>(null);
  const [createModalVisible, setCreateModalVisible] = useState(false);
  const [editingKey, setEditingKey] = useState<Sub2APIKey | null>(null);
  const [draftName, setDraftName] = useState(DESKTOP_DEFAULT_KEY_NAME);
  const [draftGroupId, setDraftGroupId] = useState<number | null>(null);
  const [switchingKeyId, setSwitchingKeyId] = useState<number | null>(null);

  const keysQuery = useSub2APIKeys(1, 200);
  const groupsQuery = useSub2APIAvailableGroups();
  const createKeyMutation = useCreateSub2APIKeyMutation();
  const updateKeyMutation = useUpdateSub2APIKeyMutation();
  const deleteKeyMutation = useDeleteSub2APIKeyMutation();

  const keys = useMemo(() => keysQuery.data?.items ?? [], [keysQuery.data?.items]);
  const groups = useMemo(() => groupsQuery.data ?? [], [groupsQuery.data]);
  const scopeMeta = getManagedCloudMetaForScope(activeScope);
  const activeScopeApiKey =
    activeScope === "claude"
      ? (activeClaudeProvider?.apiKey?.trim() ?? null)
      : (activeCodexProvider?.apiKey?.trim() ?? null);

  const groupsByScope = useMemo(
    () => ({
      claude: groups.filter((group) => {
        const route = resolveManagedCloudRouteForGroup(group);
        return route.ok && route.scope === "claude";
      }),
      codex: groups.filter((group) => {
        const route = resolveManagedCloudRouteForGroup(group);
        return route.ok && route.scope === "codex";
      }),
    }),
    [groups],
  );

  const scopedGroups = groupsByScope[activeScope];
  const alternateScope: ManagedCloudDesktopScope = activeScope === "claude" ? "codex" : "claude";
  const alternateGroups = groupsByScope[alternateScope];
  const groupFilterOptions = useMemo(
    () => [
      {
        id: "all",
        label: text.allGroups,
        description: text.showAllKeys(scopeMeta.cliLabel),
      },
      ...scopedGroups.map((group) => ({
        id: String(group.id),
        label: group.name,
        description: `${group.platform} · ${group.rate_multiplier}x`,
      })),
    ],
    [scopeMeta.cliLabel, scopedGroups, text],
  );

  const keyRoutes = useMemo(
    () => new Map(keys.map((key) => [key.id, resolveManagedCloudRouteForKey(key, groups)])),
    [groups, keys],
  );

  const scopedKeys = useMemo(
    () =>
      keys.filter((key) => {
        const route = keyRoutes.get(key.id);
        return route?.ok && route.scope === activeScope;
      }),
    [activeScope, keyRoutes, keys],
  );

  const filteredKeys = useMemo(() => {
    const query = normalizeFilter(keyFilter);
    return scopedKeys.filter((key) => {
      if (groupFilterId !== null && key.group_id !== groupFilterId) {
        return false;
      }
      if (!query) {
        return true;
      }
      return (
        key.name.toLowerCase().includes(query) ||
        (key.group?.name?.toLowerCase().includes(query) ?? false) ||
        maskApiKey(key.key).toLowerCase().includes(query)
      );
    });
  }, [groupFilterId, keyFilter, scopedKeys]);

  useEffect(() => {
    if (groupFilterId !== null && !scopedGroups.some((group) => group.id === groupFilterId)) {
      setGroupFilterId(null);
    }
  }, [groupFilterId, scopedGroups]);

  useEffect(() => {
    if (!createModalVisible && !editingKey) {
      return;
    }
    if (draftGroupId !== null && scopedGroups.some((group) => group.id === draftGroupId)) {
      return;
    }
    setDraftGroupId(scopedGroups[0]?.id ?? null);
  }, [createModalVisible, draftGroupId, editingKey, scopedGroups]);

  const setupDefaultProviderWithKey = useCallback(
    async (apiKey: string, scope: ManagedCloudDesktopScope, name?: string) => {
      const targetEndpoint = authEndpoint ?? serviceEndpoint;
      if (!isValidSub2APIEndpoint(targetEndpoint)) {
        throw new Error(text.serviceEndpointInvalid);
      }

      await invokeDesktopCommand("setup_default_provider", {
        endpoint: targetEndpoint,
        apiKey,
        scope,
        ...(name ? { name } : {}),
      });
      await loadProviders();
    },
    [authEndpoint, loadProviders, serviceEndpoint, text.serviceEndpointInvalid],
  );

  const closeModal = useCallback(() => {
    setCreateModalVisible(false);
    setEditingKey(null);
    setDraftName(DESKTOP_DEFAULT_KEY_NAME);
    setDraftGroupId(scopedGroups[0]?.id ?? null);
  }, [scopedGroups]);

  const openCreateModal = useCallback(() => {
    setEditingKey(null);
    setDraftName(DESKTOP_DEFAULT_KEY_NAME);
    setDraftGroupId(groupFilterId ?? scopedGroups[0]?.id ?? null);
    setCreateModalVisible(true);
  }, [groupFilterId, scopedGroups]);

  const openEditModal = useCallback(
    (key: Sub2APIKey) => {
      setCreateModalVisible(false);
      setEditingKey(key);
      setDraftName(key.name);
      setDraftGroupId(key.group_id ?? scopedGroups[0]?.id ?? null);
    },
    [scopedGroups],
  );

  const handleSaveKey = useCallback(async () => {
    const name = draftName.trim();
    if (!name) {
      Alert.alert(text.missingNameTitle, text.missingNameBody);
      return;
    }
    if (draftGroupId === null) {
      Alert.alert(text.missingGroupTitle, text.missingGroupBody);
      return;
    }

    try {
      if (editingKey) {
        await updateKeyMutation.mutateAsync({
          id: editingKey.id,
          patch: { name, group_id: draftGroupId },
        });
      } else {
        await createKeyMutation.mutateAsync({
          name,
          group_id: draftGroupId,
        });
      }
      closeModal();
    } catch (error) {
      Alert.alert(editingKey ? text.updateFailed : text.createFailed, getErrorMessage(error));
    }
  }, [
    closeModal,
    createKeyMutation,
    draftGroupId,
    draftName,
    editingKey,
    text.createFailed,
    text.missingGroupBody,
    text.missingGroupTitle,
    text.missingNameBody,
    text.missingNameTitle,
    text.updateFailed,
    updateKeyMutation,
  ]);

  const handleUseKey = useCallback(
    async (key: Sub2APIKey) => {
      const resolved = keyRoutes.get(key.id) ?? resolveManagedCloudRouteForKey(key, groups);
      if (!resolved.ok) {
        Alert.alert(text.cannotApplyKey, resolved.reason);
        return;
      }
      if (resolved.scope !== activeScope) {
        const targetMeta = getManagedCloudMetaForScope(resolved.scope);
        setActiveScope(resolved.scope);
        Alert.alert(
          text.movedTitle,
          text.movedKeyMessage(key.name, targetMeta.cliLabel, CLOUD_NAME),
        );
        return;
      }

      setSwitchingKeyId(key.id);
      try {
        await setupDefaultProviderWithKey(key.key, activeScope, key.group?.name ?? key.name);
        Alert.alert(
          text.globalUpdatedTitle,
          text.globalKeyUpdatedMessage(
            CLOUD_NAME,
            key.name,
            scopeMeta.cliLabel,
            scopeMeta.configTarget,
          ),
        );
      } catch (error) {
        Alert.alert(text.switchFailed, getErrorMessage(error));
      } finally {
        setSwitchingKeyId(null);
      }
    },
    [
      activeScope,
      groups,
      keyRoutes,
      scopeMeta.cliLabel,
      scopeMeta.configTarget,
      setupDefaultProviderWithKey,
      text,
    ],
  );

  const handleDeleteKey = useCallback(
    async (keyId: number) => {
      try {
        await deleteKeyMutation.mutateAsync(keyId);
      } catch (error) {
        Alert.alert(text.deleteFailed, getErrorMessage(error));
      }
    },
    [deleteKeyMutation, text.deleteFailed],
  );

  const modalVisible = createModalVisible || editingKey !== null;

  return (
    <>
      <SettingsSection title={text.title}>
        <View style={[settingsStyles.card, styles.cardBody]}>
          <View style={styles.statusRow}>
            <View style={settingsStyles.rowContent}>
              <Text style={styles.formTitle}>{scopeMeta.cliLabel}</Text>
              <Text style={styles.sectionHint}>
                {text.sectionHint(scopeMeta.platform, scopeMeta.cliLabel)}
              </Text>
            </View>
            <Pressable
              onPress={openCreateModal}
              style={({ pressed }) => [
                styles.primaryButton,
                pressed && styles.buttonPressed,
                scopedGroups.length === 0 && styles.disabledButton,
              ]}
              disabled={scopedGroups.length === 0}
              testID="sub2api-open-create-key-modal"
            >
              <Text style={styles.primaryButtonText}>{text.createApiKey}</Text>
            </Pressable>
          </View>

          <View style={styles.tabBar}>
            <SegmentedControl
              options={SCOPE_OPTIONS}
              value={activeScope}
              onValueChange={(value) => setActiveScope(value)}
              size="sm"
              testID="sub2api-api-key-tabs"
            />
          </View>

          <Text style={styles.usageHint}>{text.pageHint}</Text>
          {scopedGroups.length === 0 ? (
            <View style={styles.dashedCard}>
              <Text style={styles.emptyTitle}>{text.noCompatibleGroupsTitle}</Text>
              <Text style={styles.emptyBody}>
                {text.noCompatibleGroupsBody(scopeMeta.platform, scopeMeta.cliLabel, CLOUD_NAME)}
              </Text>
              {alternateGroups.length > 0 ? (
                <Pressable
                  onPress={() => setActiveScope(alternateScope)}
                  style={({ pressed }) => [styles.secondaryButton, pressed && styles.buttonPressed]}
                >
                  <Text style={styles.secondaryButtonText}>
                    {text.viewScopeInstead(getManagedCloudMetaForScope(alternateScope).cliLabel)}
                  </Text>
                </Pressable>
              ) : null}
            </View>
          ) : null}

          <View style={styles.groupPickerBlock}>
            <Text style={styles.fieldLabel}>{text.search}</Text>
            <TextInput
              value={keyFilter}
              onChangeText={setKeyFilter}
              placeholder={text.searchPlaceholder(scopeMeta.platform)}
              placeholderTextColor={theme.colors.foregroundMuted}
              autoCapitalize="none"
              autoCorrect={false}
              style={styles.textInput}
            />
            <View style={styles.groupComboWrap}>
              <ComboSelect
                label={text.filterByGroup}
                title={text.filterByGroupTitle(scopeMeta.cliLabel)}
                value={groupFilterId != null ? String(groupFilterId) : "all"}
                options={groupFilterOptions}
                placeholder={text.allGroups}
                isLoading={groupsQuery.isFetching}
                onSelect={(id) => setGroupFilterId(id === "all" ? null : Number(id))}
                showLabel
                testID={`sub2api-key-filter-group-${activeScope}`}
              />
            </View>
          </View>

          {keysQuery.error ? (
            <View style={styles.errorBlock}>
              <Text style={styles.errorHint}>{getErrorMessage(keysQuery.error)}</Text>
              <Pressable
                onPress={() => void keysQuery.refetch()}
                style={({ pressed }) => [styles.secondaryButton, pressed && styles.buttonPressed]}
              >
                <Text style={styles.secondaryButtonText}>{text.retry}</Text>
              </Pressable>
            </View>
          ) : keysQuery.isLoading ? (
            <Text style={styles.usageHint}>{text.loadingKeys}</Text>
          ) : filteredKeys.length === 0 ? (
            <Text style={styles.usageHint}>{text.noKeysMatch(scopeMeta.cliLabel)}</Text>
          ) : (
            <View style={styles.keyRowList}>
              {filteredKeys.map((key) => {
                const keyRoute =
                  keyRoutes.get(key.id) ?? resolveManagedCloudRouteForKey(key, groups);
                const trimmedKey = key.key.trim();
                const activeForScope = activeScopeApiKey === trimmedKey;
                const applying = switchingKeyId === key.id;
                return (
                  <View key={key.id} style={styles.keyRow}>
                    <View style={settingsStyles.rowContent}>
                      <Text style={settingsStyles.rowTitle}>{key.name}</Text>
                      <Text style={settingsStyles.rowHint}>
                        {maskApiKey(key.key)} · {text.group}:{" "}
                        {key.group?.name ?? key.group_id ?? text.none}
                      </Text>
                      <Text style={settingsStyles.rowHint}>
                        {text.used}: {formatUsd(key.quota_used)}
                      </Text>
                      {key.quota > 0 ? (
                        <View style={styles.usageMeterBlock}>
                          <View style={styles.usageMeterHeader}>
                            <Text style={styles.usageMeterLabel}>{text.quota}</Text>
                            <Text style={styles.usageMeterValue}>
                              {formatUsd(key.quota_used)} / {formatUsd(key.quota)}
                            </Text>
                          </View>
                          <View style={styles.usageMeterTrack}>
                            <View
                              style={[
                                styles.usageMeterFillBase,
                                getUsageTone(clampProgress(key.quota_used / key.quota)) ===
                                  "warning" && styles.usageMeterFillWarning,
                                getUsageTone(clampProgress(key.quota_used / key.quota)) ===
                                  "danger" && styles.usageMeterFillDanger,
                                {
                                  width: `${clampProgress(key.quota_used / key.quota) * 100}%`,
                                },
                              ]}
                            />
                          </View>
                        </View>
                      ) : (
                        <Text style={styles.usageHint}>
                          {text.quota}: {text.unlimited}
                        </Text>
                      )}
                      <View style={styles.usageWindowWrap}>
                        <View style={styles.usageWindowPill}>
                          <Text style={styles.usageWindowPillText}>
                            5h {formatUsd(key.usage_5h)}
                            {key.rate_limit_5h > 0 ? ` / ${formatUsd(key.rate_limit_5h)}` : ""}
                          </Text>
                        </View>
                        <View style={styles.usageWindowPill}>
                          <Text style={styles.usageWindowPillText}>
                            1d {formatUsd(key.usage_1d)}
                            {key.rate_limit_1d > 0 ? ` / ${formatUsd(key.rate_limit_1d)}` : ""}
                          </Text>
                        </View>
                        <View style={styles.usageWindowPill}>
                          <Text style={styles.usageWindowPillText}>
                            7d {formatUsd(key.usage_7d)}
                            {key.rate_limit_7d > 0 ? ` / ${formatUsd(key.rate_limit_7d)}` : ""}
                          </Text>
                        </View>
                      </View>
                      {keyRoute.ok ? (
                        <Text style={styles.usageHint}>
                          {text.advancedAction}{" "}
                          <Text style={styles.sectionHintEm}>{scopeMeta.configTarget}</Text>
                        </Text>
                      ) : (
                        <Text style={styles.errorHint}>{keyRoute.reason}</Text>
                      )}
                      {activeForScope ? (
                        <Text style={styles.activeProviderText}>
                          {text.activeForCli(scopeMeta.cliLabel)}
                        </Text>
                      ) : key.group?.status === "inactive" ? (
                        <Text style={styles.errorHint}>{text.groupInactive}</Text>
                      ) : null}
                    </View>
                    <View style={[styles.keyActions, { maxWidth: 220 }]}>
                      <Pressable
                        onPress={() => openEditModal(key)}
                        style={({ pressed }) => [
                          styles.secondaryButton,
                          pressed && styles.buttonPressed,
                          updateKeyMutation.isPending && styles.disabledButton,
                        ]}
                        disabled={updateKeyMutation.isPending}
                      >
                        <Text style={styles.secondaryButtonText}>{text.edit}</Text>
                      </Pressable>
                      <Pressable
                        onPress={() => void handleUseKey(key)}
                        style={({ pressed }) => [
                          activeForScope ? styles.useKeyButtonUsed : styles.primaryButton,
                          pressed && !activeForScope && styles.buttonPressed,
                          (applying || switchingKeyId !== null || !keyRoute.ok) &&
                            styles.disabledButton,
                        ]}
                        disabled={
                          activeForScope || applying || switchingKeyId !== null || !keyRoute.ok
                        }
                        testID={`sub2api-use-key-${activeScope}-${key.id}`}
                      >
                        <Text
                          style={
                            activeForScope ? styles.useKeyButtonUsedText : styles.primaryButtonText
                          }
                        >
                          {applying
                            ? text.applying
                            : activeForScope
                              ? text.activeCta(scopeMeta.cliLabel)
                              : text.setGlobalDefault}
                        </Text>
                      </Pressable>
                      <Pressable
                        onPress={() => void handleDeleteKey(key.id)}
                        style={({ pressed }) => [
                          styles.removeButton,
                          pressed && styles.buttonPressed,
                        ]}
                      >
                        <Text style={styles.removeButtonText}>{text.delete}</Text>
                      </Pressable>
                    </View>
                  </View>
                );
              })}
            </View>
          )}
        </View>
      </SettingsSection>

      <AdaptiveModalSheet
        title={text.modalTitle(Boolean(editingKey))}
        visible={modalVisible}
        onClose={closeModal}
        desktopMaxWidth={440}
        testID="sub2api-key-modal"
      >
        <Text style={styles.usageHint}>
          {editingKey
            ? text.modalHintEdit(scopeMeta.platform, scopeMeta.cliLabel)
            : text.modalHintCreate(scopeMeta.platform, scopeMeta.cliLabel)}
        </Text>
        <Text style={styles.fieldLabel}>{text.name}</Text>
        <AdaptiveTextInput
          value={draftName}
          onChangeText={setDraftName}
          placeholder={text.keyName}
          placeholderTextColor={theme.colors.foregroundMuted}
          autoCapitalize="none"
          autoCorrect={false}
          style={styles.textInput}
        />
        <View style={styles.groupComboWrap}>
          <ComboSelect
            label={text.groupLabel}
            title={text.selectGroupTitle(scopeMeta.cliLabel)}
            value={draftGroupId != null ? String(draftGroupId) : ""}
            options={scopedGroups.map((group) => ({
              id: String(group.id),
              label: group.name,
              description: `${group.platform} · ${group.rate_multiplier}x`,
            }))}
            placeholder={text.selectGroupPlaceholder(scopeMeta.platform)}
            isLoading={groupsQuery.isFetching}
            onSelect={(id) => setDraftGroupId(Number(id))}
            showLabel
            testID={`sub2api-key-modal-group-${activeScope}`}
          />
        </View>
        <View style={styles.formActions}>
          <Pressable
            onPress={closeModal}
            style={({ pressed }) => [styles.secondaryButton, pressed && styles.buttonPressed]}
          >
            <Text style={styles.secondaryButtonText}>{text.cancel}</Text>
          </Pressable>
          <Pressable
            onPress={() => void handleSaveKey()}
            style={({ pressed }) => [
              styles.primaryButton,
              pressed && styles.buttonPressed,
              (draftGroupId === null ||
                createKeyMutation.isPending ||
                updateKeyMutation.isPending) &&
                styles.disabledButton,
            ]}
            disabled={
              draftGroupId === null || createKeyMutation.isPending || updateKeyMutation.isPending
            }
          >
            <Text style={styles.primaryButtonText}>
              {createKeyMutation.isPending || updateKeyMutation.isPending
                ? text.saving
                : editingKey
                  ? text.save
                  : text.create}
            </Text>
          </Pressable>
        </View>
      </AdaptiveModalSheet>
    </>
  );
}

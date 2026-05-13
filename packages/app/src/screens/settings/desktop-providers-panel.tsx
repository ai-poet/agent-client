import { useCallback, useMemo, useState } from "react";
import { Alert, Pressable, ScrollView, Text, TextInput, View } from "react-native";
import { AdaptiveModalSheet } from "@/components/adaptive-modal-sheet";
import { invokeDesktopCommand } from "@/desktop/electron/invoke";
import { useUnistyles } from "react-native-unistyles";
import { SegmentedControl } from "@/components/ui/segmented-control";
import { SettingsSection } from "@/screens/settings/settings-section";
import { settingsStyles } from "@/styles/settings";
import { useAppSettings } from "@/hooks/use-settings";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";
import { AccessModeSection } from "@/screens/settings/access-mode-section";
import { useDesktopProvidersStore } from "@/screens/settings/desktop-providers-context";
import { managedProviderSettingsStyles as styles } from "@/screens/settings/managed-provider-settings-styles";
import {
  getErrorMessage,
  getCustomTargetSegmentOptions,
  providerTargetHint,
  maskApiKey,
  providerWritesClaude,
  providerWritesCodex,
} from "@/screens/settings/managed-provider-settings-shared";
import type { DesktopProviderPayload } from "@/screens/settings/sub2api-provider-types";

type ConfigPreviewTarget = "claude" | "codex" | null;

type ConfigSnapshot = {
  claudeSettings: string | null;
  codexAuth: string | null;
  codexConfig: string | null;
};

function RouteHeroCard({
  label,
  provider,
  text,
}: {
  label: string;
  provider: DesktopProviderPayload | null;
  text: ReturnType<typeof getSub2APIMessages>["settings"]["desktopProviders"];
}) {
  return (
    <View style={[settingsStyles.card, styles.cardBody, provider ? styles.heroCardActive : null]}>
      {provider ? (
        <>
          <View style={styles.heroTitleRow}>
            <View
              style={[styles.providerDotHero, styles.providerDotActive]}
              accessibilityLabel={text.active}
            />
            <Text style={styles.heroLabel}>{label}</Text>
          </View>
          <Text style={styles.heroName}>{provider.name}</Text>
          <Text style={styles.heroEndpoint}>{provider.endpoint}</Text>
          <Text style={styles.heroKeyHint}>{text.keyPrefix(maskApiKey(provider.apiKey))}</Text>
          <Text style={styles.heroMetaHint}>{providerTargetHint(provider, text)}</Text>
        </>
      ) : (
        <>
          <Text style={styles.heroLabel}>{label}</Text>
          <Text style={styles.heroName}>{text.notConfigured}</Text>
          <Text style={styles.sectionHint}>{text.chooseSavedEndpoint}</Text>
        </>
      )}
    </View>
  );
}

export function DesktopProvidersPanel() {
  const { theme } = useUnistyles();
  const { settings } = useAppSettings();
  const locale = useSub2APILocale();
  const messages = useMemo(() => getSub2APIMessages(locale).settings, [locale]);
  const text = messages.desktopProviders;
  const providerTargetText = messages.accessProviderTargets;
  const isByok = settings.accessMode === "byok";
  const [previewTarget, setPreviewTarget] = useState<ConfigPreviewTarget>(null);
  const [configSnapshot, setConfigSnapshot] = useState<ConfigSnapshot | null>(null);
  const [isPreviewLoading, setIsPreviewLoading] = useState(false);
  const {
    providers,
    activeClaudeProviderId,
    activeCodexProviderId,
    activeClaudeProvider,
    activeCodexProvider,
    showAddProviderForm,
    editProviderName,
    setEditProviderName,
    editProviderEndpoint,
    setEditProviderEndpoint,
    editProviderApiKey,
    setEditProviderApiKey,
    customTarget,
    setCustomTarget,
    openCustomProviderForm,
    closeCustomProviderForm,
    handleSwitchProvider,
    handleRemoveProvider,
    handleAddProvider,
  } = useDesktopProvidersStore();

  const openConfigPreview = useCallback(async (target: Exclude<ConfigPreviewTarget, null>) => {
    try {
      setIsPreviewLoading(true);
      const snapshot = await invokeDesktopCommand<ConfigSnapshot>("backup_config");
      setConfigSnapshot(snapshot);
      setPreviewTarget(target);
    } catch (error) {
      Alert.alert(text.unablePreviewConfig, getErrorMessage(error));
    } finally {
      setIsPreviewLoading(false);
    }
  }, [text.unablePreviewConfig]);

  const openConfigFile = useCallback(
    async (target: "claude-settings" | "codex-auth" | "codex-config") => {
      try {
        await invokeDesktopCommand("open_provider_config_file", { target });
      } catch (error) {
        Alert.alert(text.unableOpenConfig, getErrorMessage(error));
      }
    },
    [text.unableOpenConfig],
  );

  const previewTitle =
    previewTarget === "codex" ? text.previewTitleCodex : text.previewTitleClaude;
  const previewBlocks = useMemo(() => {
    if (!configSnapshot || !previewTarget) {
      return [];
    }
    if (previewTarget === "claude") {
      return [
        {
          title: "~/.claude/settings.json",
          contents: configSnapshot.claudeSettings,
        },
      ];
    }
    return [
      {
        title: "~/.codex/auth.json",
        contents: configSnapshot.codexAuth,
      },
      {
        title: "~/.codex/config.toml",
        contents: configSnapshot.codexConfig,
      },
    ];
  }, [configSnapshot, previewTarget]);

  return (
    <>
      <AccessModeSection />

      <SettingsSection title={text.activeRoutesTitle}>
        <Text style={[styles.sectionHint, { marginBottom: theme.spacing[2] }]}>
          {text.activeRoutesHint}
        </Text>
        <View style={styles.routeHeroStack}>
          <RouteHeroCard
            label={providerTargetText.claude}
            provider={activeClaudeProvider}
            text={text}
          />
          <RouteHeroCard
            label={providerTargetText.codex}
            provider={activeCodexProvider}
            text={text}
          />
        </View>
      </SettingsSection>

      <SettingsSection title={text.configFilesTitle}>
        <View style={[settingsStyles.card, styles.cardBody]}>
          <Text style={styles.sectionHint}>{text.configFilesHint}</Text>
          <View style={styles.scopeActionsRow}>
            <Pressable
              onPress={() => void openConfigPreview("claude")}
              style={({ pressed }) => [
                styles.secondaryButton,
                pressed && styles.buttonPressed,
                isPreviewLoading && styles.disabledButton,
              ]}
              disabled={isPreviewLoading}
            >
              <Text style={styles.secondaryButtonText}>{text.previewClaude}</Text>
            </Pressable>
            <Pressable
              onPress={() => void openConfigFile("claude-settings")}
              style={({ pressed }) => [styles.secondaryButton, pressed && styles.buttonPressed]}
            >
              <Text style={styles.secondaryButtonText}>{text.openClaudeFile}</Text>
            </Pressable>
          </View>
          <View style={styles.scopeActionsRow}>
            <Pressable
              onPress={() => void openConfigPreview("codex")}
              style={({ pressed }) => [
                styles.secondaryButton,
                pressed && styles.buttonPressed,
                isPreviewLoading && styles.disabledButton,
              ]}
              disabled={isPreviewLoading}
            >
              <Text style={styles.secondaryButtonText}>{text.previewCodex}</Text>
            </Pressable>
            <Pressable
              onPress={() => void openConfigFile("codex-auth")}
              style={({ pressed }) => [styles.secondaryButton, pressed && styles.buttonPressed]}
            >
              <Text style={styles.secondaryButtonText}>{text.openAuthJson}</Text>
            </Pressable>
            <Pressable
              onPress={() => void openConfigFile("codex-config")}
              style={({ pressed }) => [styles.secondaryButton, pressed && styles.buttonPressed]}
            >
              <Text style={styles.secondaryButtonText}>{text.openConfigToml}</Text>
            </Pressable>
          </View>
        </View>
      </SettingsSection>

      <SettingsSection title={text.savedEndpointsTitle}>
        {providers.length === 0 ? (
          <View style={styles.dashedCard}>
            <Text style={styles.emptyTitle}>{text.noSavedEndpoints}</Text>
            <Text style={styles.emptyBody}>{isByok ? text.noSavedByok : text.noSavedCloud}</Text>
          </View>
        ) : (
          <View style={settingsStyles.card}>
            {providers.map((provider, index) => {
              const forClaude = providerWritesClaude(provider);
              const forCodex = providerWritesCodex(provider);
              const claudeActive = activeClaudeProviderId === provider.id;
              const codexActive = activeCodexProviderId === provider.id;
              return (
                <View
                  key={provider.id}
                  style={[settingsStyles.row, index > 0 && settingsStyles.rowBorder]}
                >
                  <View style={settingsStyles.rowContent}>
                    <Text style={settingsStyles.rowTitle}>{provider.name}</Text>
                    <Text style={settingsStyles.rowHint}>{provider.endpoint}</Text>
                    <Text style={styles.providerMetaHint}>{providerTargetHint(provider, text)}</Text>
                    <View style={[styles.scopeActionsRow, { marginTop: theme.spacing[1] }]}>
                      {claudeActive ? <Text style={styles.scopeBadge}>{text.claudeActive}</Text> : null}
                      {codexActive ? <Text style={styles.scopeBadge}>{text.codexActive}</Text> : null}
                    </View>
                  </View>
                  <View style={[styles.providerActions, { flexWrap: "wrap", maxWidth: 200 }]}>
                    {forClaude ? (
                      <Pressable
                        onPress={() => void handleSwitchProvider(provider.id, "claude")}
                        style={({ pressed }) => [
                          styles.primaryButton,
                          styles.compactScopeButton,
                          pressed && styles.buttonPressed,
                          claudeActive && styles.disabledButton,
                        ]}
                        disabled={claudeActive}
                      >
                        <Text style={styles.primaryButtonText}>{text.useClaude}</Text>
                      </Pressable>
                    ) : null}
                    {forCodex ? (
                      <Pressable
                        onPress={() => void handleSwitchProvider(provider.id, "codex")}
                        style={({ pressed }) => [
                          styles.primaryButton,
                          styles.compactScopeButton,
                          pressed && styles.buttonPressed,
                          codexActive && styles.disabledButton,
                        ]}
                        disabled={codexActive}
                      >
                        <Text style={styles.primaryButtonText}>{text.useCodex}</Text>
                      </Pressable>
                    ) : null}
                    {!provider.isDefault ? (
                      <Pressable
                        onPress={() => void handleRemoveProvider(provider.id)}
                        style={({ pressed }) => [
                          styles.removeButton,
                          pressed && styles.buttonPressed,
                        ]}
                      >
                        <Text style={styles.removeButtonText}>{text.remove}</Text>
                      </Pressable>
                    ) : null}
                  </View>
                </View>
              );
            })}
          </View>
        )}
      </SettingsSection>

      <SettingsSection title={text.customEndpointTitle}>
        <View style={[settingsStyles.card, styles.cardBody]}>
          {showAddProviderForm ? (
            <View style={styles.formBody}>
              <Text style={styles.fieldLabel}>{text.target}</Text>
              <SegmentedControl
                options={getCustomTargetSegmentOptions(providerTargetText)}
                value={customTarget}
                onValueChange={setCustomTarget}
                size="sm"
              />
              {customTarget === "claude" ? (
                <Text style={styles.usageHint}>{text.claudeUsageHint}</Text>
              ) : (
                <Text style={styles.usageHint}>{text.codexUsageHint}</Text>
              )}
              <Text style={styles.fieldLabel}>{text.name}</Text>
              <TextInput
                value={editProviderName}
                onChangeText={setEditProviderName}
                placeholder={text.providerNamePlaceholder}
                placeholderTextColor={theme.colors.foregroundMuted}
                autoCapitalize="none"
                autoCorrect={false}
                style={styles.textInput}
              />
              <Text style={styles.fieldLabel}>{text.endpoint}</Text>
              <Text style={styles.usageHint}>{text.endpointHint}</Text>
              <TextInput
                value={editProviderEndpoint}
                onChangeText={setEditProviderEndpoint}
                placeholder={text.endpointPlaceholder}
                placeholderTextColor={theme.colors.foregroundMuted}
                autoCapitalize="none"
                autoCorrect={false}
                style={styles.textInput}
              />
              <Text style={styles.fieldLabel}>{text.apiKey}</Text>
              <Text style={styles.usageHint}>
                {customTarget === "claude" ? text.claudeCredentialHint : text.codexCredentialHint}
              </Text>
              <TextInput
                value={editProviderApiKey}
                onChangeText={setEditProviderApiKey}
                placeholder={text.apiKey}
                placeholderTextColor={theme.colors.foregroundMuted}
                autoCapitalize="none"
                autoCorrect={false}
                secureTextEntry
                style={styles.textInput}
              />
              <View style={styles.formActions}>
                <Pressable
                  onPress={() => void handleAddProvider()}
                  style={({ pressed }) => [styles.primaryButton, pressed && styles.buttonPressed]}
                >
                  <Text style={styles.primaryButtonText}>{text.add}</Text>
                </Pressable>
                <Pressable
                  onPress={closeCustomProviderForm}
                  style={({ pressed }) => [styles.secondaryButton, pressed && styles.buttonPressed]}
                >
                  <Text style={styles.secondaryButtonText}>{text.cancel}</Text>
                </Pressable>
              </View>
            </View>
          ) : (
            <Pressable
              onPress={openCustomProviderForm}
              style={({ pressed }) => [styles.addProviderButton, pressed && styles.buttonPressed]}
            >
              <Text style={styles.addProviderButtonText}>{text.addCustomProvider}</Text>
            </Pressable>
          )}
        </View>
      </SettingsSection>

      <AdaptiveModalSheet
        title={previewTitle}
        visible={previewTarget !== null}
        onClose={() => setPreviewTarget(null)}
        desktopMaxWidth={760}
        testID="config-preview-modal"
      >
        <ScrollView style={{ maxHeight: 480 }}>
          <View style={styles.formBody}>
            {previewBlocks.map((block) => (
              <View key={block.title} style={styles.routeSummaryCard}>
                <Text style={styles.formTitle}>{block.title}</Text>
                <Text style={styles.usageHint}>
                  {block.contents
                    ? text.currentOnDiskContents
                    : text.fileNotCreatedYet}
                </Text>
                <View style={styles.configPreviewBlock}>
                  <Text
                    selectable
                    style={{
                      color: theme.colors.foreground,
                      fontSize: theme.fontSize.xs,
                      fontFamily: "monospace",
                    }}
                  >
                    {block.contents ?? text.emptyFile}
                  </Text>
                </View>
              </View>
            ))}
          </View>
        </ScrollView>
      </AdaptiveModalSheet>
    </>
  );
}

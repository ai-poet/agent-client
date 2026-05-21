import { useCallback, useMemo, useState, useSyncExternalStore } from "react";
import type { ComponentType, ReactNode } from "react";
import { Alert, Pressable, ScrollView, Text, View } from "react-native";
import { useRouter } from "expo-router";
import { useFocusEffect } from "@react-navigation/native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { Buffer } from "buffer";
import {
  ArrowLeft,
  Sun,
  Moon,
  Monitor,
  ChevronDown,
  Settings,
  Server,
  Keyboard,
  Stethoscope,
  Info,
  Shield,
  Puzzle,
  Plus,
  Cloud,
  RotateCcw,
} from "lucide-react-native";
import { SidebarHeaderRow } from "@/components/sidebar/sidebar-header-row";
import { SidebarSeparator } from "@/components/sidebar/sidebar-separator";
import { ScreenTitle } from "@/components/headers/screen-title";
import { HeaderIconBadge } from "@/components/headers/header-icon-badge";
import { SettingsSection } from "@/screens/settings/settings-section";
import {
  useAppSettings,
  type AppLanguage,
  type AppSettings,
  type SendBehavior,
} from "@/hooks/use-settings";
import { THEME_SWATCHES } from "@/styles/theme";
import { getHostRuntimeStore, isHostRuntimeConnected, useHosts } from "@/runtime/host-runtime";
import { TitlebarDragRegion } from "@/components/desktop/titlebar-drag-region";
import { useWindowControlsPadding } from "@/utils/desktop-window";
import { confirmDialog } from "@/utils/confirm-dialog";
import { resetPaseoEnvironment } from "@/utils/reset-paseo-environment";
import { BackHeader } from "@/components/headers/back-header";
import { ScreenHeader } from "@/components/headers/screen-header";
import { AddHostMethodModal } from "@/components/add-host-method-modal";
import { AddHostModal } from "@/components/add-host-modal";
import { PairLinkModal } from "@/components/pair-link-modal";
import { KeyboardShortcutsSection } from "@/screens/settings/keyboard-shortcuts-section";
import { Button } from "@/components/ui/button";
import { SegmentedControl } from "@/components/ui/segmented-control";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { DesktopPermissionsSection } from "@/desktop/components/desktop-permissions-section";
import { IntegrationsSection } from "@/desktop/components/integrations-section";
import { isElectronRuntime } from "@/desktop/host";
import { useDesktopAppUpdater } from "@/desktop/updates/use-desktop-app-updater";
import { formatVersionWithPrefix } from "@/desktop/updates/desktop-updates";
import { resolveAppVersion } from "@/utils/app-version";
import { settingsStyles } from "@/styles/settings";
import { THINKING_TONE_NATIVE_PCM_BASE64 } from "@/utils/thinking-tone.native-pcm";
import { useVoiceAudioEngineOptional } from "@/contexts/voice-context";
import { HostPage, HostRenameButton } from "@/screens/settings/host-page";
import { ManagedProviderSettingsPage } from "@/screens/settings/managed-provider-settings-page";
import { PaseoCloudSettingsPage } from "@/screens/settings/paseo-cloud-settings-page";
import { DesktopProvidersStoreProvider } from "@/screens/settings/desktop-providers-context";
import { useIsCompactFormFactor } from "@/constants/layout";
import { useLocalDaemonServerId } from "@/hooks/use-is-local-daemon";
import { APP_NAME, CLOUD_NAME } from "@/config/branding";
import { getSub2APIMessages, resolveSub2APILocaleFromPreference } from "@/i18n/sub2api";
import {
  buildHostOpenProjectRoute,
  buildSettingsHostRoute,
  buildSettingsSectionRoute,
  type SettingsSectionSlug,
} from "@/utils/host-routes";

// ---------------------------------------------------------------------------
// View model
// ---------------------------------------------------------------------------

export type SettingsView =
  | { kind: "root" }
  | { kind: "section"; section: SettingsSectionSlug }
  | { kind: "host"; serverId: string };

interface SidebarSectionItem {
  id: SettingsSectionSlug;
  icon: ComponentType<{ size: number; color: string }>;
  desktopOnly?: boolean;
}

const SIDEBAR_SECTION_ITEMS: SidebarSectionItem[] = [
  { id: "general", icon: Settings },
  { id: "paseo-cloud", icon: Cloud, desktopOnly: true },
  { id: "managed-provider", icon: Server, desktopOnly: true },
  { id: "shortcuts", icon: Keyboard, desktopOnly: true },
  { id: "integrations", icon: Puzzle, desktopOnly: true },
  { id: "permissions", icon: Shield, desktopOnly: true },
  { id: "diagnostics", icon: Stethoscope },
  { id: "about", icon: Info },
];

type SettingsText = ReturnType<typeof getSub2APIMessages>["settings"];

function getSettingsSectionLabel(section: SettingsSectionSlug, text: SettingsText): string {
  switch (section) {
    case "general":
      return text.sections.general;
    case "paseo-cloud":
      return CLOUD_NAME;
    case "managed-provider":
      return text.sections.managedProvider;
    case "shortcuts":
      return text.sections.shortcuts;
    case "integrations":
      return text.sections.integrations;
    case "permissions":
      return text.sections.permissions;
    case "diagnostics":
      return text.sections.diagnostics;
    case "about":
      return text.sections.about;
  }
}

// ---------------------------------------------------------------------------
// Theme helpers (General section)
// ---------------------------------------------------------------------------

function ThemeIcon({
  theme,
  size,
  color,
}: {
  theme: AppSettings["theme"];
  size: number;
  color: string;
}) {
  switch (theme) {
    case "light":
      return <Sun size={size} color={color} />;
    case "dark":
      return <Moon size={size} color={color} />;
    case "auto":
      return <Monitor size={size} color={color} />;
    default:
      return <ThemeSwatch color={THEME_SWATCHES[theme]} size={size} />;
  }
}

function ThemeSwatch({ color, size }: { color: string; size: number }) {
  return (
    <View
      style={{
        width: size,
        height: size,
        borderRadius: size / 2,
        backgroundColor: color,
        borderWidth: 1,
        borderColor: "rgba(255,255,255,0.15)",
      }}
    />
  );
}

const LANGUAGE_VALUES: AppLanguage[] = ["auto", "zh", "en"];

// ---------------------------------------------------------------------------
// Section components
// ---------------------------------------------------------------------------

interface GeneralSectionProps {
  settings: AppSettings;
  text: SettingsText;
  handleThemeChange: (theme: AppSettings["theme"]) => void;
  handleLanguageChange: (language: AppLanguage) => void;
  handleSendBehaviorChange: (behavior: SendBehavior) => void;
}

function GeneralSection({
  settings,
  text,
  handleThemeChange,
  handleLanguageChange,
  handleSendBehaviorChange,
}: GeneralSectionProps) {
  const { theme } = useUnistyles();
  const iconSize = theme.iconSize.md;
  const iconColor = theme.colors.foregroundMuted;

  return (
    <SettingsSection title={text.general}>
      <View style={settingsStyles.card}>
        <View style={settingsStyles.row}>
          <View style={settingsStyles.rowContent}>
            <Text style={settingsStyles.rowTitle}>{text.theme}</Text>
          </View>
          <DropdownMenu>
            <DropdownMenuTrigger
              style={({ pressed }) => [styles.themeTrigger, pressed && { opacity: 0.85 }]}
            >
              <ThemeIcon theme={settings.theme} size={iconSize} color={iconColor} />
              <Text style={styles.themeTriggerText}>{text.themes[settings.theme]}</Text>
              <ChevronDown size={theme.iconSize.sm} color={iconColor} />
            </DropdownMenuTrigger>
            <DropdownMenuContent side="bottom" align="end" width={200}>
              {(["light", "dark", "auto"] as const).map((t) => (
                <DropdownMenuItem
                  key={t}
                  selected={settings.theme === t}
                  onSelect={() => handleThemeChange(t)}
                  leading={<ThemeIcon theme={t} size={iconSize} color={iconColor} />}
                >
                  {text.themes[t]}
                </DropdownMenuItem>
              ))}
              <DropdownMenuSeparator />
              {(["zinc", "midnight", "claude", "ghostty"] as const).map((t) => (
                <DropdownMenuItem
                  key={t}
                  selected={settings.theme === t}
                  onSelect={() => handleThemeChange(t)}
                  leading={<ThemeIcon theme={t} size={iconSize} color={iconColor} />}
                >
                  {text.themes[t]}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
        </View>
        <View style={[settingsStyles.row, settingsStyles.rowBorder]}>
          <View style={settingsStyles.rowContent}>
            <Text style={settingsStyles.rowTitle}>{text.language}</Text>
            <Text style={settingsStyles.rowHint}>{text.languageHint}</Text>
          </View>
          <SegmentedControl
            size="sm"
            value={settings.language}
            onValueChange={handleLanguageChange}
            options={LANGUAGE_VALUES.map((value) => ({ value, label: text.languages[value] }))}
          />
        </View>
        <View style={[settingsStyles.row, settingsStyles.rowBorder]}>
          <View style={settingsStyles.rowContent}>
            <Text style={settingsStyles.rowTitle}>{text.defaultSend}</Text>
            <Text style={settingsStyles.rowHint}>{text.defaultSendHint}</Text>
          </View>
          <SegmentedControl
            size="sm"
            value={settings.sendBehavior}
            onValueChange={handleSendBehaviorChange}
            options={[
              { value: "interrupt", label: text.interrupt },
              { value: "queue", label: text.queue },
            ]}
          />
        </View>
      </View>
    </SettingsSection>
  );
}

interface DiagnosticsSectionProps {
  text: SettingsText;
  voiceAudioEngine: ReturnType<typeof useVoiceAudioEngineOptional>;
  isPlaybackTestRunning: boolean;
  playbackTestResult: string | null;
  handlePlaybackTest: () => Promise<void>;
}

function DiagnosticsSection({
  text,
  voiceAudioEngine,
  isPlaybackTestRunning,
  playbackTestResult,
  handlePlaybackTest,
}: DiagnosticsSectionProps) {
  return (
    <SettingsSection title={text.diagnostics}>
      <View style={settingsStyles.card}>
        <View style={settingsStyles.row}>
          <View style={settingsStyles.rowContent}>
            <Text style={settingsStyles.rowTitle}>{text.testAudio}</Text>
            {playbackTestResult ? (
              <Text style={settingsStyles.rowHint}>{playbackTestResult}</Text>
            ) : null}
          </View>
          <Button
            variant="secondary"
            size="sm"
            onPress={() => void handlePlaybackTest()}
            disabled={!voiceAudioEngine || isPlaybackTestRunning}
          >
            {isPlaybackTestRunning ? text.playing : text.playTest}
          </Button>
        </View>
      </View>
    </SettingsSection>
  );
}

interface DangerZoneSectionProps {
  text: SettingsText;
}

function DangerZoneSection({ text }: DangerZoneSectionProps) {
  const [isResetting, setIsResetting] = useState(false);

  const handleResetEnvironment = useCallback(() => {
    if (isResetting) {
      return;
    }
    void confirmDialog({
      title: text.resetEnvironmentTitle,
      message: text.resetEnvironmentConfirmMessage,
      confirmLabel: text.resetEnvironmentAction,
      cancelLabel: text.cancel,
      destructive: true,
    })
      .then((confirmed) => {
        if (!confirmed) {
          return;
        }
        setIsResetting(true);
        void resetPaseoEnvironment().catch((error) => {
          setIsResetting(false);
          const message = error instanceof Error ? error.message : String(error);
          Alert.alert(text.error, text.resetEnvironmentFailed(message));
        });
      })
      .catch((error) => {
        console.error("[Settings] Failed to open reset confirmation", error);
        Alert.alert(text.error, text.resetEnvironmentFailed(String(error)));
      });
  }, [isResetting, text]);

  return (
    <SettingsSection title={text.sections.dangerZone}>
      <View style={settingsStyles.card}>
        <View style={settingsStyles.row}>
          <View style={settingsStyles.rowContent}>
            <Text style={settingsStyles.rowTitle}>{text.resetEnvironmentTitle}</Text>
            <Text style={settingsStyles.rowHint}>{text.resetEnvironmentHint}</Text>
          </View>
          <Button
            variant="ghost"
            size="sm"
            onPress={handleResetEnvironment}
            disabled={isResetting}
            leftIcon={RotateCcw}
            textStyle={styles.dangerText}
            testID="settings-reset-environment"
          >
            {isResetting ? text.resettingEnvironment : text.resetEnvironmentAction}
          </Button>
        </View>
      </View>
    </SettingsSection>
  );
}

interface AboutSectionProps {
  text: SettingsText;
  appVersionText: string;
  isDesktopApp: boolean;
}

function AboutSection({ text, appVersionText, isDesktopApp }: AboutSectionProps) {
  return (
    <SettingsSection title={text.about}>
      <View style={settingsStyles.card}>
        <View style={settingsStyles.row}>
          <View style={settingsStyles.rowContent}>
            <Text style={settingsStyles.rowTitle}>{text.version}</Text>
          </View>
          <Text style={styles.aboutValue}>{appVersionText}</Text>
        </View>
        {isDesktopApp ? <DesktopAppUpdateRow text={text} /> : null}
      </View>
    </SettingsSection>
  );
}

function DesktopAppUpdateRow({ text }: { text: SettingsText }) {
  const { settings, updateSettings } = useAppSettings();
  const {
    isDesktopApp,
    statusText,
    availableUpdate,
    errorMessage,
    isChecking,
    isInstalling,
    checkForUpdates,
    installUpdate,
  } = useDesktopAppUpdater();

  useFocusEffect(
    useCallback(() => {
      if (!isDesktopApp) {
        return undefined;
      }
      void checkForUpdates({ silent: true });
      return undefined;
    }, [checkForUpdates, isDesktopApp]),
  );

  const handleCheckForUpdates = useCallback(() => {
    if (!isDesktopApp) {
      return;
    }
    void checkForUpdates();
  }, [checkForUpdates, isDesktopApp]);

  const handleReleaseChannelChange = useCallback(
    (releaseChannel: AppSettings["releaseChannel"]) => {
      void updateSettings({ releaseChannel });
    },
    [updateSettings],
  );

  const handleInstallUpdate = useCallback(() => {
    if (!isDesktopApp) {
      return;
    }

    void confirmDialog({
      title: text.installDesktopUpdate,
      message: text.installDesktopUpdateMessage(APP_NAME),
      confirmLabel: text.installUpdate,
      cancelLabel: text.cancel,
    })
      .then((confirmed) => {
        if (!confirmed) {
          return;
        }
        void installUpdate();
      })
      .catch((error) => {
        console.error("[Settings] Failed to open app update confirmation", error);
        Alert.alert(text.error, text.unableOpenUpdateConfirmation);
      });
  }, [installUpdate, isDesktopApp, text]);

  if (!isDesktopApp) {
    return null;
  }

  return (
    <>
      <View style={[settingsStyles.row, settingsStyles.rowBorder]}>
        <View style={settingsStyles.rowContent}>
          <Text style={settingsStyles.rowTitle}>{text.releaseChannel}</Text>
          <Text style={settingsStyles.rowHint}>{text.releaseChannelHint}</Text>
        </View>
        <SegmentedControl
          size="sm"
          value={settings.releaseChannel}
          onValueChange={handleReleaseChannelChange}
          options={[
            { value: "stable", label: text.stable },
            { value: "beta", label: text.beta },
          ]}
        />
      </View>
      <View style={[settingsStyles.row, settingsStyles.rowBorder]}>
        <View style={settingsStyles.rowContent}>
          <Text style={settingsStyles.rowTitle}>{text.appUpdates}</Text>
          <Text style={settingsStyles.rowHint}>{statusText}</Text>
          {availableUpdate?.latestVersion ? (
            <Text style={settingsStyles.rowHint}>
              {text.readyToInstall(formatVersionWithPrefix(availableUpdate.latestVersion))}
            </Text>
          ) : null}
          {errorMessage ? <Text style={styles.aboutErrorText}>{errorMessage}</Text> : null}
        </View>
        <View style={styles.aboutUpdateActions}>
          <Button
            variant="outline"
            size="sm"
            onPress={handleCheckForUpdates}
            disabled={isChecking || isInstalling}
          >
            {isChecking ? text.checking : text.check}
          </Button>
          <Button
            variant="default"
            size="sm"
            onPress={handleInstallUpdate}
            disabled={isChecking || isInstalling || !availableUpdate}
          >
            {isInstalling
              ? text.installing
              : availableUpdate?.latestVersion
                ? text.updateTo(formatVersionWithPrefix(availableUpdate.latestVersion))
                : text.update}
          </Button>
        </View>
      </View>
    </>
  );
}

// ---------------------------------------------------------------------------
// Sidebar
// ---------------------------------------------------------------------------

function useAnyOnlineHostServerId(serverIds: string[]): string | null {
  const runtime = getHostRuntimeStore();
  return useSyncExternalStore(
    (onStoreChange) => runtime.subscribeAll(onStoreChange),
    () => {
      let firstOnlineServerId: string | null = null;
      let firstOnlineAt: string | null = null;
      for (const serverId of serverIds) {
        const snapshot = runtime.getSnapshot(serverId);
        const lastOnlineAt = snapshot?.lastOnlineAt ?? null;
        if (!isHostRuntimeConnected(snapshot) || !lastOnlineAt) {
          continue;
        }
        if (!firstOnlineAt || lastOnlineAt < firstOnlineAt) {
          firstOnlineAt = lastOnlineAt;
          firstOnlineServerId = serverId;
        }
      }
      return firstOnlineServerId;
    },
    () => null,
  );
}

interface SettingsSidebarProps {
  view: SettingsView;
  text: SettingsText;
  onSelectSection: (section: SettingsSectionSlug) => void;
  onSelectHost: (serverId: string) => void;
  onAddHost: () => void;
  onBackToWorkspace: () => void;
  layout: "desktop" | "mobile";
}

function SettingsSidebar({
  view,
  text,
  onSelectSection,
  onSelectHost,
  onAddHost,
  onBackToWorkspace,
  layout,
}: SettingsSidebarProps) {
  const { theme } = useUnistyles();
  const hosts = useHosts();
  const localServerId = useLocalDaemonServerId();
  const sortedHosts = useMemo(() => {
    if (!localServerId) {
      return hosts;
    }
    const localIndex = hosts.findIndex((host) => host.serverId === localServerId);
    if (localIndex <= 0) {
      return hosts;
    }
    const next = hosts.slice();
    const [local] = next.splice(localIndex, 1);
    next.unshift(local);
    return next;
  }, [hosts, localServerId]);
  const isDesktopApp = isElectronRuntime();
  const items = SIDEBAR_SECTION_ITEMS.filter((item) => !item.desktopOnly || isDesktopApp);
  const padding = useWindowControlsPadding("sidebar");
  const isDesktop = layout === "desktop";
  const containerStyle = isDesktop ? sidebarStyles.desktopContainer : sidebarStyles.mobileContainer;
  const selectedSectionId = view.kind === "section" ? view.section : null;
  const selectedServerId = view.kind === "host" ? view.serverId : null;

  return (
    <View style={containerStyle} testID="settings-sidebar">
      {isDesktop ? (
        <>
          <TitlebarDragRegion />
          {padding.top > 0 ? <View style={{ height: padding.top }} /> : null}
        </>
      ) : null}
      {isDesktop ? (
        <SidebarHeaderRow
          icon={ArrowLeft}
          label={text.back}
          onPress={onBackToWorkspace}
          testID="settings-back-to-workspace"
        />
      ) : null}
      <View style={sidebarStyles.list}>
        {items.map((item) => {
          const isSelected = selectedSectionId === item.id;
          const IconComponent = item.icon;
          return (
            <Pressable
              key={item.id}
              accessibilityRole="button"
              accessibilityState={{ selected: isSelected }}
              onPress={() => onSelectSection(item.id)}
              style={({ hovered = false }) => [
                sidebarStyles.item,
                hovered && sidebarStyles.itemHovered,
                isSelected && sidebarStyles.itemSelected,
              ]}
            >
              <IconComponent
                size={theme.iconSize.md}
                color={isSelected ? theme.colors.foreground : theme.colors.foregroundMuted}
              />
              <Text
                style={[sidebarStyles.label, isSelected && { color: theme.colors.foreground }]}
                numberOfLines={1}
              >
                {getSettingsSectionLabel(item.id, text)}
              </Text>
            </Pressable>
          );
        })}
      </View>
      <SidebarSeparator />
      <View style={sidebarStyles.list}>
        {sortedHosts.map((host) => {
          const isSelected = selectedServerId === host.serverId;
          const isLocal = localServerId !== null && host.serverId === localServerId;
          return (
            <Pressable
              key={host.serverId}
              accessibilityRole="button"
              accessibilityState={{ selected: isSelected }}
              onPress={() => onSelectHost(host.serverId)}
              testID={`settings-host-entry-${host.serverId}`}
              style={({ hovered = false }) => [
                sidebarStyles.item,
                hovered && sidebarStyles.itemHovered,
                isSelected && sidebarStyles.itemSelected,
              ]}
            >
              <Server
                size={theme.iconSize.md}
                color={isSelected ? theme.colors.foreground : theme.colors.foregroundMuted}
              />
              <Text
                style={[sidebarStyles.label, isSelected && { color: theme.colors.foreground }]}
                numberOfLines={1}
              >
                {host.label}
              </Text>
              {isLocal ? (
                <Text style={sidebarStyles.localMarker} testID="settings-host-local-marker">
                  {text.local}
                </Text>
              ) : null}
            </Pressable>
          );
        })}
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={text.addHostLabel}
          onPress={onAddHost}
          testID="settings-add-host"
          style={({ hovered = false }) => [
            sidebarStyles.item,
            hovered && sidebarStyles.itemHovered,
          ]}
        >
          <Plus size={theme.iconSize.md} color={theme.colors.foregroundMuted} />
          <Text style={sidebarStyles.label} numberOfLines={1}>
            {text.addHostLabel}
          </Text>
        </Pressable>
      </View>
    </View>
  );
}

// ---------------------------------------------------------------------------
// Main screen
// ---------------------------------------------------------------------------

export interface SettingsScreenProps {
  view: SettingsView;
}

export default function SettingsScreen({ view }: SettingsScreenProps) {
  const router = useRouter();
  const { theme } = useUnistyles();
  const voiceAudioEngine = useVoiceAudioEngineOptional();
  const { settings, isLoading: settingsLoading, updateSettings } = useAppSettings();
  const appLocale = useMemo(
    () => resolveSub2APILocaleFromPreference(settings.language),
    [settings.language],
  );
  const settingsText = useMemo(() => getSub2APIMessages(appLocale).settings, [appLocale]);
  const [isAddHostMethodVisible, setIsAddHostMethodVisible] = useState(false);
  const [isDirectHostVisible, setIsDirectHostVisible] = useState(false);
  const [isPasteLinkVisible, setIsPasteLinkVisible] = useState(false);
  const [isPlaybackTestRunning, setIsPlaybackTestRunning] = useState(false);
  const [playbackTestResult, setPlaybackTestResult] = useState<string | null>(null);
  const isDesktopApp = isElectronRuntime();
  const appVersion = resolveAppVersion();
  const appVersionText = formatVersionWithPrefix(appVersion);
  const isCompactLayout = useIsCompactFormFactor();
  const insets = useSafeAreaInsets();
  const hosts = useHosts();
  const hostServerIds = useMemo(() => hosts.map((host) => host.serverId), [hosts]);
  const anyOnlineServerId = useAnyOnlineHostServerId(hostServerIds);

  const handleThemeChange = useCallback(
    (nextTheme: AppSettings["theme"]) => {
      void updateSettings({ theme: nextTheme });
    },
    [updateSettings],
  );

  const handleLanguageChange = useCallback(
    (language: AppLanguage) => {
      void updateSettings({ language });
    },
    [updateSettings],
  );

  const handleSendBehaviorChange = useCallback(
    (behavior: SendBehavior) => {
      void updateSettings({ sendBehavior: behavior });
    },
    [updateSettings],
  );

  const handlePlaybackTest = useCallback(async () => {
    if (!voiceAudioEngine || isPlaybackTestRunning) {
      return;
    }

    setIsPlaybackTestRunning(true);
    setPlaybackTestResult(null);

    try {
      const bytes = Buffer.from(THINKING_TONE_NATIVE_PCM_BASE64, "base64");
      await voiceAudioEngine.initialize();
      voiceAudioEngine.stop();
      await voiceAudioEngine.play({
        type: "audio/pcm;rate=16000;bits=16",
        size: bytes.byteLength,
        async arrayBuffer() {
          return Uint8Array.from(bytes).buffer;
        },
      });
      setPlaybackTestResult(null);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      console.error("[Settings] Playback test failed", error);
      setPlaybackTestResult(settingsText.playbackFailed(message));
    } finally {
      setIsPlaybackTestRunning(false);
    }
  }, [isPlaybackTestRunning, settingsText, voiceAudioEngine]);

  const closeAddConnectionFlow = useCallback(() => {
    setIsAddHostMethodVisible(false);
    setIsDirectHostVisible(false);
    setIsPasteLinkVisible(false);
  }, []);

  const goBackToAddConnectionMethods = useCallback(() => {
    setIsDirectHostVisible(false);
    setIsPasteLinkVisible(false);
    setIsAddHostMethodVisible(true);
  }, []);

  const handleAddHost = useCallback(() => {
    setIsAddHostMethodVisible(true);
  }, []);

  const handleHostAdded = useCallback(
    ({ serverId }: { serverId: string }) => {
      const target = buildSettingsHostRoute(serverId);
      if (isCompactLayout) {
        router.push(target);
      } else {
        router.replace(target);
      }
    },
    [isCompactLayout, router],
  );

  const handleSelectSection = useCallback(
    (section: SettingsSectionSlug) => {
      const target = buildSettingsSectionRoute(section);
      if (isCompactLayout) {
        router.push(target);
      } else {
        router.replace(target);
      }
    },
    [isCompactLayout, router],
  );

  const handleSelectHost = useCallback(
    (serverId: string) => {
      const target = buildSettingsHostRoute(serverId);
      if (isCompactLayout) {
        router.push(target);
      } else {
        router.replace(target);
      }
    },
    [isCompactLayout, router],
  );

  const handleScanQr = useCallback(() => {
    closeAddConnectionFlow();
    router.push({
      pathname: "/pair-scan",
      params: { source: "settings" },
    });
  }, [closeAddConnectionFlow, router]);

  const handleHostRemoved = useCallback(() => {
    const fallback = buildSettingsSectionRoute("general");
    if (isCompactLayout) {
      router.replace("/settings");
    } else {
      router.replace(fallback);
    }
  }, [isCompactLayout, router]);

  const handleBackToRoot = useCallback(() => {
    if (router.canGoBack()) {
      router.back();
    } else {
      router.replace("/settings");
    }
  }, [router]);

  const handleBackToWorkspace = useCallback(() => {
    if (router.canGoBack()) {
      router.back();
      return;
    }
    if (anyOnlineServerId) {
      router.replace(buildHostOpenProjectRoute(anyOnlineServerId));
      return;
    }
    router.replace("/");
  }, [anyOnlineServerId, router]);

  const detailHeader = ((): {
    title: string;
    Icon: ComponentType<{ size: number; color: string }>;
    titleAccessory?: ReactNode;
  } | null => {
    if (view.kind === "host") {
      const host = hosts.find((h) => h.serverId === view.serverId);
      if (!host) return null;
      return {
        title: host.label,
        Icon: Server,
        titleAccessory: <HostRenameButton host={host} />,
      };
    }
    if (view.kind === "section") {
      const item = SIDEBAR_SECTION_ITEMS.find((s) => s.id === view.section);
      if (!item) return null;
      return { title: getSettingsSectionLabel(item.id, settingsText), Icon: item.icon };
    }
    return null;
  })();

  const content = (() => {
    if (view.kind === "host") {
      return <HostPage serverId={view.serverId} onHostRemoved={handleHostRemoved} />;
    }
    if (view.kind === "section") {
      switch (view.section) {
        case "general":
          return (
            <GeneralSection
              settings={settings}
              text={settingsText}
              handleThemeChange={handleThemeChange}
              handleLanguageChange={handleLanguageChange}
              handleSendBehaviorChange={handleSendBehaviorChange}
            />
          );
        case "paseo-cloud":
          return isDesktopApp ? <PaseoCloudSettingsPage text={settingsText} /> : null;
        case "managed-provider":
          return isDesktopApp ? <ManagedProviderSettingsPage text={settingsText} /> : null;
        case "shortcuts":
          return isDesktopApp ? <KeyboardShortcutsSection /> : null;
        case "integrations":
          return isDesktopApp ? <IntegrationsSection /> : null;
        case "permissions":
          return isDesktopApp ? <DesktopPermissionsSection /> : null;
        case "diagnostics":
          return (
            <>
              <DiagnosticsSection
                text={settingsText}
                voiceAudioEngine={voiceAudioEngine}
                isPlaybackTestRunning={isPlaybackTestRunning}
                playbackTestResult={playbackTestResult}
                handlePlaybackTest={handlePlaybackTest}
              />
              {isDesktopApp ? <DangerZoneSection text={settingsText} /> : null}
            </>
          );
        case "about":
          return (
            <AboutSection
              text={settingsText}
              appVersionText={appVersionText}
              isDesktopApp={isDesktopApp}
            />
          );
      }
    }
    return null;
  })();

  const needsDesktopProvidersStore =
    isDesktopApp &&
    view.kind === "section" &&
    (view.section === "paseo-cloud" || view.section === "managed-provider");

  const renderedContent = needsDesktopProvidersStore ? (
    <DesktopProvidersStoreProvider>{content}</DesktopProvidersStoreProvider>
  ) : (
    content
  );

  if (settingsLoading) {
    return (
      <View style={styles.loadingContainer}>
        <Text style={styles.loadingText}>{settingsText.loadingSettings}</Text>
      </View>
    );
  }

  const addHostModals = (
    <>
      <AddHostMethodModal
        visible={isAddHostMethodVisible}
        onClose={closeAddConnectionFlow}
        onDirectConnection={() => {
          setIsAddHostMethodVisible(false);
          setIsDirectHostVisible(true);
        }}
        onPasteLink={() => {
          setIsAddHostMethodVisible(false);
          setIsPasteLinkVisible(true);
        }}
        onScanQr={handleScanQr}
      />
      <AddHostModal
        visible={isDirectHostVisible}
        onClose={closeAddConnectionFlow}
        onCancel={goBackToAddConnectionMethods}
        onSaved={handleHostAdded}
      />
      <PairLinkModal
        visible={isPasteLinkVisible}
        onClose={closeAddConnectionFlow}
        onCancel={goBackToAddConnectionMethods}
        onSaved={handleHostAdded}
      />
    </>
  );

  // Mobile root: full-screen sidebar-as-list.
  if (isCompactLayout && view.kind === "root") {
    return (
      <View style={styles.container}>
        <BackHeader title={settingsText.title} onBack={handleBackToWorkspace} />
        <ScrollView
          style={styles.scrollView}
          contentContainerStyle={{ paddingBottom: insets.bottom }}
        >
          <SettingsSidebar
            view={view}
            text={settingsText}
            onSelectSection={handleSelectSection}
            onSelectHost={handleSelectHost}
            onAddHost={handleAddHost}
            onBackToWorkspace={handleBackToWorkspace}
            layout="mobile"
          />
        </ScrollView>
        {addHostModals}
      </View>
    );
  }

  // Mobile detail: full-screen content with a back header that returns to the list.
  if (isCompactLayout) {
    return (
      <View style={styles.container}>
        <BackHeader
          title={detailHeader?.title}
          titleAccessory={detailHeader?.titleAccessory}
          onBack={handleBackToRoot}
        />
        <ScrollView
          style={styles.scrollView}
          contentContainerStyle={{ paddingBottom: insets.bottom }}
        >
          <View style={styles.content}>{renderedContent}</View>
        </ScrollView>
        {addHostModals}
      </View>
    );
  }

  // Desktop split view — mirrors AppContainer: sidebar owns the titlebar drag
  // region + traffic-light padding; detail pane renders whatever header the
  // selected section provides.
  return (
    <View style={styles.container}>
      <View style={desktopStyles.row}>
        <SettingsSidebar
          view={view}
          text={settingsText}
          onSelectSection={handleSelectSection}
          onSelectHost={handleSelectHost}
          onAddHost={handleAddHost}
          onBackToWorkspace={handleBackToWorkspace}
          layout="desktop"
        />
        <View style={desktopStyles.contentPane}>
          <ScreenHeader
            borderless={!detailHeader}
            left={
              detailHeader ? (
                <>
                  <HeaderIconBadge>
                    <detailHeader.Icon
                      size={theme.iconSize.md}
                      color={theme.colors.foregroundMuted}
                    />
                  </HeaderIconBadge>
                  <ScreenTitle testID="settings-detail-header-title">
                    {detailHeader.title}
                  </ScreenTitle>
                  {detailHeader.titleAccessory}
                </>
              ) : null
            }
            leftStyle={desktopStyles.detailLeft}
          />
          <ScrollView
            style={styles.scrollView}
            contentContainerStyle={{ paddingBottom: insets.bottom }}
          >
            <View style={styles.content}>{renderedContent}</View>
          </ScrollView>
        </View>
      </View>
      {addHostModals}
    </View>
  );
}

// ---------------------------------------------------------------------------
// Styles
// ---------------------------------------------------------------------------

const styles = StyleSheet.create((theme) => ({
  loadingContainer: {
    flex: 1,
    backgroundColor: theme.colors.surface0,
    alignItems: "center",
    justifyContent: "center",
  },
  loadingText: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.lg,
  },
  container: {
    flex: 1,
    backgroundColor: theme.colors.surface0,
  },
  scrollView: {
    flex: 1,
  },
  content: {
    padding: theme.spacing[4],
    paddingTop: theme.spacing[6],
    width: "100%",
    maxWidth: 720,
    alignSelf: "center",
  },
  aboutValue: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
  },
  aboutErrorText: {
    color: theme.colors.palette.red[300],
    fontSize: theme.fontSize.xs,
    marginTop: theme.spacing[1],
  },
  aboutUpdateActions: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
  },
  dangerText: {
    color: theme.colors.destructive,
  },
  themeTrigger: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[1],
    paddingVertical: theme.spacing[1],
    paddingHorizontal: theme.spacing[2],
    borderRadius: theme.borderRadius.md,
    borderWidth: 1,
    borderColor: theme.colors.border,
  },
  themeTriggerText: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
  },
}));

const desktopStyles = StyleSheet.create((theme) => ({
  row: {
    flex: 1,
    flexDirection: "row",
  },
  contentPane: {
    flex: 1,
  },
  detailLeft: {
    gap: theme.spacing[2],
  },
}));

const sidebarStyles = StyleSheet.create((theme) => ({
  desktopContainer: {
    width: 320,
    borderRightWidth: 1,
    borderRightColor: theme.colors.border,
    backgroundColor: theme.colors.surfaceSidebar,
  },
  mobileContainer: {
    paddingVertical: theme.spacing[2],
    paddingHorizontal: theme.spacing[2],
  },
  list: {
    paddingVertical: theme.spacing[2],
    paddingHorizontal: theme.spacing[2],
    gap: theme.spacing[1],
  },
  item: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    minHeight: 36,
    paddingVertical: theme.spacing[2],
    paddingHorizontal: theme.spacing[2],
    borderRadius: theme.borderRadius.lg,
  },
  itemHovered: {
    backgroundColor: theme.colors.surfaceSidebarHover,
  },
  itemSelected: {
    backgroundColor: theme.colors.surfaceSidebarHover,
  },
  label: {
    fontSize: theme.fontSize.base,
    color: theme.colors.foregroundMuted,
    fontWeight: theme.fontWeight.normal,
    flex: 1,
  },
  localMarker: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
    paddingHorizontal: theme.spacing[2],
    paddingVertical: 2,
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.full,
    backgroundColor: theme.colors.surface3,
  },
}));

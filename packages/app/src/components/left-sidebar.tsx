import {
  memo,
  useCallback,
  useMemo,
  useState,
  useEffect,
  useRef,
  type ComponentType,
  type Dispatch,
  type ReactElement,
  type RefObject,
  type SetStateAction,
} from "react";
import {
  View,
  Pressable,
  Text,
  Modal,
  ScrollView,
  useWindowDimensions,
  StyleSheet as RNStyleSheet,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import Animated, {
  useAnimatedStyle,
  interpolate,
  Extrapolation,
  runOnJS,
  useSharedValue,
} from "react-native-reanimated";
import { Gesture, GestureDetector } from "react-native-gesture-handler";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import {
  ArrowDownNarrowWide,
  Box,
  Check,
  ChevronsDownUp,
  Cloud,
  Folder,
  FolderPlus,
  MessageSquarePlus,
  Settings,
  X,
} from "lucide-react-native";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
} from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Shortcut } from "@/components/ui/shortcut";
import { useShortcutKeys } from "@/hooks/use-shortcut-keys";
import { router, usePathname } from "expo-router";
import {
  usePanelStore,
  selectIsAgentListOpen,
  MIN_SIDEBAR_WIDTH,
  MAX_SIDEBAR_WIDTH,
} from "@/stores/panel-store";
import { useSidebarCollapsedSectionsStore } from "@/stores/sidebar-collapsed-sections-store";
import { useSidebarSortStore, type SidebarSortMode } from "@/stores/sidebar-sort-store";
import { SidebarWorkspaceList } from "./sidebar-workspace-list";
import { SidebarAgentListSkeleton } from "./sidebar-agent-list-skeleton";
import { useSidebarShortcutModel } from "@/hooks/use-sidebar-shortcut-model";
import {
  useSidebarWorkspacesList,
  type SidebarProjectEntry,
} from "@/hooks/use-sidebar-workspaces-list";
import { useSidebarAnimation } from "@/contexts/sidebar-animation-context";
import { useWindowControlsPadding } from "@/utils/desktop-window";
import { TitlebarDragRegion } from "@/components/desktop/titlebar-drag-region";
import { Combobox, ComboboxItem, type ComboboxOption } from "@/components/ui/combobox";
import { useHostRuntimeSnapshot, useHosts } from "@/runtime/host-runtime";
import { formatConnectionStatus } from "@/utils/daemons";
import { useIsCompactFormFactor } from "@/constants/layout";
import { buildPaseoCloudRoute, buildSettingsRoute, mapPathnameToServer } from "@/utils/host-routes";
import { useOpenProjectPicker } from "@/hooks/use-open-project-picker";
import { isWeb } from "@/constants/platform";
import { resolveActiveHost } from "@/utils/active-host";
import { SidebarUserMenu } from "@/components/sidebar-user-menu";
import { CLOUD_NAME } from "@/config/branding";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";
import { OnboardingGuideTarget } from "@/components/onboarding-guide-target";
import { generateDraftId } from "@/stores/draft-keys";
import { useSessionStore } from "@/stores/session-store";
import { buildNewAgentRoute, resolveNewChatTarget } from "@/utils/new-agent-routing";
import { navigateToPreparedWorkspaceTab } from "@/utils/workspace-navigation";
import { WORKTREE_PERSONA_ROLES, WORKTREE_PERSONA_SKILLS } from "@server/shared/worktree-persona";

const MIN_CHAT_WIDTH = 400;

type SidebarShortcutModel = ReturnType<typeof useSidebarShortcutModel>;
type SidebarTheme = ReturnType<typeof useUnistyles>["theme"];
type SidebarText = ReturnType<typeof getSub2APIMessages>["sidebar"];

interface LeftSidebarProps {
  selectedAgentId?: string;
}

interface SidebarSharedProps {
  theme: SidebarTheme;
  activeServerId: string | null;
  activeHostLabel: string;
  activeHostStatusColor: string;
  hostOptions: ComboboxOption[];
  hostTriggerRef: RefObject<View | null>;
  isHostPickerOpen: boolean;
  setIsHostPickerOpen: Dispatch<SetStateAction<boolean>>;
  projects: SidebarProjectEntry[];
  isInitialLoad: boolean;
  isRevalidating: boolean;
  isManualRefresh: boolean;
  collapsedProjectKeys: SidebarShortcutModel["collapsedProjectKeys"];
  shortcutIndexByWorkspaceKey: SidebarShortcutModel["shortcutIndexByWorkspaceKey"];
  toggleProjectCollapsed: SidebarShortcutModel["toggleProjectCollapsed"];
  handleToggleCollapseAll: () => void;
  sortMode: SidebarSortMode;
  setSortMode: (mode: SidebarSortMode) => void;
  handleRefresh: () => void;
  handleHostSelect: (nextServerId: string) => void;
  handleNewChat: () => void;
  handleOpenSkills: () => void;
  handleOpenProject: () => void;
  handlePaseoCloud: () => void;
  handleSettings: () => void;
  renderHostOption: (input: {
    option: ComboboxOption;
    selected: boolean;
    active: boolean;
    onPress: () => void;
  }) => ReactElement;
  text: SidebarText;
}

interface MobileSidebarProps extends SidebarSharedProps {
  insetsTop: number;
  insetsBottom: number;
  isOpen: boolean;
  closeToAgent: () => void;
}

interface DesktopSidebarProps extends SidebarSharedProps {
  insetsTop: number;
  isOpen: boolean;
}

export const LeftSidebar = memo(function LeftSidebar({ selectedAgentId }: LeftSidebarProps) {
  const { theme } = useUnistyles();
  const locale = useSub2APILocale();
  const sidebarText = useMemo(() => getSub2APIMessages(locale).sidebar, [locale]);
  const insets = useSafeAreaInsets();
  const isCompactLayout = useIsCompactFormFactor();
  const isOpen = usePanelStore((state) =>
    selectIsAgentListOpen(state, { isCompact: isCompactLayout }),
  );
  const showMobileAgent = usePanelStore((state) => state.showMobileAgent);
  const pathname = usePathname();
  const daemons = useHosts();
  const activeDaemon = useMemo(
    () => resolveActiveHost({ hosts: daemons, pathname }),
    [daemons, pathname],
  );
  const activeServerId = activeDaemon?.serverId ?? null;
  const activeHostLabel = useMemo(() => {
    if (!activeDaemon) return sidebarText.noHost;
    const trimmed = activeDaemon.label?.trim();
    return trimmed && trimmed.length > 0 ? trimmed : activeDaemon.serverId;
  }, [activeDaemon, sidebarText.noHost]);
  const activeHostSnapshot = useHostRuntimeSnapshot(activeServerId ?? "");
  const activeHostStatus = activeServerId
    ? (activeHostSnapshot?.connectionStatus ?? "connecting")
    : "idle";
  const activeHostStatusColor =
    activeHostStatus === "online"
      ? theme.colors.palette.green[400]
      : activeHostStatus === "connecting"
        ? theme.colors.palette.amber[500]
        : theme.colors.palette.red[500];
  const hostOptions = useMemo(
    () =>
      daemons.map((daemon) => ({
        id: daemon.serverId,
        label: daemon.label?.trim() || daemon.serverId,
      })),
    [daemons],
  );
  const renderHostOption = useCallback(
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
    }) => (
      <HostSwitchOption
        serverId={option.id}
        label={option.label}
        selected={selected}
        active={active}
        onPress={onPress}
      />
    ),
    [],
  );
  const hostTriggerRef = useRef<View | null>(null);
  const [isHostPickerOpen, setIsHostPickerOpen] = useState(false);
  const [isSkillLibraryOpen, setIsSkillLibraryOpen] = useState(false);

  const { projects, isInitialLoad, isRevalidating, refreshAll } = useSidebarWorkspacesList({
    serverId: activeServerId,
    enabled: isCompactLayout || isOpen,
  });
  const { collapsedProjectKeys, shortcutIndexByWorkspaceKey, toggleProjectCollapsed } =
    useSidebarShortcutModel({ projects, isInitialLoad });

  const [isManualRefresh, setIsManualRefresh] = useState(false);

  const handleRefresh = useCallback(() => {
    setIsManualRefresh(true);
    refreshAll();
  }, [refreshAll]);

  useEffect(() => {
    if (!isRevalidating && isManualRefresh) {
      setIsManualRefresh(false);
    }
  }, [isRevalidating, isManualRefresh]);

  const openProjectPicker = useOpenProjectPicker(activeServerId);

  const handleOpenProjectMobile = useCallback(() => {
    showMobileAgent();
    void openProjectPicker();
  }, [showMobileAgent, openProjectPicker]);

  const handleOpenProjectDesktop = useCallback(() => {
    void openProjectPicker();
  }, [openProjectPicker]);

  const handlePaseoCloudMobile = useCallback(() => {
    showMobileAgent();
    router.push(buildPaseoCloudRoute());
  }, [showMobileAgent]);

  const handlePaseoCloudDesktop = useCallback(() => {
    router.push(buildPaseoCloudRoute());
  }, []);

  const handleSettingsMobile = useCallback(() => {
    showMobileAgent();
    router.push(buildSettingsRoute());
  }, [showMobileAgent]);

  const handleSettingsDesktop = useCallback(() => {
    router.push(buildSettingsRoute());
  }, []);

  const handleNewChat = useCallback(() => {
    const target = resolveNewChatTarget({
      pathname,
      selectedAgentId,
      activeServerId,
      getAgent: (serverId, agentId) =>
        useSessionStore.getState().sessions[serverId]?.agents.get(agentId) ?? null,
      getWorkspaces: (serverId) =>
        useSessionStore.getState().sessions[serverId]?.workspaces.values() ?? null,
    });

    if (target.kind === "workspace") {
      navigateToPreparedWorkspaceTab({
        serverId: target.serverId,
        workspaceId: target.workspaceId,
        target: { kind: "draft", draftId: generateDraftId() },
        navigationMethod: "navigate",
      });
      return;
    }

    if (target.serverId) {
      router.push(buildNewAgentRoute(target.serverId, target.workingDir) as any);
      return;
    }

    void openProjectPicker();
  }, [activeServerId, openProjectPicker, pathname, selectedAgentId]);

  const handleNewChatMobile = useCallback(() => {
    showMobileAgent();
    handleNewChat();
  }, [handleNewChat, showMobileAgent]);

  const handleOpenSkills = useCallback(() => {
    setIsSkillLibraryOpen(true);
  }, []);

  const handleHostSelect = useCallback(
    (nextServerId: string) => {
      if (!nextServerId) {
        return;
      }
      const nextPath = mapPathnameToServer(pathname, nextServerId);
      setIsHostPickerOpen(false);
      router.push(nextPath);
    },
    [pathname],
  );

  const collapseAllProjects = useSidebarCollapsedSectionsStore(
    (state) => state.collapseAllProjects,
  );
  const expandAllProjects = useSidebarCollapsedSectionsStore((state) => state.expandAllProjects);

  const handleToggleCollapseAll = useCallback(() => {
    const allKeys = projects.map((p) => p.projectKey);
    const allCollapsed = allKeys.length > 0 && allKeys.every((k) => collapsedProjectKeys.has(k));
    if (allCollapsed) {
      expandAllProjects();
    } else {
      collapseAllProjects(allKeys);
    }
  }, [projects, collapsedProjectKeys, collapseAllProjects, expandAllProjects]);

  const sortMode = useSidebarSortStore((state) => state.sortMode);
  const setSortMode = useSidebarSortStore((state) => state.setSortMode);

  const sharedProps = {
    theme,
    activeServerId,
    activeHostLabel,
    activeHostStatusColor,
    hostOptions,
    hostTriggerRef,
    isHostPickerOpen,
    setIsHostPickerOpen,
    projects,
    isInitialLoad,
    isRevalidating,
    isManualRefresh,
    collapsedProjectKeys,
    shortcutIndexByWorkspaceKey,
    toggleProjectCollapsed,
    handleToggleCollapseAll,
    sortMode,
    setSortMode,
    handleRefresh,
    handleHostSelect,
    renderHostOption,
    handleOpenSkills,
    text: sidebarText,
  };

  if (isCompactLayout) {
    return (
      <>
        <MobileSidebar
          {...sharedProps}
          insetsTop={insets.top}
          insetsBottom={insets.bottom}
          isOpen={isOpen}
          closeToAgent={showMobileAgent}
          handleNewChat={handleNewChatMobile}
          handleOpenProject={handleOpenProjectMobile}
          handlePaseoCloud={handlePaseoCloudMobile}
          handleSettings={handleSettingsMobile}
        />
        <SkillLibrarySheet
          visible={isSkillLibraryOpen}
          onClose={() => setIsSkillLibraryOpen(false)}
        />
      </>
    );
  }

  return (
    <>
      <DesktopSidebar
        {...sharedProps}
        insetsTop={insets.top}
        isOpen={isOpen}
        handleNewChat={handleNewChat}
        handleOpenProject={handleOpenProjectDesktop}
        handlePaseoCloud={handlePaseoCloudDesktop}
        handleSettings={handleSettingsDesktop}
      />
      <SkillLibrarySheet
        visible={isSkillLibraryOpen}
        onClose={() => setIsSkillLibraryOpen(false)}
      />
    </>
  );
});

function SortFilterDropdown({
  theme,
  sortMode,
  setSortMode,
  text,
}: {
  theme: SidebarTheme;
  sortMode: SidebarSortMode;
  setSortMode: (mode: SidebarSortMode) => void;
  text: SidebarText;
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        style={({ hovered = false }) => [
          styles.headerIconButton,
          hovered && styles.headerIconButtonHovered,
        ]}
        accessibilityRole="button"
        accessibilityLabel={text.sort}
      >
        {({ hovered }) => (
          <ArrowDownNarrowWide
            size={theme.iconSize.md}
            color={hovered ? theme.colors.foreground : theme.colors.foregroundMuted}
          />
        )}
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" width={180}>
        <DropdownMenuItem
          leading={
            sortMode === "project" ? (
              <Check size={14} color={theme.colors.foreground} />
            ) : (
              <View style={{ width: 14 }} />
            )
          }
          onSelect={() => setSortMode("project")}
        >
          {text.sortByProject}
        </DropdownMenuItem>
        <DropdownMenuItem
          leading={
            sortMode === "time" ? (
              <Check size={14} color={theme.colors.foreground} />
            ) : (
              <View style={{ width: 14 }} />
            )
          }
          onSelect={() => setSortMode("time")}
        >
          {text.sortByTime}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function HostSwitchOption({
  serverId,
  label,
  selected,
  active,
  onPress,
}: {
  serverId: string;
  label: string;
  selected: boolean;
  active: boolean;
  onPress: () => void;
}) {
  const snapshot = useHostRuntimeSnapshot(serverId);
  const connectionStatus = snapshot?.connectionStatus ?? "connecting";

  return (
    <ComboboxItem
      label={label}
      description={formatConnectionStatus(connectionStatus)}
      selected={selected}
      active={active}
      onPress={onPress}
    />
  );
}

function SidebarTopAction({
  icon: Icon,
  label,
  onPress,
  testID,
}: {
  icon: ComponentType<{ size?: number; color?: string }>;
  label: string;
  onPress: () => void;
  testID: string;
}) {
  const { theme } = useUnistyles();

  return (
    <Pressable
      style={({ hovered = false, pressed = false }) => [
        styles.sidebarPrimaryAction,
        (hovered || pressed) && styles.sidebarPrimaryActionHovered,
      ]}
      accessibilityLabel={label}
      accessibilityRole="button"
      onPress={onPress}
      testID={testID}
    >
      {({ hovered, pressed }) => {
        const active = hovered || pressed;
        return (
          <>
            <Icon
              size={theme.iconSize.md}
              color={active ? theme.colors.foreground : theme.colors.foregroundMuted}
            />
            <Text style={styles.sidebarPrimaryActionText} numberOfLines={1}>
              {label}
            </Text>
          </>
        );
      }}
    </Pressable>
  );
}

function SkillLibrarySheet({ visible, onClose }: { visible: boolean; onClose: () => void }) {
  const { theme } = useUnistyles();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).sidebar, [locale]);
  const isZh = locale === "zh";

  const skills = useMemo(
    () =>
      WORKTREE_PERSONA_SKILLS.map((skill) => ({
        skill,
        roles: WORKTREE_PERSONA_ROLES.filter((role) => role.defaultSkillIds.includes(skill.id)),
      })),
    [],
  );

  return (
    <Modal transparent animationType="fade" visible={visible} onRequestClose={onClose}>
      <View style={styles.skillLibraryOverlay} testID="skill-library-panel">
        <Pressable style={staticStyles.modalBackdrop} onPress={onClose} />
        <View style={styles.skillLibrarySheet}>
          <View style={styles.skillLibraryHeader}>
            <View style={styles.skillLibraryHeaderText}>
              <Text style={styles.skillLibraryPanelTitle}>{text.skillLibraryTitle}</Text>
              <Text style={styles.skillLibrarySubtitle}>
                {text.skillLibrarySubtitle(WORKTREE_PERSONA_SKILLS.length)}
              </Text>
            </View>
            <Pressable
              accessibilityLabel={text.closeSkillLibrary}
              accessibilityRole="button"
              style={styles.skillLibraryCloseButton}
              onPress={onClose}
            >
              <X size={16} color={theme.colors.foregroundMuted} />
            </Pressable>
          </View>
          <ScrollView
            style={styles.skillLibraryBody}
            contentContainerStyle={styles.skillLibraryList}
            showsVerticalScrollIndicator={false}
          >
            {skills.map(({ skill, roles }) => (
              <View key={skill.id} style={styles.skillLibraryCard}>
                <View style={styles.skillLibraryCardHeader}>
                  <View style={styles.skillLibraryIcon}>
                    <Box size={16} color={theme.colors.accent} />
                  </View>
                  <View style={styles.skillLibraryTitleGroup}>
                    <View style={styles.skillLibraryNameRow}>
                      <Text style={styles.skillLibraryName} numberOfLines={1}>
                        {skill.name}
                      </Text>
                      <View style={styles.skillLibraryTag}>
                        <Text style={styles.skillLibraryTagText}>{text.skillBuiltIn}</Text>
                      </View>
                    </View>
                    <Text style={styles.skillLibraryDescription}>{skill.description}</Text>
                  </View>
                </View>

                <View style={styles.skillLibraryRoleRow}>
                  <Text style={styles.skillLibraryMetaLabel}>{text.skillDefaultRoles}</Text>
                  <View style={styles.skillLibraryBadges}>
                    {roles.map((role) => (
                      <View key={role.id} style={styles.skillLibraryRoleBadge}>
                        <Text style={styles.skillLibraryRoleBadgeText}>
                          {isZh ? role.labelZh : role.label}
                        </Text>
                      </View>
                    ))}
                  </View>
                </View>

                <View style={styles.skillLibraryMetaGrid}>
                  <View style={styles.skillLibraryMetaItem}>
                    <Text style={styles.skillLibraryMetaLabel}>{text.skillSource}</Text>
                    <Text style={styles.skillLibraryMetaValue} numberOfLines={1}>
                      {skill.sourceUrl}
                    </Text>
                  </View>
                  <View style={styles.skillLibraryMetaItem}>
                    <Text style={styles.skillLibraryMetaLabel}>{text.skillLicense}</Text>
                    <Text style={styles.skillLibraryMetaValue} numberOfLines={1}>
                      {skill.license}
                    </Text>
                  </View>
                </View>
              </View>
            ))}
          </ScrollView>
        </View>
      </View>
    </Modal>
  );
}

function MobileSidebar({
  theme,
  activeServerId,
  activeHostLabel,
  activeHostStatusColor,
  hostOptions,
  hostTriggerRef,
  isHostPickerOpen,
  setIsHostPickerOpen,
  projects,
  isInitialLoad,
  isRevalidating,
  isManualRefresh,
  collapsedProjectKeys,
  shortcutIndexByWorkspaceKey,
  toggleProjectCollapsed,
  handleToggleCollapseAll,
  sortMode,
  setSortMode,
  handleRefresh,
  handleHostSelect,
  renderHostOption,
  text,
  handleNewChat,
  handleOpenSkills,
  handleOpenProject,
  handlePaseoCloud,
  handleSettings,
  insetsTop,
  insetsBottom,
  isOpen,
  closeToAgent,
}: MobileSidebarProps) {
  const newAgentKeys = useShortcutKeys("new-agent");
  const {
    translateX,
    backdropOpacity,
    windowWidth,
    animateToOpen,
    animateToClose,
    isGesturing,
    gestureAnimatingRef,
    closeGestureRef,
  } = useSidebarAnimation();
  const closeTouchStartX = useSharedValue(0);
  const closeTouchStartY = useSharedValue(0);

  const handleCloseFromGesture = useCallback(() => {
    gestureAnimatingRef.current = true;
    closeToAgent();
  }, [closeToAgent, gestureAnimatingRef]);

  const closeGesture = useMemo(
    () =>
      Gesture.Pan()
        .withRef(closeGestureRef)
        .enabled(isOpen)
        .manualActivation(true)
        .onTouchesDown((event) => {
          const touch = event.changedTouches[0];
          if (!touch) {
            return;
          }
          closeTouchStartX.value = touch.absoluteX;
          closeTouchStartY.value = touch.absoluteY;
        })
        .onTouchesMove((event, stateManager) => {
          const touch = event.changedTouches[0];
          if (!touch || event.numberOfTouches !== 1) {
            stateManager.fail();
            return;
          }

          const deltaX = touch.absoluteX - closeTouchStartX.value;
          const deltaY = touch.absoluteY - closeTouchStartY.value;
          const absDeltaX = Math.abs(deltaX);
          const absDeltaY = Math.abs(deltaY);

          if (deltaX >= 10) {
            stateManager.fail();
            return;
          }
          if (absDeltaY > 10 && absDeltaY > absDeltaX) {
            stateManager.fail();
            return;
          }
          if (deltaX <= -15 && absDeltaX > absDeltaY) {
            stateManager.activate();
          }
        })
        .onStart(() => {
          isGesturing.value = true;
        })
        .onUpdate((event) => {
          const newTranslateX = Math.min(0, Math.max(-windowWidth, event.translationX));
          translateX.value = newTranslateX;
          backdropOpacity.value = interpolate(
            newTranslateX,
            [-windowWidth, 0],
            [0, 1],
            Extrapolation.CLAMP,
          );
        })
        .onEnd((event) => {
          isGesturing.value = false;
          const shouldClose = event.translationX < -windowWidth / 3 || event.velocityX < -500;
          if (shouldClose) {
            animateToClose();
            runOnJS(handleCloseFromGesture)();
          } else {
            animateToOpen();
          }
        })
        .onFinalize(() => {
          isGesturing.value = false;
        }),
    [
      isOpen,
      closeGestureRef,
      closeTouchStartX,
      closeTouchStartY,
      isGesturing,
      windowWidth,
      translateX,
      backdropOpacity,
      animateToClose,
      animateToOpen,
      handleCloseFromGesture,
    ],
  );

  const mobileSidebarInsetStyle = useMemo(
    () => ({
      width: windowWidth,
      paddingTop: insetsTop,
      paddingBottom: insetsBottom,
    }),
    [windowWidth, insetsTop, insetsBottom],
  );

  const hostStatusDotStyle = useMemo(
    () => [styles.hostStatusDot, { backgroundColor: activeHostStatusColor }],
    [activeHostStatusColor],
  );

  const sidebarAnimatedStyle = useAnimatedStyle(() => ({
    transform: [{ translateX: translateX.value }],
  }));

  const backdropAnimatedStyle = useAnimatedStyle(() => ({
    opacity: backdropOpacity.value,
    pointerEvents: backdropOpacity.value > 0.01 ? "auto" : "none",
  }));

  const overlayPointerEvents = isWeb ? (isOpen ? "auto" : "none") : "box-none";

  return (
    <View style={StyleSheet.absoluteFillObject} pointerEvents={overlayPointerEvents}>
      <Animated.View style={[staticStyles.backdrop, backdropAnimatedStyle]} />

      <GestureDetector gesture={closeGesture} touchAction="pan-y">
        <Animated.View
          style={[
            staticStyles.mobileSidebar,
            mobileSidebarInsetStyle,
            sidebarAnimatedStyle,
            { backgroundColor: theme.colors.surfaceSidebar },
          ]}
          pointerEvents="auto"
        >
          <View style={styles.sidebarContent} pointerEvents="auto">
            <View style={styles.sidebarTopActions}>
              <SidebarTopAction
                icon={MessageSquarePlus}
                label={text.newChat}
                onPress={handleNewChat}
                testID="sidebar-new-chat"
              />
              <SidebarTopAction
                icon={Box}
                label={text.skills}
                onPress={handleOpenSkills}
                testID="sidebar-skill-library"
              />
            </View>

            <OnboardingGuideTarget id="sidebar.projects" style={styles.sidebarHeader}>
              <View style={styles.sidebarHeaderTitleRow}>
                <Folder size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
                <Text style={styles.sidebarHeaderTitle}>{text.projects}</Text>
              </View>
              <View style={styles.sidebarHeaderIcons}>
                <Pressable
                  style={styles.headerIconButton}
                  accessibilityLabel={text.collapseAllProjects}
                  accessibilityRole="button"
                  onPress={handleToggleCollapseAll}
                >
                  {({ hovered }) => (
                    <ChevronsDownUp
                      size={theme.iconSize.md}
                      color={hovered ? theme.colors.foreground : theme.colors.foregroundMuted}
                    />
                  )}
                </Pressable>
                <SortFilterDropdown
                  theme={theme}
                  sortMode={sortMode}
                  setSortMode={setSortMode}
                  text={text}
                />
                <OnboardingGuideTarget id="sidebar.projectAdd">
                  <Tooltip delayDuration={300}>
                    <TooltipTrigger asChild>
                      <Pressable
                        style={styles.headerIconButton}
                        accessibilityLabel={text.addProject}
                        accessibilityRole="button"
                        onPress={handleOpenProject}
                        testID="sidebar-add-project"
                      >
                        {({ hovered }) => (
                          <FolderPlus
                            size={theme.iconSize.md}
                            color={hovered ? theme.colors.foreground : theme.colors.foregroundMuted}
                          />
                        )}
                      </Pressable>
                    </TooltipTrigger>
                    <TooltipContent side="bottom" align="center" offset={8}>
                      <View style={styles.tooltipRow}>
                        <Text style={styles.tooltipText}>{text.addProject}</Text>
                        {newAgentKeys ? <Shortcut chord={newAgentKeys} /> : null}
                      </View>
                    </TooltipContent>
                  </Tooltip>
                </OnboardingGuideTarget>
              </View>
            </OnboardingGuideTarget>

            {isInitialLoad ? (
              <SidebarAgentListSkeleton />
            ) : (
              <SidebarWorkspaceList
                serverId={activeServerId}
                collapsedProjectKeys={collapsedProjectKeys}
                onToggleProjectCollapsed={toggleProjectCollapsed}
                shortcutIndexByWorkspaceKey={shortcutIndexByWorkspaceKey}
                projects={projects}
                isRefreshing={isManualRefresh && isRevalidating}
                onRefresh={handleRefresh}
                onWorkspacePress={() => closeToAgent()}
                onAddProject={handleOpenProject}
                parentGestureRef={closeGestureRef}
              />
            )}

            <View style={styles.sidebarFooter}>
              <View style={styles.footerHostSlot}>
                <Pressable
                  ref={hostTriggerRef}
                  style={({ hovered = false }) => [
                    styles.hostTrigger,
                    hovered && styles.hostTriggerHovered,
                  ]}
                  onPress={() => setIsHostPickerOpen(true)}
                  disabled={hostOptions.length === 0}
                >
                  <View style={hostStatusDotStyle} />
                  <Text style={styles.hostTriggerText} numberOfLines={1}>
                    {activeHostLabel}
                  </Text>
                </Pressable>
              </View>
              <View style={styles.footerIconRow}>
                <Pressable
                  style={styles.footerIconButton}
                  testID="sidebar-paseo-cloud"
                  nativeID="sidebar-paseo-cloud"
                  collapsable={false}
                  accessible
                  accessibilityLabel={CLOUD_NAME}
                  accessibilityRole="button"
                  onPress={handlePaseoCloud}
                >
                  {({ hovered }) => (
                    <Cloud
                      size={theme.iconSize.md}
                      color={hovered ? theme.colors.foreground : theme.colors.foregroundMuted}
                    />
                  )}
                </Pressable>
                <Pressable
                  style={styles.footerIconButton}
                  testID="sidebar-settings"
                  nativeID="sidebar-settings"
                  collapsable={false}
                  accessible
                  accessibilityLabel={text.settings}
                  accessibilityRole="button"
                  onPress={handleSettings}
                >
                  {({ hovered }) => (
                    <Settings
                      size={theme.iconSize.md}
                      color={hovered ? theme.colors.foreground : theme.colors.foregroundMuted}
                    />
                  )}
                </Pressable>
                <SidebarUserMenu onNavigateSettings={handleSettings} />
              </View>
              <Combobox
                options={hostOptions}
                value={activeServerId ?? ""}
                onSelect={handleHostSelect}
                renderOption={renderHostOption}
                searchable={false}
                title={text.switchHost}
                searchPlaceholder={text.searchHosts}
                open={isHostPickerOpen}
                onOpenChange={setIsHostPickerOpen}
                anchorRef={hostTriggerRef}
              />
            </View>
          </View>
        </Animated.View>
      </GestureDetector>
    </View>
  );
}

function DesktopSidebar({
  theme,
  activeServerId,
  activeHostLabel,
  activeHostStatusColor,
  hostOptions,
  hostTriggerRef,
  isHostPickerOpen,
  setIsHostPickerOpen,
  projects,
  isInitialLoad,
  isRevalidating,
  isManualRefresh,
  collapsedProjectKeys,
  shortcutIndexByWorkspaceKey,
  toggleProjectCollapsed,
  handleToggleCollapseAll,
  sortMode,
  setSortMode,
  handleRefresh,
  handleHostSelect,
  renderHostOption,
  text,
  handleNewChat,
  handleOpenSkills,
  handleOpenProject,
  handlePaseoCloud,
  handleSettings,
  insetsTop,
  isOpen,
}: DesktopSidebarProps) {
  const newAgentKeys = useShortcutKeys("new-agent");
  const padding = useWindowControlsPadding("sidebar");
  const sidebarWidth = usePanelStore((state) => state.sidebarWidth);
  const setSidebarWidth = usePanelStore((state) => state.setSidebarWidth);
  const { width: viewportWidth } = useWindowDimensions();
  const hostStatusDotStyle = useMemo(
    () => [styles.hostStatusDot, { backgroundColor: activeHostStatusColor }],
    [activeHostStatusColor],
  );

  const startWidthRef = useRef(sidebarWidth);
  const resizeWidth = useSharedValue(sidebarWidth);

  useEffect(() => {
    resizeWidth.value = sidebarWidth;
  }, [sidebarWidth, resizeWidth]);

  const resizeGesture = useMemo(
    () =>
      Gesture.Pan()
        .hitSlop({ left: 8, right: 8, top: 0, bottom: 0 })
        .onStart(() => {
          startWidthRef.current = sidebarWidth;
          resizeWidth.value = sidebarWidth;
        })
        .onUpdate((event) => {
          // Dragging right (positive translationX) increases width
          const newWidth = startWidthRef.current + event.translationX;
          const maxWidth = Math.max(
            MIN_SIDEBAR_WIDTH,
            Math.min(MAX_SIDEBAR_WIDTH, viewportWidth - MIN_CHAT_WIDTH),
          );
          const clampedWidth = Math.max(MIN_SIDEBAR_WIDTH, Math.min(maxWidth, newWidth));
          resizeWidth.value = clampedWidth;
        })
        .onEnd(() => {
          runOnJS(setSidebarWidth)(resizeWidth.value);
        }),
    [sidebarWidth, resizeWidth, setSidebarWidth, viewportWidth],
  );

  const resizeAnimatedStyle = useAnimatedStyle(() => ({
    width: resizeWidth.value,
  }));

  if (!isOpen) {
    return null;
  }

  return (
    <Animated.View
      style={[staticStyles.desktopSidebar, resizeAnimatedStyle, { paddingTop: insetsTop }]}
    >
      <View style={[styles.desktopSidebarBorder, { flex: 1 }]}>
        <View style={styles.sidebarDragArea}>
          <TitlebarDragRegion />
          {padding.top > 0 ? <View style={{ height: padding.top }} /> : null}
          <View style={styles.sidebarTopActions}>
            <SidebarTopAction
              icon={MessageSquarePlus}
              label={text.newChat}
              onPress={handleNewChat}
              testID="sidebar-new-chat"
            />
            <SidebarTopAction
              icon={Box}
              label={text.skills}
              onPress={handleOpenSkills}
              testID="sidebar-skill-library"
            />
          </View>

          <OnboardingGuideTarget id="sidebar.projects" style={styles.sidebarHeader}>
            <View style={styles.sidebarHeaderTitleRow}>
              <Folder size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
              <Text style={styles.sidebarHeaderTitle}>{text.projects}</Text>
            </View>
            <View style={styles.sidebarHeaderIcons}>
              <Pressable
                style={styles.headerIconButton}
                accessibilityLabel={text.collapseAllProjects}
                accessibilityRole="button"
                onPress={handleToggleCollapseAll}
              >
                {({ hovered }) => (
                  <ChevronsDownUp
                    size={theme.iconSize.md}
                    color={hovered ? theme.colors.foreground : theme.colors.foregroundMuted}
                  />
                )}
              </Pressable>
              <SortFilterDropdown
                theme={theme}
                sortMode={sortMode}
                setSortMode={setSortMode}
                text={text}
              />
              <OnboardingGuideTarget id="sidebar.projectAdd">
                <Tooltip delayDuration={300}>
                  <TooltipTrigger asChild>
                    <Pressable
                      style={styles.headerIconButton}
                      accessibilityLabel={text.addProject}
                      accessibilityRole="button"
                      onPress={handleOpenProject}
                      testID="sidebar-add-project"
                    >
                      {({ hovered }) => (
                        <FolderPlus
                          size={theme.iconSize.md}
                          color={hovered ? theme.colors.foreground : theme.colors.foregroundMuted}
                        />
                      )}
                    </Pressable>
                  </TooltipTrigger>
                  <TooltipContent side="bottom" align="center" offset={8}>
                    <View style={styles.tooltipRow}>
                      <Text style={styles.tooltipText}>{text.addProject}</Text>
                      {newAgentKeys ? <Shortcut chord={newAgentKeys} /> : null}
                    </View>
                  </TooltipContent>
                </Tooltip>
              </OnboardingGuideTarget>
            </View>
          </OnboardingGuideTarget>
        </View>

        {isInitialLoad ? (
          <SidebarAgentListSkeleton />
        ) : (
          <SidebarWorkspaceList
            serverId={activeServerId}
            collapsedProjectKeys={collapsedProjectKeys}
            onToggleProjectCollapsed={toggleProjectCollapsed}
            shortcutIndexByWorkspaceKey={shortcutIndexByWorkspaceKey}
            projects={projects}
            isRefreshing={isManualRefresh && isRevalidating}
            onRefresh={handleRefresh}
            onAddProject={handleOpenProject}
          />
        )}

        <View style={styles.sidebarFooter}>
          <View style={styles.footerHostSlot}>
            <Pressable
              ref={hostTriggerRef}
              style={({ hovered = false }) => [
                styles.hostTrigger,
                hovered && styles.hostTriggerHovered,
              ]}
              onPress={() => setIsHostPickerOpen(true)}
              disabled={hostOptions.length === 0}
            >
              <View style={hostStatusDotStyle} />
              <Text style={styles.hostTriggerText} numberOfLines={1}>
                {activeHostLabel}
              </Text>
            </Pressable>
          </View>
          <View style={styles.footerIconRow}>
            <Pressable
              style={styles.footerIconButton}
              testID="sidebar-paseo-cloud"
              nativeID="sidebar-paseo-cloud"
              collapsable={false}
              accessible
              accessibilityLabel={CLOUD_NAME}
              accessibilityRole="button"
              onPress={handlePaseoCloud}
            >
              {({ hovered }) => (
                <Cloud
                  size={theme.iconSize.md}
                  color={hovered ? theme.colors.foreground : theme.colors.foregroundMuted}
                />
              )}
            </Pressable>
            <Pressable
              style={styles.footerIconButton}
              testID="sidebar-settings"
              nativeID="sidebar-settings"
              collapsable={false}
              accessible
              accessibilityLabel={text.settings}
              accessibilityRole="button"
              onPress={handleSettings}
            >
              {({ hovered }) => (
                <Settings
                  size={theme.iconSize.md}
                  color={hovered ? theme.colors.foreground : theme.colors.foregroundMuted}
                />
              )}
            </Pressable>
            <SidebarUserMenu onNavigateSettings={handleSettings} />
          </View>
          <Combobox
            options={hostOptions}
            value={activeServerId ?? ""}
            onSelect={handleHostSelect}
            renderOption={renderHostOption}
            searchable={false}
            title={text.switchHost}
            searchPlaceholder={text.searchHosts}
            open={isHostPickerOpen}
            onOpenChange={setIsHostPickerOpen}
            anchorRef={hostTriggerRef}
          />
        </View>

        {/* Resize handle - absolutely positioned over right border */}
        <GestureDetector gesture={resizeGesture}>
          <View style={[styles.resizeHandle, isWeb && ({ cursor: "col-resize" } as any)]} />
        </GestureDetector>
      </View>
    </Animated.View>
  );
}

// Static styles for Animated.Views — must NOT use Unistyles dynamic theme to
// avoid the "Unable to find node on an unmounted component" crash when Unistyles
// tries to patch the native node that Reanimated also manages.
const staticStyles = RNStyleSheet.create({
  backdrop: {
    ...RNStyleSheet.absoluteFillObject,
    backgroundColor: "rgba(0, 0, 0, 0.5)",
  },
  modalBackdrop: {
    ...RNStyleSheet.absoluteFillObject,
  },
  mobileSidebar: {
    position: "absolute" as const,
    top: 0,
    left: 0,
    bottom: 0,
    overflow: "hidden" as const,
  },
  desktopSidebar: {
    position: "relative" as const,
  },
});

const styles = StyleSheet.create((theme) => ({
  sidebarContent: {
    flex: 1,
    minHeight: 0,
  },
  desktopSidebarBorder: {
    borderRightWidth: 1,
    borderRightColor: theme.colors.border,
    backgroundColor: theme.colors.surfaceSidebar,
  },
  resizeHandle: {
    position: "absolute",
    right: -5,
    top: 0,
    bottom: 0,
    width: 10,
    zIndex: 10,
  },
  sidebarDragArea: {
    position: "relative",
  },
  sidebarTopActions: {
    paddingHorizontal: theme.spacing[3],
    paddingTop: theme.spacing[3],
    paddingBottom: theme.spacing[2],
    gap: theme.spacing[1],
  },
  sidebarPrimaryAction: {
    minHeight: 34,
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[3],
    paddingHorizontal: theme.spacing[2],
    paddingVertical: theme.spacing[2],
    borderRadius: theme.borderRadius.md,
    userSelect: "none",
  },
  sidebarPrimaryActionHovered: {
    backgroundColor: theme.colors.surfaceSidebarHover,
  },
  sidebarPrimaryActionText: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.base,
    fontWeight: theme.fontWeight.semibold,
    flexShrink: 1,
    minWidth: 0,
  },
  sidebarHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingLeft: theme.spacing[4],
    paddingRight: theme.spacing[3],
    paddingTop: theme.spacing[3],
    paddingBottom: theme.spacing[1],
    userSelect: "none",
  },
  sidebarHeaderTitleRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    flexShrink: 1,
    minWidth: 0,
  },
  sidebarHeaderTitle: {
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.semibold,
    color: theme.colors.foregroundMuted,
  },
  sidebarHeaderIcons: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[1],
  },
  headerIconButton: {
    width: 28,
    height: 28,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: theme.borderRadius.md,
  },
  headerIconButtonHovered: {
    backgroundColor: theme.colors.surfaceSidebarHover,
  },
  hostTrigger: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "flex-start",
    gap: theme.spacing[2],
    minWidth: 0,
    paddingVertical: theme.spacing[1],
    paddingHorizontal: theme.spacing[2],
    borderRadius: theme.borderRadius.lg,
  },
  hostTriggerHovered: {
    backgroundColor: theme.colors.surfaceSidebarHover,
  },
  hostStatusDot: {
    width: 8,
    height: 8,
    borderRadius: theme.borderRadius.full,
  },
  hostTriggerText: {
    fontSize: theme.fontSize.sm,
    color: theme.colors.foregroundMuted,
    flexShrink: 1,
    minWidth: 0,
  },
  sidebarFooter: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: theme.spacing[4],
    paddingVertical: theme.spacing[3],
    borderTopWidth: 1,
    borderTopColor: theme.colors.border,
  },
  footerHostSlot: {
    flexGrow: 0,
    flexShrink: 1,
    minWidth: 0,
    marginRight: theme.spacing[2],
  },
  footerIconRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    flexShrink: 0,
  },
  footerIconButton: {
    width: 28,
    height: 28,
    alignItems: "center",
    justifyContent: "center",
    paddingVertical: theme.spacing[1],
    paddingHorizontal: theme.spacing[1],
  },
  hostPickerList: {
    gap: theme.spacing[2],
  },
  hostPickerOption: {
    paddingVertical: theme.spacing[3],
    paddingHorizontal: theme.spacing[3],
    borderRadius: theme.borderRadius.lg,
    backgroundColor: theme.colors.surface2,
    borderWidth: theme.borderWidth[1],
    borderColor: theme.colors.border,
  },
  hostPickerOptionText: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
  },
  hostPickerCancel: {
    paddingVertical: theme.spacing[3],
    paddingHorizontal: theme.spacing[3],
    borderRadius: theme.borderRadius.lg,
    backgroundColor: theme.colors.surface0,
    borderWidth: theme.borderWidth[1],
    borderColor: theme.colors.border,
    alignItems: "center",
  },
  hostPickerCancelText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
  },
  tooltipRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
  },
  tooltipText: {
    fontSize: theme.fontSize.sm,
    color: theme.colors.popoverForeground,
  },
  skillLibraryOverlay: {
    ...StyleSheet.absoluteFillObject,
    alignItems: "center",
    justifyContent: "center",
    padding: theme.spacing[4],
    backgroundColor: "rgba(0,0,0,0.5)",
  },
  skillLibrarySheet: {
    width: "100%",
    maxWidth: 720,
    maxHeight: "88%",
    minHeight: 0,
    borderRadius: theme.borderRadius.lg,
    borderWidth: 1,
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.surface1,
    overflow: "hidden",
  },
  skillLibraryHeader: {
    flexDirection: "row",
    alignItems: "flex-start",
    justifyContent: "space-between",
    gap: theme.spacing[3],
    padding: theme.spacing[4],
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
  },
  skillLibraryHeaderText: {
    flex: 1,
    minWidth: 0,
    gap: theme.spacing[1],
  },
  skillLibraryPanelTitle: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.base,
    fontWeight: theme.fontWeight.semibold,
  },
  skillLibrarySubtitle: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
  },
  skillLibraryCloseButton: {
    width: 28,
    height: 28,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: theme.borderRadius.md,
    backgroundColor: theme.colors.surface2,
  },
  skillLibraryBody: {
    minHeight: 0,
  },
  skillLibraryList: {
    gap: theme.spacing[3],
    padding: theme.spacing[4],
  },
  skillLibraryCard: {
    gap: theme.spacing[3],
    padding: theme.spacing[4],
    borderRadius: theme.borderRadius.lg,
    borderWidth: 1,
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.surface1,
  },
  skillLibraryCardHeader: {
    flexDirection: "row",
    gap: theme.spacing[3],
    alignItems: "flex-start",
  },
  skillLibraryIcon: {
    width: 32,
    height: 32,
    borderRadius: theme.borderRadius.md,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: theme.colors.surface2,
  },
  skillLibraryTitleGroup: {
    flex: 1,
    minWidth: 0,
    gap: theme.spacing[1],
  },
  skillLibraryNameRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    minWidth: 0,
  },
  skillLibraryName: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.base,
    fontWeight: theme.fontWeight.semibold,
    flexShrink: 1,
    minWidth: 0,
  },
  skillLibraryDescription: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    lineHeight: 19,
  },
  skillLibraryTag: {
    paddingHorizontal: theme.spacing[2],
    paddingVertical: 2,
    borderRadius: theme.borderRadius.full,
    backgroundColor: theme.colors.surface2,
    flexShrink: 0,
  },
  skillLibraryTagText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.medium,
  },
  skillLibraryRoleRow: {
    gap: theme.spacing[2],
  },
  skillLibraryBadges: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: theme.spacing[2],
  },
  skillLibraryRoleBadge: {
    paddingHorizontal: theme.spacing[2],
    paddingVertical: theme.spacing[1],
    borderRadius: theme.borderRadius.md,
    backgroundColor: theme.colors.surface2,
  },
  skillLibraryRoleBadgeText: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.medium,
  },
  skillLibraryMetaGrid: {
    gap: theme.spacing[2],
  },
  skillLibraryMetaItem: {
    gap: theme.spacing[1],
  },
  skillLibraryMetaLabel: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.semibold,
    textTransform: "uppercase",
    letterSpacing: 0.5,
  },
  skillLibraryMetaValue: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.xs,
  },
}));

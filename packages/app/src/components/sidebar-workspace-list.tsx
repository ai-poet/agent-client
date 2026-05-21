import {
  View,
  Text,
  Pressable,
  Image,
  Platform,
  ActivityIndicator,
  StatusBar,
  ScrollView,
  type GestureResponderEvent,
} from "react-native";
import * as Haptics from "expo-haptics";
import { useMutation, useQueries } from "@tanstack/react-query";
import {
  useCallback,
  useMemo,
  useState,
  useEffect,
  useRef,
  type ReactElement,
  type MutableRefObject,
} from "react";
import { router, usePathname } from "expo-router";
import { navigateToWorkspace } from "@/hooks/use-workspace-navigation";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { type GestureType } from "react-native-gesture-handler";
import * as Clipboard from "expo-clipboard";
import { DiffStat } from "@/components/diff-stat";
import {
  Archive,
  CircleAlert,
  ChevronDown,
  ChevronRight,
  Copy,
  ExternalLink,
  FolderGit2,
  GitPullRequest,
  Globe,
  SquareTerminal,
  Monitor,
  MoreVertical,
  Pencil,
  Plus,
  Trash2,
} from "lucide-react-native";
import { NestableScrollContainer } from "react-native-draggable-flatlist";
import { DraggableList, type DraggableRenderItemInfo } from "./draggable-list";
import type { DraggableListDragHandleProps } from "./draggable-list.types";
import {
  getHostRuntimeStore,
  isHostRuntimeConnected,
  useHostRuntimeClient,
  useHostRuntimeIsConnected,
} from "@/runtime/host-runtime";
import { useIsCompactFormFactor } from "@/constants/layout";
import { projectIconQueryKey } from "@/hooks/use-project-icon-query";
import { parseHostWorkspaceRouteFromPathname } from "@/utils/host-routes";
import { prepareWorkspaceTab } from "@/utils/workspace-navigation";
import {
  createSidebarWorkspaceEntry,
  type SidebarProjectEntry,
  type SidebarWorkspaceEntry,
} from "@/hooks/use-sidebar-workspaces-list";
import { useSidebarOrderStore } from "@/stores/sidebar-order-store";
import { useSidebarProjectNamesStore } from "@/stores/sidebar-project-names-store";
import { useShowShortcutBadges } from "@/hooks/use-show-shortcut-badges";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
  useContextMenu,
} from "@/components/ui/context-menu";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
} from "@/components/ui/dropdown-menu";
import { SyncedLoader } from "@/components/synced-loader";
import { useToast } from "@/contexts/toast-context";
import { useCheckoutGitActionsStore } from "@/stores/checkout-git-actions-store";
import { hasVisibleOrderChanged, mergeWithRemainder } from "@/utils/sidebar-reorder";
import { decideLongPressMove } from "@/utils/sidebar-gesture-arbitration";
import { confirmDialog } from "@/utils/confirm-dialog";
import { projectIconPlaceholderLabelFromDisplayName } from "@/utils/project-display-name";
import { shouldRenderSyncedStatusLoader } from "@/utils/status-loader";
import { getStatusDotColor, isEmphasizedStatusDotBucket } from "@/utils/status-dot-color";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Shortcut } from "@/components/ui/shortcut";
import type { ShortcutKey } from "@/utils/format-shortcut";
import { useShortcutKeys } from "@/hooks/use-shortcut-keys";
import { useKeyboardActionHandler } from "@/hooks/use-keyboard-action-handler";
import { type PrHint, useWorkspacePrHint } from "@/hooks/use-checkout-pr-status-query";
import { buildSidebarProjectRowModel } from "@/utils/sidebar-project-row-model";
import {
  getNavigationActiveWorkspaceSelection,
  useIsNavigationProjectActive,
  useIsNavigationWorkspaceSelected,
} from "@/stores/navigation-active-workspace-store";
import {
  normalizeWorkspaceDescriptor,
  useSessionStore,
  type WorkspaceDescriptor,
} from "@/stores/session-store";
import { useWorkspaceFields } from "@/stores/session-store-hooks";
import { redirectIfArchivingActiveWorkspace } from "@/utils/sidebar-workspace-archive-redirect";
import { openExternalUrl } from "@/utils/open-external-url";
import {
  requireWorkspaceExecutionDirectory,
  resolveWorkspaceExecutionDirectory,
} from "@/utils/workspace-execution";
import {
  navigateToPreparedWorkspaceTab,
  navigateToReplacementWorkspacePreservingFocusedTab,
} from "@/utils/workspace-navigation";
import { WorkspaceHoverCard } from "@/components/workspace-hover-card";
import { GitHubIcon } from "@/components/icons/github-icon";
import { generateDraftId } from "@/stores/draft-keys";
import { isWeb as platformIsWeb, isNative as platformIsNative } from "@/constants/platform";
import {
  createWorktreeQuickly,
  isCreatingWorktreePlaceholderId,
} from "@/utils/quick-create-worktree";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";

function toProjectIconDataUri(icon: { mimeType: string; data: string } | null): string | null {
  if (!icon) {
    return null;
  }
  return `data:${icon.mimeType};base64,${icon.data}`;
}

const workspaceKeyExtractor = (workspace: SidebarWorkspaceEntry) => workspace.workspaceKey;

const projectKeyExtractor = (project: SidebarProjectEntry) => project.projectKey;
const WORKSPACE_STATUS_DOT_WIDTH = 14;
const DEFAULT_STATUS_DOT_SIZE = 7;
const EMPHASIZED_STATUS_DOT_SIZE = 9;
const DEFAULT_STATUS_DOT_OFFSET = 0;
const EMPHASIZED_STATUS_DOT_OFFSET = -1;
function getWorkspacePrIconColor(
  theme: ReturnType<typeof useUnistyles>["theme"],
  state: PrHint["state"],
) {
  switch (state) {
    case "merged":
      return theme.colors.palette.purple[500];
    case "open":
      return theme.colors.palette.green[500];
    case "closed":
      return theme.colors.palette.red[500];
  }
}

interface SidebarWorkspaceListProps {
  projects: SidebarProjectEntry[];
  serverId: string | null;
  collapsedProjectKeys: ReadonlySet<string>;
  onToggleProjectCollapsed: (projectKey: string) => void;
  shortcutIndexByWorkspaceKey: Map<string, number>;
  isRefreshing?: boolean;
  onRefresh?: () => void;
  onWorkspacePress?: () => void;
  onAddProject?: () => void;
  listFooterComponent?: ReactElement | null;
  /** Gesture ref for coordinating with parent gestures (e.g., sidebar close) */
  parentGestureRef?: MutableRefObject<GestureType | undefined>;
}

interface ProjectHeaderRowProps {
  project: SidebarProjectEntry;
  displayName: string;
  iconDataUri: string | null;
  workspace: SidebarWorkspaceEntry | null;
  selected?: boolean;
  chevron: "expand" | "collapse" | null;
  onPress: () => void;
  serverId: string | null;
  canCreateWorktree: boolean;
  isProjectActive?: boolean;
  onWorkspacePress?: () => void;
  shortcutNumber?: number | null;
  showShortcutBadge?: boolean;
  drag: () => void;
  isDragging: boolean;
  isArchiving?: boolean;
  menuController: ReturnType<typeof useContextMenu> | null;
  onRemoveProject?: () => void;
  onRenameProject?: () => void;
  removeProjectStatus?: "idle" | "pending";
  dragHandleProps?: DraggableListDragHandleProps;
}

interface WorkspaceRowInnerProps {
  workspace: SidebarWorkspaceEntry;
  selected: boolean;
  shortcutNumber: number | null;
  showShortcutBadge: boolean;
  onPress: () => void;
  drag: () => void;
  isDragging: boolean;
  isArchiving: boolean;
  isCreating?: boolean;
  dragHandleProps?: DraggableListDragHandleProps;
  menuController: ReturnType<typeof useContextMenu> | null;
  archiveLabel?: string;
  archiveStatus?: "idle" | "pending" | "success";
  archivePendingLabel?: string;
  onArchive?: () => void;
  onCopyBranchName?: () => void;
  onCopyPath?: () => void;
  archiveShortcutKeys?: ShortcutKey[][] | null;
}

function normalizePlaceholderWorkspaceName(name: string): string {
  return name.trim().toLocaleLowerCase();
}

function extractPlaceholderSlug(workspaceId: string): string | null {
  const separatorIndex = workspaceId.indexOf(":");
  if (separatorIndex < 0 || separatorIndex >= workspaceId.length - 1) {
    return null;
  }
  return workspaceId.slice(separatorIndex + 1);
}

function workspaceDirectoryHasSlug(workspaceDirectory: string | undefined, slug: string): boolean {
  if (!workspaceDirectory) {
    return false;
  }
  const normalizedDirectory = workspaceDirectory.replace(/\\/g, "/");
  return normalizedDirectory.endsWith(`/${slug}`);
}

function findPlaceholderReplacement(
  placeholder: SidebarWorkspaceEntry,
  realWorkspaces: readonly SidebarWorkspaceEntry[],
): SidebarWorkspaceEntry | null {
  const normalizedPlaceholderName = normalizePlaceholderWorkspaceName(placeholder.name);
  const byName = realWorkspaces.find(
    (workspace) => normalizePlaceholderWorkspaceName(workspace.name) === normalizedPlaceholderName,
  );
  if (byName) {
    return byName;
  }
  const placeholderSlug = extractPlaceholderSlug(placeholder.workspaceId);
  if (!placeholderSlug) {
    return null;
  }
  return (
    realWorkspaces.find((workspace) =>
      workspaceDirectoryHasSlug(workspace.workspaceDirectory, placeholderSlug),
    ) ?? null
  );
}

export function getSidebarRenderableProjects(
  projects: SidebarProjectEntry[],
): SidebarProjectEntry[] {
  let didChange = false;
  const nextProjects = projects.map((project) => {
    const realWorkspaces = project.workspaces.filter(
      (workspace) => !isCreatingWorktreePlaceholderId(workspace.workspaceId),
    );
    if (realWorkspaces.length === 0) {
      return project;
    }

    const filteredWorkspaces = project.workspaces.filter((workspace) => {
      if (!isCreatingWorktreePlaceholderId(workspace.workspaceId)) {
        return true;
      }
      return findPlaceholderReplacement(workspace, realWorkspaces) === null;
    });

    if (filteredWorkspaces.length === project.workspaces.length) {
      return project;
    }
    didChange = true;
    return {
      ...project,
      workspaces: filteredWorkspaces,
    };
  });

  return didChange ? nextProjects : projects;
}

export function findPlaceholderToRealWorkspaceReplacements(
  projects: readonly SidebarProjectEntry[],
): Map<string, SidebarWorkspaceEntry> {
  const replacements = new Map<string, SidebarWorkspaceEntry>();
  for (const project of projects) {
    const realWorkspaces = project.workspaces.filter(
      (workspace) => !isCreatingWorktreePlaceholderId(workspace.workspaceId),
    );
    if (realWorkspaces.length === 0) {
      continue;
    }
    for (const workspace of project.workspaces) {
      if (!isCreatingWorktreePlaceholderId(workspace.workspaceId)) {
        continue;
      }
      const replacement = findPlaceholderReplacement(workspace, realWorkspaces);
      if (replacement) {
        replacements.set(workspace.workspaceId, replacement);
      }
    }
  }
  return replacements;
}

function useSidebarWorkspaceEntry(
  serverId: string | null,
  workspaceId: string | null,
): SidebarWorkspaceEntry | null {
  const projectWorkspaceEntry = useCallback(
    (workspace: WorkspaceDescriptor): SidebarWorkspaceEntry =>
      createSidebarWorkspaceEntry({ serverId: serverId ?? "", workspace }),
    [serverId],
  );

  return useWorkspaceFields(serverId, workspaceId, projectWorkspaceEntry);
}

export function PrBadge({ hint }: { hint: PrHint }) {
  const { theme } = useUnistyles();
  const [isHovered, setIsHovered] = useState(false);
  const activeColor = isHovered ? theme.colors.foreground : theme.colors.foregroundMuted;
  const iconColor = getWorkspacePrIconColor(theme, hint.state);

  const handlePressIn = useCallback((event: GestureResponderEvent) => {
    event.stopPropagation();
  }, []);

  const handlePress = useCallback(
    (event: GestureResponderEvent) => {
      event.stopPropagation();
      void openExternalUrl(hint.url);
    },
    [hint.url],
  );

  return (
    <Pressable
      accessibilityRole="link"
      accessibilityLabel={`Pull request #${hint.number}`}
      hitSlop={4}
      onPressIn={handlePressIn}
      onPress={handlePress}
      onHoverIn={() => setIsHovered(true)}
      onHoverOut={() => setIsHovered(false)}
      style={({ pressed }) => [prBadgeStyles.badge, pressed && prBadgeStyles.badgePressed]}
    >
      {isHovered ? (
        <ExternalLink size={12} color={activeColor} />
      ) : (
        <GitPullRequest size={12} color={iconColor} />
      )}
      <Text style={[prBadgeStyles.text, { color: activeColor }]} numberOfLines={1}>
        #{hint.number}
      </Text>
    </Pressable>
  );
}

const prBadgeStyles = StyleSheet.create((theme) => ({
  badge: {
    flexDirection: "row",
    alignItems: "center",
    gap: 2,
  },
  badgePressed: {
    opacity: 0.82,
  },
  text: {
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.normal,
    lineHeight: 14,
  },
}));

function ChecksBadge({ checks }: { checks: PrHint["checks"] }): ReactElement | null {
  const { theme } = useUnistyles();

  if (!checks || checks.length === 0) return null;

  const failed = checks.filter((c) => c.status === "failure").length;
  if (failed === 0) return null;

  const color = theme.colors.palette.red[500];
  const label = `${failed} failed`;

  return (
    <View style={checksBadgeStyles.badge}>
      <GitHubIcon size={10} color={color} />
      <Text style={[checksBadgeStyles.text, { color }]}>{label}</Text>
    </View>
  );
}

const checksBadgeStyles = StyleSheet.create((theme) => ({
  badge: {
    flexDirection: "row",
    alignItems: "center",
    gap: 2,
  },
  text: {
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.normal,
    lineHeight: 14,
  },
}));

function WorkspaceStatusIndicator({
  bucket,
  workspaceKind,
  loading = false,
}: {
  bucket: SidebarWorkspaceEntry["statusBucket"];
  workspaceKind: SidebarWorkspaceEntry["workspaceKind"];
  loading?: boolean;
}) {
  const { theme } = useUnistyles();
  const shouldShowSyncedLoader = shouldRenderSyncedStatusLoader({ bucket });

  if (loading) {
    return (
      <View style={styles.workspaceStatusDot}>
        <ActivityIndicator size={8} color={theme.colors.foregroundMuted} />
      </View>
    );
  }

  if (shouldShowSyncedLoader) {
    return (
      <View style={styles.workspaceStatusDot}>
        <SyncedLoader
          size={11}
          color={
            theme.colorScheme === "light"
              ? theme.colors.palette.amber[700]
              : theme.colors.palette.amber[500]
          }
        />
      </View>
    );
  }

  if (bucket === "needs_input") {
    return (
      <View style={styles.workspaceStatusDot}>
        <CircleAlert size={14} color={theme.colors.palette.amber[500]} />
      </View>
    );
  }

  const KindIcon =
    workspaceKind === "local_checkout" ? Monitor : workspaceKind === "worktree" ? FolderGit2 : null;
  if (!KindIcon) return null;

  const dotColor = getStatusDotColor({ theme, bucket, showDoneAsInactive: false });
  const statusDotSize = isEmphasizedStatusDotBucket(bucket)
    ? EMPHASIZED_STATUS_DOT_SIZE
    : DEFAULT_STATUS_DOT_SIZE;
  const statusDotOffset =
    statusDotSize === EMPHASIZED_STATUS_DOT_SIZE
      ? EMPHASIZED_STATUS_DOT_OFFSET
      : DEFAULT_STATUS_DOT_OFFSET;

  return (
    <View style={styles.workspaceStatusDot}>
      <KindIcon size={14} color={theme.colors.foregroundMuted} />
      {dotColor ? (
        <View
          style={[
            styles.statusDotOverlay,
            {
              backgroundColor: dotColor,
              borderColor: theme.colors.surface0,
              width: statusDotSize,
              height: statusDotSize,
              right: statusDotOffset,
              bottom: statusDotOffset,
            },
          ]}
        />
      ) : null}
    </View>
  );
}

function ProjectLeadingVisual({
  displayName,
  iconDataUri,
  workspace,
  chevron = null,
  showChevron = false,
  isArchiving = false,
}: {
  displayName: string;
  iconDataUri: string | null;
  workspace: SidebarWorkspaceEntry | null;
  chevron?: "expand" | "collapse" | null;
  showChevron?: boolean;
  isArchiving?: boolean;
}) {
  const { theme } = useUnistyles();
  const placeholderLabel = projectIconPlaceholderLabelFromDisplayName(displayName);
  const placeholderInitial = placeholderLabel.charAt(0).toUpperCase();
  const activeWorkspace = workspace;
  const shouldShowWorkspaceStatus =
    activeWorkspace !== null && (isArchiving || activeWorkspace.statusBucket !== "done");
  const shouldShowSyncedLoader = activeWorkspace
    ? shouldRenderSyncedStatusLoader({ bucket: activeWorkspace.statusBucket })
    : false;

  if (showChevron && chevron !== null) {
    return (
      <View style={styles.projectLeadingVisualSlot}>
        <ProjectInlineChevron chevron={chevron} />
      </View>
    );
  }

  const projectIcon = iconDataUri ? (
    <Image source={{ uri: iconDataUri }} style={styles.projectIcon} />
  ) : (
    <View style={styles.projectIconFallback}>
      <Text style={styles.projectIconFallbackText}>{placeholderInitial}</Text>
    </View>
  );

  if (!shouldShowWorkspaceStatus || !activeWorkspace) {
    return <View style={styles.projectLeadingVisualSlot}>{projectIcon}</View>;
  }

  if (isArchiving) {
    return (
      <View style={styles.projectLeadingVisualSlot}>
        <ActivityIndicator size={8} color={theme.colors.foregroundMuted} />
      </View>
    );
  }

  if (shouldShowSyncedLoader) {
    return (
      <View style={styles.projectLeadingVisualSlot}>
        <SyncedLoader
          size={11}
          color={
            theme.colorScheme === "light"
              ? theme.colors.palette.amber[700]
              : theme.colors.palette.amber[500]
          }
        />
      </View>
    );
  }

  if (activeWorkspace.statusBucket === "needs_input") {
    return (
      <View style={styles.projectLeadingVisualSlot}>
        <CircleAlert size={14} color={theme.colors.palette.amber[500]} />
      </View>
    );
  }

  const dotColor = getStatusDotColor({
    theme,
    bucket: activeWorkspace.statusBucket,
    showDoneAsInactive: false,
  });
  const statusDotSize = isEmphasizedStatusDotBucket(activeWorkspace.statusBucket)
    ? EMPHASIZED_STATUS_DOT_SIZE
    : DEFAULT_STATUS_DOT_SIZE;
  const statusDotOffset =
    statusDotSize === EMPHASIZED_STATUS_DOT_SIZE
      ? EMPHASIZED_STATUS_DOT_OFFSET
      : DEFAULT_STATUS_DOT_OFFSET;

  return (
    <View style={styles.projectLeadingVisualSlot}>
      {projectIcon}
      {dotColor ? (
        <View
          style={[
            styles.statusDotOverlay,
            {
              backgroundColor: dotColor,
              borderColor: theme.colors.surface0,
              width: statusDotSize,
              height: statusDotSize,
              right: statusDotOffset,
              bottom: statusDotOffset,
            },
          ]}
        />
      ) : null}
    </View>
  );
}

function ProjectInlineChevron({ chevron }: { chevron: "expand" | "collapse" | null }) {
  if (chevron === null) {
    return null;
  }
  if (chevron === "collapse") {
    return <ChevronDown size={14} color="#9ca3af" />;
  }
  return <ChevronRight size={14} color="#9ca3af" />;
}

function NewWorktreeButton({
  displayName,
  onPress,
  visible,
  loading = false,
  testID,
  showShortcutHint = false,
}: {
  displayName: string;
  onPress: () => void;
  visible: boolean;
  loading?: boolean;
  testID: string;
  showShortcutHint?: boolean;
}) {
  const { theme } = useUnistyles();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).sidebarWorkspace, [locale]);
  const newWorktreeKeys = useShortcutKeys("new-worktree");

  return (
    <View style={styles.projectTrailingControlSlot} pointerEvents={visible ? "auto" : "none"}>
      <Tooltip delayDuration={0} enabledOnDesktop enabledOnMobile={false}>
        <TooltipTrigger asChild disabled={!visible}>
          <Pressable
            style={({ hovered, pressed }) => [
              styles.projectIconActionButton,
              !visible && styles.projectIconActionButtonHidden,
              (hovered || pressed) && !loading && styles.projectIconActionButtonHovered,
            ]}
            onPress={(event) => {
              event.stopPropagation();
              onPress();
            }}
            disabled={loading}
            accessibilityRole="button"
            accessibilityLabel={text.createNewWorktree(displayName)}
            testID={testID}
          >
            {({ hovered, pressed }) =>
              loading ? (
                <ActivityIndicator size={14} color={theme.colors.foregroundMuted} />
              ) : (
                <Plus
                  size={15}
                  color={
                    hovered || pressed ? theme.colors.foreground : theme.colors.foregroundMuted
                  }
                />
              )
            }
          </Pressable>
        </TooltipTrigger>
        <TooltipContent side="bottom" align="center" offset={8}>
          <View style={styles.projectActionTooltipRow}>
            <Text style={styles.projectActionTooltipText}>{text.newWorktree}</Text>
            {showShortcutHint && newWorktreeKeys ? (
              <Shortcut chord={newWorktreeKeys} style={styles.projectActionTooltipShortcut} />
            ) : null}
          </View>
        </TooltipContent>
      </Tooltip>
    </View>
  );
}

function useLongPressDragInteraction(input: {
  drag: () => void;
  menuController: ReturnType<typeof useContextMenu> | null;
}) {
  const didLongPressRef = useRef(false);
  const dragArmedRef = useRef(false);
  const dragActivatedRef = useRef(false);
  const didStartDragRef = useRef(false);
  const scrollIntentRef = useRef(false);
  const menuOpenedRef = useRef(false);
  const touchStartRef = useRef<{ x: number; y: number } | null>(null);
  const touchCurrentRef = useRef<{ x: number; y: number } | null>(null);
  const dragArmTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const contextMenuTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const clearTimers = useCallback(() => {
    if (dragArmTimerRef.current) {
      clearTimeout(dragArmTimerRef.current);
      dragArmTimerRef.current = null;
    }
    if (contextMenuTimerRef.current) {
      clearTimeout(contextMenuTimerRef.current);
      contextMenuTimerRef.current = null;
    }
  }, []);

  const openContextMenuAtStartPoint = useCallback(() => {
    if (!input.menuController || !touchStartRef.current) {
      return;
    }
    const statusBarHeight = Platform.OS === "android" ? (StatusBar.currentHeight ?? 0) : 0;
    input.menuController.setAnchorRect({
      x: touchStartRef.current.x,
      y: touchStartRef.current.y + statusBarHeight,
      width: 0,
      height: 0,
    });
    input.menuController.setOpen(true);
    menuOpenedRef.current = true;
    didLongPressRef.current = true;
  }, [input.menuController]);

  const handleLongPress = useCallback(() => {
    // Manual timers own long-press behavior on mobile.
  }, []);

  useEffect(() => {
    return () => {
      clearTimers();
    };
  }, [clearTimers]);

  const armTimers = useCallback(() => {
    clearTimers();

    const DRAG_ARM_DELAY_MS = 180;
    const DRAG_ARM_STATIONARY_SLOP_PX = 4;
    const CONTEXT_MENU_DELAY_MS = 450;
    const CONTEXT_MENU_STATIONARY_SLOP_PX = 6;

    dragArmTimerRef.current = setTimeout(() => {
      if (scrollIntentRef.current || didStartDragRef.current || menuOpenedRef.current) {
        return;
      }
      const start = touchStartRef.current;
      const current = touchCurrentRef.current ?? start;
      if (!start || !current) {
        return;
      }
      const dx = current.x - start.x;
      const dy = current.y - start.y;
      const distance = Math.sqrt(dx * dx + dy * dy);
      if (distance > DRAG_ARM_STATIONARY_SLOP_PX) {
        return;
      }
      dragArmedRef.current = true;
      dragActivatedRef.current = true;
      didLongPressRef.current = true;
      void Haptics.selectionAsync().catch(() => {});
      input.drag();
    }, DRAG_ARM_DELAY_MS);

    if (!input.menuController || platformIsWeb) {
      return;
    }

    contextMenuTimerRef.current = setTimeout(() => {
      if (scrollIntentRef.current || didStartDragRef.current || menuOpenedRef.current) {
        return;
      }
      const start = touchStartRef.current;
      const current = touchCurrentRef.current ?? start;
      if (!start || !current) {
        return;
      }
      const dx = current.x - start.x;
      const dy = current.y - start.y;
      const distance = Math.sqrt(dx * dx + dy * dy);
      if (distance > CONTEXT_MENU_STATIONARY_SLOP_PX) {
        return;
      }
      void Haptics.selectionAsync().catch(() => {});
      openContextMenuAtStartPoint();
    }, CONTEXT_MENU_DELAY_MS);
  }, [clearTimers, input.menuController, openContextMenuAtStartPoint]);

  const handleDragIntent = useCallback(
    (details: { dx: number; dy: number; distance: number }) => {
      if (!dragActivatedRef.current) {
        return;
      }
      didStartDragRef.current = true;
      didLongPressRef.current = true;
      clearTimers();
      void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium).catch(() => {});
    },
    [clearTimers],
  );

  const handleScrollIntent = useCallback(
    (details: { dx: number; dy: number; distance: number }) => {
      scrollIntentRef.current = true;
      didLongPressRef.current = true;
      clearTimers();
    },
    [clearTimers],
  );

  const handleSwipeIntent = useCallback(
    (details: { dx: number; dy: number; distance: number }) => {
      didLongPressRef.current = true;
      clearTimers();
    },
    [clearTimers],
  );

  const handlePressIn = useCallback(
    (event: GestureResponderEvent) => {
      didLongPressRef.current = false;
      dragArmedRef.current = false;
      dragActivatedRef.current = false;
      didStartDragRef.current = false;
      scrollIntentRef.current = false;
      menuOpenedRef.current = false;
      touchStartRef.current = {
        x: event.nativeEvent.pageX,
        y: event.nativeEvent.pageY,
      };
      touchCurrentRef.current = {
        x: event.nativeEvent.pageX,
        y: event.nativeEvent.pageY,
      };
      armTimers();
    },
    [armTimers],
  );

  const handleTouchMove = useCallback(
    (event: any) => {
      const start = touchStartRef.current;
      if (!start || didStartDragRef.current || menuOpenedRef.current) {
        return;
      }

      const touch = event?.nativeEvent?.touches?.[0] ?? event?.nativeEvent;
      const x = touch?.pageX;
      const y = touch?.pageY;
      if (typeof x !== "number" || typeof y !== "number") {
        return;
      }

      const current = { x, y };
      touchCurrentRef.current = current;
      const dx = current.x - start.x;
      const dy = current.y - start.y;
      const distance = Math.sqrt(dx * dx + dy * dy);
      const decision = decideLongPressMove({
        dragArmed: dragArmedRef.current,
        didStartDrag: didStartDragRef.current,
        startPoint: start,
        currentPoint: current,
      });

      if (decision === "vertical_scroll") {
        handleScrollIntent({ dx, dy, distance });
        return;
      }

      if (decision === "horizontal_swipe" || decision === "cancel_long_press") {
        handleSwipeIntent({ dx, dy, distance });
        return;
      }

      if (decision === "start_drag") {
        handleDragIntent({ dx, dy, distance });
      }
    },
    [handleDragIntent, handleScrollIntent, handleSwipeIntent],
  );

  const handlePressOut = useCallback(() => {
    clearTimers();
    dragArmedRef.current = false;
    dragActivatedRef.current = false;
    touchStartRef.current = null;
    touchCurrentRef.current = null;
  }, [clearTimers]);

  return {
    didLongPressRef,
    handleLongPress,
    handlePressIn,
    handleTouchMove,
    handlePressOut,
  };
}

function ProjectHeaderRow({
  project,
  displayName,
  iconDataUri,
  workspace,
  selected = false,
  chevron,
  onPress,
  serverId,
  canCreateWorktree,
  isProjectActive = false,
  onWorkspacePress,
  shortcutNumber = null,
  showShortcutBadge = false,
  drag,
  isDragging,
  isArchiving = false,
  menuController,
  onRemoveProject,
  onRenameProject,
  removeProjectStatus = "idle",
  dragHandleProps,
}: ProjectHeaderRowProps) {
  const { theme } = useUnistyles();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).sidebarWorkspace, [locale]);
  const [isHovered, setIsHovered] = useState(false);
  const isMobileBreakpoint = useIsCompactFormFactor();
  const runtimeServerId = serverId ?? "";
  const client = useHostRuntimeClient(runtimeServerId);
  const isConnected = useHostRuntimeIsConnected(runtimeServerId);
  const mergeWorkspaces = useSessionStore((state) => state.mergeWorkspaces);
  const removeWorkspace = useSessionStore((state) => state.removeWorkspace);
  const setWorkspaces = useSessionStore((state) => state.setWorkspaces);
  const toast = useToast();
  const handleBeginWorkspaceSetup = useCallback(() => {
    if (!serverId) {
      return;
    }
    const draftId = generateDraftId();
    void createWorktreeQuickly({
      client,
      isConnected,
      serverId,
      sourceDirectory: project.iconWorkingDir,
      projectId: project.projectKey,
      projectDisplayName: displayName,
      projectRootPath: project.iconWorkingDir,
      mergeWorkspaces,
      removeWorkspace,
      setWorkspaces,
      toast,
      onPlaceholderCreated: (placeholderWorkspace) => {
        onWorkspacePress?.();
        navigateToPreparedWorkspaceTab({
          serverId,
          workspaceId: placeholderWorkspace.id,
          target: { kind: "draft", draftId },
          navigationMethod: "navigate",
        });
      },
      onCreated: ({ workspace }) => {
        navigateToPreparedWorkspaceTab({
          serverId,
          workspaceId: workspace.id,
          target: { kind: "draft", draftId },
          navigationMethod: "replace",
        });
      },
    });
  }, [
    client,
    displayName,
    isConnected,
    mergeWorkspaces,
    navigateToPreparedWorkspaceTab,
    onWorkspacePress,
    project.iconWorkingDir,
    project.projectKey,
    removeWorkspace,
    serverId,
    setWorkspaces,
    toast,
  ]);

  const interaction = useLongPressDragInteraction({
    drag,
    menuController,
  });

  const handlePress = useCallback(() => {
    if (interaction.didLongPressRef.current) {
      interaction.didLongPressRef.current = false;
      return;
    }
    onPress();
  }, [interaction.didLongPressRef, onPress]);

  const rowChildren = (
    <>
      <View style={styles.projectRowLeft}>
        <ProjectLeadingVisual
          displayName={displayName}
          iconDataUri={iconDataUri}
          workspace={workspace}
          chevron={chevron}
          showChevron={isHovered && chevron !== null}
          isArchiving={isArchiving}
        />

        <View style={styles.projectTitleGroup}>
          <Text style={styles.projectTitle} numberOfLines={1}>
            {displayName}
          </Text>
        </View>
      </View>
      <View style={styles.projectTrailingActions}>
        {canCreateWorktree ? (
          <NewWorktreeButton
            displayName={displayName}
            onPress={handleBeginWorkspaceSetup}
            visible
            showShortcutHint={isProjectActive}
            testID={`sidebar-project-new-worktree-${project.projectKey}`}
          />
        ) : null}
        {onRemoveProject ? (
          <View>
            <DropdownMenu>
              <DropdownMenuTrigger
                hitSlop={8}
                style={({ hovered = false }) => [
                  styles.projectKebabButton,
                  hovered && styles.projectKebabButtonHovered,
                ]}
                accessibilityRole="button"
                accessibilityLabel={text.projectActions}
                testID={`sidebar-project-kebab-${project.projectKey}`}
              >
                {({ hovered }) => (
                  <MoreVertical
                    size={14}
                    color={hovered ? theme.colors.foreground : theme.colors.foregroundMuted}
                  />
                )}
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" width={220}>
                {onRenameProject ? (
                  <DropdownMenuItem
                    testID={`sidebar-project-menu-rename-${project.projectKey}`}
                    leading={<Pencil size={14} color={theme.colors.foregroundMuted} />}
                    onSelect={onRenameProject}
                  >
                    {text.renameProject}
                  </DropdownMenuItem>
                ) : null}
                <DropdownMenuItem
                  testID={`sidebar-project-menu-remove-${project.projectKey}`}
                  leading={<Trash2 size={14} color={theme.colors.foregroundMuted} />}
                  status={removeProjectStatus}
                  pendingLabel={text.deleting}
                  onSelect={onRemoveProject}
                >
                  {text.deleteProject}
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </View>
        ) : null}
      </View>
      {showShortcutBadge && shortcutNumber !== null ? (
        <View style={styles.shortcutBadge}>
          <Text style={styles.shortcutBadgeText}>{shortcutNumber}</Text>
        </View>
      ) : null}
    </>
  );

  if (menuController) {
    return (
      <View
        {...(dragHandleProps?.attributes as any)}
        {...(dragHandleProps?.listeners as any)}
        ref={dragHandleProps?.setActivatorNodeRef as any}
        onPointerEnter={() => setIsHovered(true)}
        onPointerLeave={() => setIsHovered(false)}
      >
        <ContextMenuTrigger
          enabledOnMobile={false}
          style={({ pressed }) => [
            styles.projectRow,
            isDragging && styles.projectRowDragging,
            selected && styles.sidebarRowSelected,
            isHovered && styles.projectRowHovered,
            pressed && styles.projectRowPressed,
          ]}
          onPressIn={interaction.handlePressIn}
          onTouchMove={interaction.handleTouchMove}
          onPressOut={interaction.handlePressOut}
          onPress={handlePress}
          testID={`sidebar-project-row-${project.projectKey}`}
        >
          {rowChildren}
        </ContextMenuTrigger>
      </View>
    );
  }

  return (
    <View
      {...(dragHandleProps?.attributes as any)}
      {...(dragHandleProps?.listeners as any)}
      ref={dragHandleProps?.setActivatorNodeRef as any}
      onPointerEnter={() => setIsHovered(true)}
      onPointerLeave={() => setIsHovered(false)}
    >
      <Pressable
        style={({ pressed }) => [
          styles.projectRow,
          isDragging && styles.projectRowDragging,
          selected && styles.sidebarRowSelected,
          isHovered && styles.projectRowHovered,
          pressed && styles.projectRowPressed,
        ]}
        onPressIn={interaction.handlePressIn}
        onTouchMove={interaction.handleTouchMove}
        onPressOut={interaction.handlePressOut}
        onPress={handlePress}
        testID={`sidebar-project-row-${project.projectKey}`}
      >
        {rowChildren}
      </Pressable>
    </View>
  );
}

function WorkspaceRowInner({
  workspace,
  selected,
  shortcutNumber,
  showShortcutBadge,
  onPress,
  drag,
  isDragging,
  isArchiving,
  isCreating = false,
  dragHandleProps,
  menuController,
  archiveLabel,
  archiveStatus = "idle",
  archivePendingLabel,
  onArchive,
  onCopyBranchName,
  onCopyPath,
  archiveShortcutKeys,
}: WorkspaceRowInnerProps) {
  const { theme } = useUnistyles();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).sidebarWorkspace, [locale]);
  const isCompact = useIsCompactFormFactor();
  const [isHovered, setIsHovered] = useState(false);
  const isTouchPlatform = platformIsNative;
  const workspaceDirectory = resolveWorkspaceExecutionDirectory({
    workspaceDirectory: workspace.workspaceDirectory,
  });
  const prHint = useWorkspacePrHint({
    serverId: workspace.serverId,
    cwd: workspaceDirectory ?? "",
    enabled: workspace.projectKind === "git" && Boolean(workspaceDirectory),
  });
  const interaction = useLongPressDragInteraction({
    drag,
    menuController,
  });

  const handlePress = useCallback(() => {
    if (interaction.didLongPressRef.current) {
      interaction.didLongPressRef.current = false;
      return;
    }
    onPress();
  }, [interaction.didLongPressRef, onPress]);

  const isDesktop = !isTouchPlatform;
  const showScriptsIcon = isDesktop && workspace.hasRunningScripts;
  const hasRunningService = workspace.scripts.some(
    (s) => s.lifecycle === "running" && (s.type ?? "service") === "service",
  );

  return (
    <WorkspaceHoverCard workspace={workspace} prHint={prHint} isDragging={isDragging}>
      <View
        {...(dragHandleProps?.attributes as any)}
        {...(dragHandleProps?.listeners as any)}
        ref={dragHandleProps?.setActivatorNodeRef as any}
        style={styles.workspaceRowContainer}
        onPointerEnter={() => setIsHovered(true)}
        onPointerLeave={() => setIsHovered(false)}
      >
        <Pressable
          disabled={isArchiving}
          aria-selected={selected}
          accessibilityRole="button"
          accessibilityState={{ selected }}
          style={({ pressed }) => [
            styles.workspaceRow,
            isDragging && styles.workspaceRowDragging,
            selected && styles.sidebarRowSelected,
            isHovered && styles.workspaceRowHovered,
            pressed && styles.workspaceRowPressed,
          ]}
          onPressIn={interaction.handlePressIn}
          onTouchMove={interaction.handleTouchMove}
          onPressOut={interaction.handlePressOut}
          onPress={handlePress}
          testID={`sidebar-workspace-row-${workspace.workspaceKey}`}
        >
          <View style={styles.workspaceRowMain}>
            <View style={styles.workspaceRowLeft}>
              <WorkspaceStatusIndicator
                bucket={workspace.statusBucket}
                workspaceKind={workspace.workspaceKind}
                loading={isArchiving || isCreating}
              />
              <Text
                style={[
                  styles.workspaceBranchText,
                  isHovered && styles.workspaceBranchTextHovered,
                  isCreating && styles.workspaceBranchTextCreating,
                ]}
                numberOfLines={1}
              >
                {workspace.name}
              </Text>
            </View>
            <View style={styles.workspaceRowRight}>
              {showScriptsIcon ? (
                <View testID="workspace-globe-icon" accessibilityLabel={text.scriptsAvailable}>
                  {hasRunningService ? (
                    <Globe size={12} color={theme.colors.palette.blue[500]} />
                  ) : (
                    <SquareTerminal size={12} color={theme.colors.palette.blue[500]} />
                  )}
                </View>
              ) : null}
              {isCreating ? (
                <Text style={styles.workspaceCreatingText}>{text.creating}</Text>
              ) : null}
              {onArchive && (isHovered || isTouchPlatform) ? (
                <DropdownMenu>
                  <DropdownMenuTrigger
                    hitSlop={8}
                    style={({ hovered = false }) => [
                      styles.kebabButton,
                      hovered && styles.kebabButtonHovered,
                    ]}
                    accessibilityRole="button"
                    accessibilityLabel={text.workspaceActions}
                    testID={`sidebar-workspace-kebab-${workspace.workspaceKey}`}
                  >
                    {({ hovered }) => (
                      <MoreVertical
                        size={14}
                        color={hovered ? theme.colors.foreground : theme.colors.foregroundMuted}
                      />
                    )}
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end" width={260}>
                    {onCopyPath ? (
                      <DropdownMenuItem
                        testID={`sidebar-workspace-menu-copy-path-${workspace.workspaceKey}`}
                        leading={<Copy size={14} color={theme.colors.foregroundMuted} />}
                        onSelect={onCopyPath}
                      >
                        {text.copyPath}
                      </DropdownMenuItem>
                    ) : null}
                    {onCopyBranchName ? (
                      <DropdownMenuItem
                        testID={`sidebar-workspace-menu-copy-branch-name-${workspace.workspaceKey}`}
                        leading={<Copy size={14} color={theme.colors.foregroundMuted} />}
                        onSelect={onCopyBranchName}
                      >
                        {text.copyBranchName}
                      </DropdownMenuItem>
                    ) : null}
                    <DropdownMenuItem
                      testID={`sidebar-workspace-menu-archive-${workspace.workspaceKey}`}
                      leading={<Archive size={14} color={theme.colors.foregroundMuted} />}
                      trailing={
                        archiveShortcutKeys ? <Shortcut chord={archiveShortcutKeys} /> : null
                      }
                      status={archiveStatus}
                      pendingLabel={archivePendingLabel}
                      onSelect={onArchive}
                    >
                      {archiveLabel ?? text.archive}
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              ) : workspace.diffStat ? (
                <DiffStat
                  additions={workspace.diffStat.additions}
                  deletions={workspace.diffStat.deletions}
                />
              ) : null}
              {showShortcutBadge && shortcutNumber !== null ? (
                <View style={styles.shortcutBadge}>
                  <Text style={styles.shortcutBadgeText}>{shortcutNumber}</Text>
                </View>
              ) : null}
            </View>
          </View>
          {prHint ? (
            <View style={styles.workspacePrBadgeRow}>
              <PrBadge hint={prHint} />
              <ChecksBadge checks={prHint.checks} />
            </View>
          ) : null}
        </Pressable>
      </View>
    </WorkspaceHoverCard>
  );
}

function WorkspaceRowWithMenu({
  workspace,
  selected,
  shortcutNumber,
  showShortcutBadge,
  onPress,
  drag,
  isDragging,
  dragHandleProps,
  canCopyBranchName,
  isCreating = false,
}: {
  workspace: SidebarWorkspaceEntry;
  selected: boolean;
  shortcutNumber: number | null;
  showShortcutBadge: boolean;
  onPress: () => void;
  drag: () => void;
  isDragging: boolean;
  dragHandleProps?: DraggableListDragHandleProps;
  canCopyBranchName: boolean;
  isCreating?: boolean;
}) {
  const toast = useToast();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).sidebarWorkspace, [locale]);
  const archiveWorktree = useCheckoutGitActionsStore((state) => state.archiveWorktree);
  const [isArchivingWorkspace, setIsArchivingWorkspace] = useState(false);
  const isCreatingPlaceholder = isCreatingWorktreePlaceholderId(workspace.workspaceId);
  const workspaceDirectory = resolveWorkspaceExecutionDirectory({
    workspaceDirectory: workspace.workspaceDirectory,
  });
  const archiveStatus = useCheckoutGitActionsStore((state) =>
    workspaceDirectory
      ? state.getStatus({
          serverId: workspace.serverId,
          cwd: workspaceDirectory,
          actionId: "archive-worktree",
        })
      : "idle",
  );
  const isWorktree = workspace.workspaceKind === "worktree";
  const isArchiving = isWorktree ? archiveStatus === "pending" : isArchivingWorkspace;
  const redirectAfterArchive = useCallback(() => {
    redirectIfArchivingActiveWorkspace({
      serverId: workspace.serverId,
      workspaceId: workspace.workspaceId,
    });
  }, [workspace.serverId, workspace.workspaceId]);

  const handleArchiveWorktree = useCallback(() => {
    if (isArchiving) {
      return;
    }

    void (async () => {
      const confirmed = await confirmDialog({
        title: text.archiveWorktreeTitle,
        message: text.archiveWorktreeMessage(workspace.name),
        confirmLabel: text.archive,
        cancelLabel: text.cancel,
        destructive: true,
      });

      if (!confirmed) {
        return;
      }
      let workspaceDirectory: string;
      try {
        workspaceDirectory = requireWorkspaceExecutionDirectory({
          workspaceId: workspace.workspaceId,
          workspaceDirectory: workspace.workspaceDirectory,
        });
      } catch (error) {
        toast.error(error instanceof Error ? error.message : text.workspacePathUnavailable);
        return;
      }

      if (!workspaceDirectory) {
        toast.error(text.workspacePathUnavailable);
        return;
      }

      redirectAfterArchive();

      void archiveWorktree({
        serverId: workspace.serverId,
        cwd: workspaceDirectory,
        worktreePath: workspaceDirectory,
      }).catch((error) => {
        const message = error instanceof Error ? error.message : text.failedArchiveWorktree;
        toast.error(message);
      });
    })();
  }, [
    archiveWorktree,
    isArchiving,
    redirectAfterArchive,
    toast,
    workspace.name,
    workspace.workspaceDirectory,
    workspace.serverId,
    workspace.workspaceId,
    text,
  ]);

  const handleArchiveWorkspace = useCallback(() => {
    if (isArchivingWorkspace) {
      return;
    }

    void (async () => {
      const confirmed = await confirmDialog({
        title: text.hideWorkspaceTitle,
        message: text.hideWorkspaceMessage(workspace.name),
        confirmLabel: text.hide,
        cancelLabel: text.cancel,
        destructive: true,
      });
      if (!confirmed) {
        return;
      }

      const client = getHostRuntimeStore().getClient(workspace.serverId);
      if (!client) {
        toast.error(text.hostNotConnected);
        return;
      }

      setIsArchivingWorkspace(true);
      redirectAfterArchive();

      void (async () => {
        try {
          const payload = await client.archiveWorkspace(workspace.workspaceId);
          if (payload.error) {
            throw new Error(payload.error);
          }
        } catch (error) {
          toast.error(error instanceof Error ? error.message : text.failedHideWorkspace);
        } finally {
          setIsArchivingWorkspace(false);
        }
      })();
    })();
  }, [
    isArchivingWorkspace,
    redirectAfterArchive,
    toast,
    workspace.name,
    workspace.serverId,
    workspace.workspaceId,
    text,
  ]);

  const handleCopyPath = useCallback(() => {
    let workspaceDirectory: string;
    try {
      workspaceDirectory = requireWorkspaceExecutionDirectory({
        workspaceId: workspace.workspaceId,
        workspaceDirectory: workspace.workspaceDirectory,
      });
    } catch (error) {
      toast.error(error instanceof Error ? error.message : text.workspacePathUnavailable);
      return;
    }
    void Clipboard.setStringAsync(workspaceDirectory);
    toast.copied(text.pathCopied);
  }, [text, toast, workspace.workspaceDirectory, workspace.workspaceId]);

  const handleCopyBranchName = useCallback(() => {
    void Clipboard.setStringAsync(workspace.name);
    toast.copied(text.branchNameCopied);
  }, [text.branchNameCopied, toast, workspace.name]);

  const archiveShortcutKeys = useShortcutKeys("archive-worktree");

  useKeyboardActionHandler({
    handlerId: `worktree-archive-${workspace.workspaceKey}`,
    actions: ["worktree.archive"],
    enabled: selected && !isArchiving && !isCreatingPlaceholder,
    priority: 0,
    handle: () => {
      if (isWorktree) {
        handleArchiveWorktree();
      } else {
        handleArchiveWorkspace();
      }
      return true;
    },
  });

  return (
    <WorkspaceRowInner
      workspace={workspace}
      selected={selected}
      shortcutNumber={shortcutNumber}
      showShortcutBadge={showShortcutBadge}
      onPress={onPress}
      drag={drag}
      isDragging={isDragging}
      isArchiving={isArchiving}
      isCreating={isCreating || isCreatingPlaceholder}
      dragHandleProps={dragHandleProps}
      menuController={null}
      archiveLabel={isWorktree ? text.archiveWorktree : text.hideFromSidebar}
      archiveStatus={isWorktree ? archiveStatus : isArchivingWorkspace ? "pending" : "idle"}
      archivePendingLabel={isWorktree ? text.archiving : text.hiding}
      onArchive={
        isCreatingPlaceholder
          ? undefined
          : isWorktree
            ? handleArchiveWorktree
            : handleArchiveWorkspace
      }
      onCopyBranchName={
        canCopyBranchName && !isCreatingPlaceholder ? handleCopyBranchName : undefined
      }
      onCopyPath={isCreatingPlaceholder ? undefined : handleCopyPath}
      archiveShortcutKeys={selected && !isCreatingPlaceholder ? archiveShortcutKeys : null}
    />
  );
}

function NonGitProjectRowWithMenuContent({
  project,
  displayName,
  iconDataUri,
  workspace,
  selected,
  onPress,
  shortcutNumber,
  showShortcutBadge,
  drag,
  isDragging,
  dragHandleProps,
}: {
  project: SidebarProjectEntry;
  displayName: string;
  iconDataUri: string | null;
  workspace: SidebarWorkspaceEntry;
  selected: boolean;
  onPress: () => void;
  shortcutNumber: number | null;
  showShortcutBadge: boolean;
  drag: () => void;
  isDragging: boolean;
  dragHandleProps?: DraggableListDragHandleProps;
}) {
  const toast = useToast();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).sidebarWorkspace, [locale]);
  const contextMenu = useContextMenu();
  const [isArchivingWorkspace, setIsArchivingWorkspace] = useState(false);
  const redirectAfterArchive = useCallback(() => {
    redirectIfArchivingActiveWorkspace({
      serverId: workspace.serverId,
      workspaceId: workspace.workspaceId,
    });
  }, [workspace.serverId, workspace.workspaceId]);

  const handleArchiveWorkspace = useCallback(() => {
    if (isArchivingWorkspace) {
      return;
    }

    void (async () => {
      const confirmed = await confirmDialog({
        title: text.hideWorkspaceTitle,
        message: text.hideWorkspaceMessage(workspace.name),
        confirmLabel: text.hide,
        cancelLabel: text.cancel,
        destructive: true,
      });
      if (!confirmed) {
        return;
      }

      const client = getHostRuntimeStore().getClient(workspace.serverId);
      if (!client) {
        toast.error(text.hostNotConnected);
        return;
      }

      setIsArchivingWorkspace(true);
      redirectAfterArchive();

      void (async () => {
        try {
          const payload = await client.archiveWorkspace(workspace.workspaceId);
          if (payload.error) {
            throw new Error(payload.error);
          }
        } catch (error) {
          toast.error(error instanceof Error ? error.message : text.failedHideWorkspace);
        } finally {
          setIsArchivingWorkspace(false);
        }
      })();
    })();
  }, [
    isArchivingWorkspace,
    redirectAfterArchive,
    toast,
    workspace.name,
    workspace.serverId,
    workspace.workspaceId,
    text,
  ]);

  return (
    <>
      <ProjectHeaderRow
        project={project}
        displayName={displayName}
        iconDataUri={iconDataUri}
        workspace={workspace}
        selected={selected}
        chevron={null}
        onPress={onPress}
        serverId={null}
        canCreateWorktree={false}
        shortcutNumber={shortcutNumber}
        showShortcutBadge={showShortcutBadge}
        drag={drag}
        isDragging={isDragging}
        isArchiving={isArchivingWorkspace}
        menuController={contextMenu}
        dragHandleProps={dragHandleProps}
      />
      <ContextMenuContent
        align="start"
        width={220}
        mobileMode="sheet"
        testID={`sidebar-workspace-context-${workspace.workspaceKey}`}
      >
        <ContextMenuItem
          testID={`sidebar-workspace-context-${workspace.workspaceKey}-archive`}
          status={isArchivingWorkspace ? "pending" : "idle"}
          pendingLabel={text.hiding}
          destructive
          onSelect={handleArchiveWorkspace}
        >
          {text.hideFromSidebar}
        </ContextMenuItem>
      </ContextMenuContent>
    </>
  );
}

function NonGitProjectRowWithMenu(props: {
  project: SidebarProjectEntry;
  displayName: string;
  iconDataUri: string | null;
  workspace: SidebarWorkspaceEntry;
  selected: boolean;
  onPress: () => void;
  shortcutNumber: number | null;
  showShortcutBadge: boolean;
  drag: () => void;
  isDragging: boolean;
  dragHandleProps?: DraggableListDragHandleProps;
}) {
  return (
    <ContextMenu>
      <NonGitProjectRowWithMenuContent {...props} />
    </ContextMenu>
  );
}

function FlattenedProjectRow({
  project,
  displayName,
  iconDataUri,
  rowModel,
  onPress,
  serverId,
  onWorkspacePress,
  shortcutNumber,
  showShortcutBadge,
  drag,
  isDragging,
  dragHandleProps,
  isProjectActive = false,
  onRemoveProject,
  onRenameProject,
  removeProjectStatus,
  selectionEnabled,
}: {
  project: SidebarProjectEntry;
  displayName: string;
  iconDataUri: string | null;
  rowModel: Extract<ReturnType<typeof buildSidebarProjectRowModel>, { kind: "workspace_link" }>;
  onPress: () => void;
  serverId: string | null;
  onWorkspacePress?: () => void;
  shortcutNumber: number | null;
  showShortcutBadge: boolean;
  drag: () => void;
  isDragging: boolean;
  dragHandleProps?: DraggableListDragHandleProps;
  isProjectActive?: boolean;
  onRemoveProject?: () => void;
  onRenameProject?: () => void;
  removeProjectStatus?: "idle" | "pending";
  selectionEnabled: boolean;
}) {
  const workspace = useSidebarWorkspaceEntry(serverId, rowModel.workspace.workspaceId);
  const selected = useIsNavigationWorkspaceSelected({
    serverId,
    workspaceId: rowModel.workspace.workspaceId,
    enabled: selectionEnabled,
  });

  if (!workspace) {
    return null;
  }

  if (project.projectKind === "directory") {
    return (
      <NonGitProjectRowWithMenu
        project={project}
        displayName={displayName}
        iconDataUri={iconDataUri}
        workspace={workspace}
        selected={selected}
        onPress={onPress}
        shortcutNumber={shortcutNumber}
        showShortcutBadge={showShortcutBadge}
        drag={drag}
        isDragging={isDragging}
        dragHandleProps={dragHandleProps}
      />
    );
  }

  return (
    <ProjectHeaderRow
      project={project}
      displayName={displayName}
      iconDataUri={iconDataUri}
      workspace={workspace}
      selected={selected}
      chevron={rowModel.chevron}
      onPress={onPress}
      serverId={serverId}
      canCreateWorktree={rowModel.trailingAction === "new_worktree"}
      isProjectActive={isProjectActive}
      onWorkspacePress={onWorkspacePress}
      shortcutNumber={shortcutNumber}
      showShortcutBadge={showShortcutBadge}
      drag={drag}
      isDragging={isDragging}
      menuController={null}
      onRemoveProject={onRemoveProject}
      onRenameProject={onRenameProject}
      removeProjectStatus={removeProjectStatus}
      dragHandleProps={dragHandleProps}
    />
  );
}

function WorkspaceRow({
  workspace,
  shortcutNumber,
  showShortcutBadge,
  onPress,
  drag,
  isDragging,
  dragHandleProps,
  canCopyBranchName,
  selectionEnabled,
}: {
  workspace: SidebarWorkspaceEntry;
  shortcutNumber: number | null;
  showShortcutBadge: boolean;
  onPress: () => void;
  drag: () => void;
  isDragging: boolean;
  dragHandleProps?: DraggableListDragHandleProps;
  canCopyBranchName: boolean;
  selectionEnabled: boolean;
}) {
  const isCreatingPlaceholder = isCreatingWorktreePlaceholderId(workspace.workspaceId);
  const hydratedWorkspace = useSidebarWorkspaceEntry(workspace.serverId, workspace.workspaceId);
  const selected = useIsNavigationWorkspaceSelected({
    serverId: workspace.serverId,
    workspaceId: workspace.workspaceId,
    enabled: selectionEnabled,
  });

  if (!hydratedWorkspace) {
    return null;
  }

  return (
    <WorkspaceRowWithMenu
      workspace={hydratedWorkspace}
      selected={selected}
      shortcutNumber={shortcutNumber}
      showShortcutBadge={showShortcutBadge}
      onPress={isCreatingPlaceholder ? () => {} : onPress}
      drag={drag}
      isDragging={isDragging}
      dragHandleProps={dragHandleProps}
      canCopyBranchName={canCopyBranchName && !isCreatingPlaceholder}
      isCreating={isCreatingPlaceholder}
    />
  );
}

function ProjectBlock({
  project,
  collapsed,
  displayName,
  iconDataUri,
  serverId,
  selectionEnabled,
  showShortcutBadges,
  shortcutIndexByWorkspaceKey,
  parentGestureRef,
  onToggleCollapsed,
  onWorkspacePress,
  onWorkspaceReorder,
  currentPathname,
  drag,
  isDragging,
  dragHandleProps,
  useNestable,
}: {
  project: SidebarProjectEntry;
  collapsed: boolean;
  displayName: string;
  iconDataUri: string | null;
  serverId: string | null;
  selectionEnabled: boolean;
  showShortcutBadges: boolean;
  shortcutIndexByWorkspaceKey: Map<string, number>;
  parentGestureRef?: MutableRefObject<GestureType | undefined>;
  onToggleCollapsed: () => void;
  onWorkspacePress?: () => void;
  onWorkspaceReorder: (projectKey: string, workspaces: SidebarWorkspaceEntry[]) => void;
  currentPathname: string | null;
  drag: () => void;
  isDragging: boolean;
  dragHandleProps?: DraggableListDragHandleProps;
  useNestable: boolean;
}) {
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).sidebarWorkspace, [locale]);
  const rowModel = useMemo(
    () =>
      buildSidebarProjectRowModel({
        project,
        collapsed,
      }),
    [collapsed, project],
  );

  const projectWorkspaceIds = useMemo(
    () => project.workspaces.map((workspace) => workspace.workspaceId),
    [project.workspaces],
  );
  const isProjectActive = useIsNavigationProjectActive({
    serverId,
    workspaceIds: projectWorkspaceIds,
    enabled: selectionEnabled,
  });

  const renderWorkspaceRow = useCallback(
    (
      item: SidebarWorkspaceEntry,
      input?: {
        drag?: () => void;
        isDragging?: boolean;
        dragHandleProps?: DraggableListDragHandleProps;
      },
    ) => {
      return (
        <WorkspaceRow
          workspace={item}
          shortcutNumber={shortcutIndexByWorkspaceKey.get(item.workspaceKey) ?? null}
          showShortcutBadge={showShortcutBadges}
          canCopyBranchName={project.projectKind === "git"}
          selectionEnabled={selectionEnabled}
          onPress={() => {
            if (!serverId) {
              return;
            }
            onWorkspacePress?.();
            navigateToWorkspace(serverId, item.workspaceId, { currentPathname });
          }}
          drag={input?.drag ?? (() => {})}
          isDragging={input?.isDragging ?? false}
          dragHandleProps={input?.dragHandleProps}
        />
      );
    },
    [
      project.projectKind,
      currentPathname,
      onWorkspacePress,
      serverId,
      selectionEnabled,
      shortcutIndexByWorkspaceKey,
      showShortcutBadges,
    ],
  );

  const renderWorkspace = useCallback(
    ({
      item,
      drag: workspaceDrag,
      isActive,
      dragHandleProps: workspaceDragHandleProps,
    }: DraggableRenderItemInfo<SidebarWorkspaceEntry>) => {
      return renderWorkspaceRow(item, {
        drag: workspaceDrag,
        isDragging: isActive,
        dragHandleProps: workspaceDragHandleProps,
      });
    },
    [renderWorkspaceRow],
  );

  const handleWorkspaceDragEnd = useCallback(
    (workspaces: SidebarWorkspaceEntry[]) => {
      onWorkspaceReorder(project.projectKey, workspaces);
    },
    [onWorkspaceReorder, project.projectKey],
  );

  const toast = useToast();
  const [isRemovingProject, setIsRemovingProject] = useState(false);
  const setProjectName = useSidebarProjectNamesStore((state) => state.setProjectName);

  const handleRenameProject = useCallback(() => {
    if (platformIsWeb) {
      const newName = window.prompt(text.renamePrompt, displayName);
      if (newName && newName.trim() !== "" && newName.trim() !== displayName) {
        setProjectName(project.projectKey, newName.trim());
      }
    }
  }, [displayName, project.projectKey, setProjectName, text.renamePrompt]);

  const handleRemoveProject = useCallback(() => {
    if (isRemovingProject || !serverId) {
      return;
    }

    void (async () => {
      const confirmed = await confirmDialog({
        title: text.removeProjectTitle,
        message: text.removeProjectMessage(displayName),
        confirmLabel: text.remove,
        cancelLabel: text.cancel,
        destructive: true,
      });
      if (!confirmed) {
        return;
      }

      const client = getHostRuntimeStore().getClient(serverId);
      if (!client) {
        toast.error(text.hostNotConnected);
        return;
      }

      setIsRemovingProject(true);

      void Promise.allSettled(
        project.workspaces.map(async (ws) => {
          const payload = await client.archiveWorkspace(ws.workspaceId);
          if (payload.error) {
            throw new Error(payload.error);
          }
        }),
      ).then((results) => {
        const failed = results.filter((r) => r.status === "rejected");
        if (failed.length > 0) {
          toast.error(text.failedRemoveSomeWorkspaces);
        }
        setIsRemovingProject(false);
      });
    })();
  }, [isRemovingProject, serverId, displayName, toast, project.workspaces, text]);

  return (
    <View style={styles.projectBlock}>
      {rowModel.kind === "workspace_link" ? (
        <FlattenedProjectRow
          project={project}
          displayName={displayName}
          iconDataUri={iconDataUri}
          rowModel={rowModel}
          onPress={() => {
            if (!serverId) {
              return;
            }
            onWorkspacePress?.();
            navigateToWorkspace(serverId, rowModel.workspace.workspaceId, {
              currentPathname,
            });
          }}
          serverId={serverId}
          onWorkspacePress={onWorkspacePress}
          shortcutNumber={shortcutIndexByWorkspaceKey.get(rowModel.workspace.workspaceKey) ?? null}
          showShortcutBadge={showShortcutBadges}
          drag={drag}
          isDragging={isDragging}
          dragHandleProps={dragHandleProps}
          isProjectActive={isProjectActive}
          onRemoveProject={handleRemoveProject}
          onRenameProject={handleRenameProject}
          removeProjectStatus={isRemovingProject ? "pending" : "idle"}
          selectionEnabled={selectionEnabled}
        />
      ) : (
        <>
          <ProjectHeaderRow
            project={project}
            displayName={displayName}
            iconDataUri={iconDataUri}
            workspace={null}
            selected={false}
            chevron={rowModel.chevron}
            onPress={onToggleCollapsed}
            serverId={serverId}
            canCreateWorktree={rowModel.trailingAction === "new_worktree"}
            isProjectActive={isProjectActive}
            onWorkspacePress={onWorkspacePress}
            drag={drag}
            isDragging={isDragging}
            isArchiving={isRemovingProject}
            menuController={null}
            onRemoveProject={handleRemoveProject}
            onRenameProject={handleRenameProject}
            removeProjectStatus={isRemovingProject ? "pending" : "idle"}
            dragHandleProps={dragHandleProps}
          />

          {!collapsed ? (
            <DraggableList
              testID={`sidebar-workspace-list-${project.projectKey}`}
              data={project.workspaces}
              keyExtractor={workspaceKeyExtractor}
              renderItem={renderWorkspace}
              onDragEnd={handleWorkspaceDragEnd}
              scrollEnabled={false}
              useDragHandle
              nestable={useNestable}
              simultaneousGestureRef={parentGestureRef}
              containerStyle={styles.workspaceListContainer}
            />
          ) : null}
        </>
      )}
    </View>
  );
}

export function SidebarWorkspaceList({
  projects,
  serverId,
  collapsedProjectKeys,
  onToggleProjectCollapsed,
  shortcutIndexByWorkspaceKey,
  isRefreshing = false,
  onRefresh,
  onWorkspacePress,
  onAddProject,
  listFooterComponent,
  parentGestureRef,
}: SidebarWorkspaceListProps) {
  const pathname = usePathname();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).sidebarWorkspace, [locale]);
  const projectNameOverrides = useSidebarProjectNamesStore((state) => state.namesByProjectKey);
  const showShortcutBadges = useShowShortcutBadges();
  const projectsForRender = useMemo(() => getSidebarRenderableProjects(projects), [projects]);

  const getProjectOrder = useSidebarOrderStore((state) => state.getProjectOrder);
  const setProjectOrder = useSidebarOrderStore((state) => state.setProjectOrder);
  const getWorkspaceOrder = useSidebarOrderStore((state) => state.getWorkspaceOrder);
  const setWorkspaceOrder = useSidebarOrderStore((state) => state.setWorkspaceOrder);
  const setWorkspaces = useSessionStore((state) => state.setWorkspaces);

  const isWorkspaceRoute = useMemo(
    () => Boolean(pathname && parseHostWorkspaceRouteFromPathname(pathname)),
    [pathname],
  );
  const selectionEnabled = isWorkspaceRoute;

  const projectIconRequests = useMemo(() => {
    if (!serverId) {
      return [];
    }
    const unique = new Map<string, { serverId: string; cwd: string }>();
    for (const project of projectsForRender) {
      const cwd = project.iconWorkingDir.trim();
      if (!cwd) {
        continue;
      }
      unique.set(`${serverId}:${cwd}`, { serverId, cwd });
    }
    return Array.from(unique.values());
  }, [projectsForRender, serverId]);

  const projectIconQueries = useQueries({
    queries: projectIconRequests.map((request) => ({
      queryKey: projectIconQueryKey(request.serverId, request.cwd),
      queryFn: async () => {
        const client = getHostRuntimeStore().getClient(request.serverId);
        if (!client) {
          return null;
        }
        const result = await client.requestProjectIcon(request.cwd);
        return result.icon;
      },
      select: toProjectIconDataUri,
      enabled: Boolean(
        getHostRuntimeStore().getClient(request.serverId) &&
          isHostRuntimeConnected(getHostRuntimeStore().getSnapshot(request.serverId)) &&
          request.cwd,
      ),
      staleTime: Infinity,
      gcTime: 1000 * 60 * 60,
      refetchOnMount: false,
      refetchOnWindowFocus: false,
      refetchOnReconnect: false,
    })),
  });

  const projectIconByProjectKey = useMemo(() => {
    const iconByServerAndCwd = new Map<string, string | null>();
    for (let index = 0; index < projectIconRequests.length; index += 1) {
      const request = projectIconRequests[index];
      if (!request) {
        continue;
      }
      iconByServerAndCwd.set(
        `${request.serverId}:${request.cwd}`,
        projectIconQueries[index]?.data ?? null,
      );
    }

    const byProject = new Map<string, string | null>();
    for (const project of projectsForRender) {
      const cwd = project.iconWorkingDir.trim();
      if (!cwd || !serverId) {
        byProject.set(project.projectKey, null);
        continue;
      }
      byProject.set(project.projectKey, iconByServerAndCwd.get(`${serverId}:${cwd}`) ?? null);
    }

    return byProject;
  }, [projectIconQueries, projectIconRequests, projectsForRender, serverId]);

  useEffect(() => {
    if (!serverId) {
      return;
    }

    const replacements = findPlaceholderToRealWorkspaceReplacements(projects);
    if (replacements.size === 0) {
      return;
    }

    const activeSelection = getNavigationActiveWorkspaceSelection();
    if (activeSelection && activeSelection.serverId === serverId) {
      const replacement = replacements.get(activeSelection.workspaceId);
      if (replacement && replacement.workspaceId !== activeSelection.workspaceId) {
        // A websocket `workspace_update` can land before the local quick-create response.
        // Preserve the focused tab target so the later `onCreated` navigation stays idempotent.
        navigateToReplacementWorkspacePreservingFocusedTab({
          serverId,
          fromWorkspaceId: activeSelection.workspaceId,
          toWorkspaceId: replacement.workspaceId,
          navigationMethod: "replace",
        });
      }
    }

    setWorkspaces(serverId, (current) => {
      let changed = false;
      const next = new Map(current);
      for (const workspaceId of replacements.keys()) {
        if (next.delete(workspaceId)) {
          changed = true;
        }
      }
      return changed ? next : current;
    });
  }, [projects, serverId, setWorkspaces]);

  const handleProjectDragEnd = useCallback(
    (reorderedProjects: SidebarProjectEntry[]) => {
      if (!serverId) {
        return;
      }

      const reorderedProjectKeys = reorderedProjects.map((project) => project.projectKey);
      const currentProjectOrder = getProjectOrder(serverId);
      if (
        !hasVisibleOrderChanged({
          currentOrder: currentProjectOrder,
          reorderedVisibleKeys: reorderedProjectKeys,
        })
      ) {
        return;
      }

      setProjectOrder(
        serverId,
        mergeWithRemainder({
          currentOrder: currentProjectOrder,
          reorderedVisibleKeys: reorderedProjectKeys,
        }),
      );
    },
    [getProjectOrder, serverId, setProjectOrder],
  );

  const handleWorkspaceReorder = useCallback(
    (projectKey: string, reorderedWorkspaces: SidebarWorkspaceEntry[]) => {
      if (!serverId) {
        return;
      }

      const reorderedWorkspaceKeys = reorderedWorkspaces.map((workspace) => workspace.workspaceKey);
      const currentWorkspaceOrder = getWorkspaceOrder(serverId, projectKey);
      if (
        !hasVisibleOrderChanged({
          currentOrder: currentWorkspaceOrder,
          reorderedVisibleKeys: reorderedWorkspaceKeys,
        })
      ) {
        return;
      }

      setWorkspaceOrder(
        serverId,
        projectKey,
        mergeWithRemainder({
          currentOrder: currentWorkspaceOrder,
          reorderedVisibleKeys: reorderedWorkspaceKeys,
        }),
      );
    },
    [getWorkspaceOrder, serverId, setWorkspaceOrder],
  );

  const renderProject = useCallback(
    ({ item, drag, isActive, dragHandleProps }: DraggableRenderItemInfo<SidebarProjectEntry>) => {
      return (
        <ProjectBlock
          project={item}
          collapsed={collapsedProjectKeys.has(item.projectKey)}
          displayName={projectNameOverrides[item.projectKey] || item.projectName}
          iconDataUri={projectIconByProjectKey.get(item.projectKey) ?? null}
          serverId={serverId}
          selectionEnabled={selectionEnabled}
          showShortcutBadges={showShortcutBadges}
          shortcutIndexByWorkspaceKey={shortcutIndexByWorkspaceKey}
          parentGestureRef={parentGestureRef}
          onToggleCollapsed={() => onToggleProjectCollapsed(item.projectKey)}
          onWorkspacePress={onWorkspacePress}
          onWorkspaceReorder={handleWorkspaceReorder}
          currentPathname={pathname}
          drag={drag}
          isDragging={isActive}
          dragHandleProps={dragHandleProps}
          useNestable={platformIsNative}
        />
      );
    },
    [
      collapsedProjectKeys,
      handleWorkspaceReorder,
      onWorkspacePress,
      onToggleProjectCollapsed,
      parentGestureRef,
      pathname,
      projectIconByProjectKey,
      selectionEnabled,
      serverId,
      shortcutIndexByWorkspaceKey,
      showShortcutBadges,
      platformIsNative,
    ],
  );

  const content = (
    <>
      {projectsForRender.length === 0 ? (
        <View style={styles.emptyContainer}>
          <Text style={styles.emptyTitle}>{text.noProjectsYet}</Text>
          <Text style={styles.emptyText}>{text.addProjectToStart}</Text>
          <Button variant="ghost" size="sm" leftIcon={Plus} onPress={onAddProject}>
            {text.addProject}
          </Button>
        </View>
      ) : (
        <DraggableList
          testID="sidebar-project-list"
          data={projectsForRender}
          keyExtractor={projectKeyExtractor}
          renderItem={renderProject}
          onDragEnd={handleProjectDragEnd}
          scrollEnabled={false}
          useDragHandle
          nestable={platformIsNative}
          simultaneousGestureRef={parentGestureRef}
          containerStyle={styles.projectListContainer}
        />
      )}
      {listFooterComponent}
    </>
  );

  return (
    <View style={styles.container}>
      {platformIsNative ? (
        <NestableScrollContainer
          style={styles.list}
          contentContainerStyle={styles.listContent}
          showsVerticalScrollIndicator={false}
          testID="sidebar-project-workspace-list-scroll"
        >
          {content}
        </NestableScrollContainer>
      ) : (
        <ScrollView
          style={styles.list}
          contentContainerStyle={styles.listContent}
          showsVerticalScrollIndicator={false}
          testID="sidebar-project-workspace-list-scroll"
        >
          {content}
        </ScrollView>
      )}
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  container: {
    flex: 1,
  },
  list: {
    flex: 1,
  },
  listContent: {
    paddingHorizontal: theme.spacing[2],
    paddingTop: theme.spacing[2],
    paddingBottom: theme.spacing[4],
  },
  projectListContainer: {
    width: "100%",
  },
  projectBlock: {
    marginBottom: theme.spacing[1],
  },
  workspaceListContainer: {},
  emptyContainer: {
    marginHorizontal: theme.spacing[2],
    marginTop: theme.spacing[4],
    paddingTop: theme.spacing[6],
    paddingBottom: theme.spacing[4],
    paddingHorizontal: theme.spacing[4],
    borderRadius: theme.borderRadius.lg,
    backgroundColor: theme.colors.surface0,
    alignItems: "center",
    gap: theme.spacing[3],
  },
  emptyTitle: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.medium,
    textAlign: "center",
  },
  emptyText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    textAlign: "center",
  },
  projectRow: {
    minHeight: 36,
    paddingVertical: theme.spacing[2],
    paddingHorizontal: theme.spacing[2],
    borderRadius: theme.borderRadius.lg,
    marginBottom: theme.spacing[1],
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: theme.spacing[2],
    userSelect: "none",
  },
  projectRowHovered: {
    backgroundColor: theme.colors.surfaceSidebarHover,
  },
  projectRowPressed: {
    backgroundColor: theme.colors.surface2,
  },
  projectRowDragging: {
    backgroundColor: theme.colors.surface2,
    borderWidth: 1,
    borderColor: theme.colors.border,
    transform: [{ scale: 1.02 }],
    zIndex: 3,
    ...theme.shadow.md,
  },
  projectRowLeft: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    flex: 1,
    minWidth: 0,
  },
  projectTitleGroup: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[1],
    flex: 1,
    minWidth: 0,
  },
  projectIcon: {
    width: "100%",
    height: "100%",
    borderRadius: theme.borderRadius.sm,
  },
  projectLeadingVisualSlot: {
    position: "relative",
    width: theme.iconSize.md,
    height: theme.iconSize.md,
    flexShrink: 0,
    alignItems: "center",
    justifyContent: "center",
  },
  projectIconFallback: {
    width: "100%",
    height: "100%",
    borderRadius: theme.borderRadius.sm,
    borderWidth: 1,
    borderColor: theme.colors.border,
    alignItems: "center",
    justifyContent: "center",
  },
  projectIconFallbackText: {
    color: theme.colors.foregroundMuted,
    fontSize: 9,
  },
  projectTitle: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
    fontWeight: "400",
    minWidth: 0,
    flexShrink: 1,
  },
  projectActionButton: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[1],
    paddingHorizontal: theme.spacing[2],
    paddingVertical: theme.spacing[1],
    borderRadius: theme.borderRadius.md,
    flexShrink: 0,
  },
  projectActionButtonHovered: {
    backgroundColor: theme.colors.surfaceSidebarHover,
  },
  projectActionButtonText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  projectIconActionButton: {
    width: 24,
    height: 24,
    borderRadius: theme.borderRadius.md,
    alignItems: "center",
    justifyContent: "center",
    flexShrink: 0,
  },
  projectIconActionButtonHovered: {
    backgroundColor: theme.colors.surfaceSidebarHover,
  },
  projectIconActionButtonHidden: {
    opacity: 0,
  },
  projectTrailingActions: {
    flexDirection: "row",
    alignItems: "center",
    gap: 2,
    flexShrink: 0,
  },
  projectKebabButton: {
    width: 24,
    height: 24,
    borderRadius: theme.borderRadius.md,
    alignItems: "center",
    justifyContent: "center",
    flexShrink: 0,
  },
  projectKebabButtonHidden: {
    opacity: 0,
  },
  projectKebabButtonHovered: {
    backgroundColor: theme.colors.surface2,
  },
  projectTrailingControlSlot: {
    width: 24,
    height: 24,
    alignItems: "center",
    justifyContent: "center",
    flexShrink: 0,
  },
  projectActionTooltipRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
  },
  projectActionTooltipText: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
  },
  projectActionTooltipShortcut: {
    backgroundColor: theme.colors.surface3,
    borderColor: theme.colors.borderAccent,
  },
  workspaceRow: {
    minHeight: 36,
    marginBottom: theme.spacing[1],
    paddingVertical: theme.spacing[2],
    paddingLeft: theme.spacing[3] + theme.spacing[3],
    paddingRight: theme.spacing[3],
    borderRadius: theme.borderRadius.lg,
    flexDirection: "column",
    alignItems: "stretch",
    justifyContent: "center",
    gap: theme.spacing[1],
    userSelect: "none",
  },
  workspaceRowMain: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: theme.spacing[2],
    width: "100%",
  },
  workspaceRowLeft: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    flex: 1,
    minWidth: 0,
  },
  workspaceRowRight: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    flexShrink: 0,
  },
  workspaceRowHovered: {
    backgroundColor: theme.colors.surfaceSidebarHover,
  },
  workspaceRowPressed: {
    backgroundColor: theme.colors.surface2,
  },
  workspaceRowDragging: {
    backgroundColor: theme.colors.surface2,
    borderWidth: 1,
    borderColor: theme.colors.border,
    transform: [{ scale: 1.02 }],
    zIndex: 3,
    ...theme.shadow.md,
  },
  sidebarRowSelected: {
    backgroundColor: theme.colors.surfaceSidebarHover,
  },
  workspaceRowContainer: {
    position: "relative",
  },
  workspaceStatusDot: {
    position: "relative",
    width: WORKSPACE_STATUS_DOT_WIDTH,
    height: 16,
    borderRadius: theme.borderRadius.full,
    flexShrink: 0,
    alignItems: "center",
    justifyContent: "center",
  },
  statusDotOverlay: {
    position: "absolute",
    right: DEFAULT_STATUS_DOT_OFFSET,
    bottom: DEFAULT_STATUS_DOT_OFFSET,
    width: DEFAULT_STATUS_DOT_SIZE,
    height: DEFAULT_STATUS_DOT_SIZE,
    borderRadius: theme.borderRadius.full,
    borderWidth: 1,
  },
  workspaceArchivingOverlay: {
    ...StyleSheet.absoluteFillObject,
    borderRadius: theme.borderRadius.lg,
    backgroundColor: `${theme.colors.surface0}cc`,
    alignItems: "center",
    justifyContent: "center",
    flexDirection: "row",
    gap: theme.spacing[2],
    zIndex: 1,
  },
  workspaceArchivingText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    fontWeight: "600",
  },
  workspaceBranchText: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
    fontWeight: "400",
    lineHeight: 20,
    opacity: 0.76,
    flex: 1,
    minWidth: 0,
  },
  workspaceBranchTextCreating: {
    opacity: 0.92,
  },
  workspaceBranchTextHovered: {
    opacity: 1,
  },
  workspacePrBadgeRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    paddingLeft: WORKSPACE_STATUS_DOT_WIDTH + theme.spacing[2],
  },
  workspaceCreatingText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    flexShrink: 0,
  },
  kebabButton: {
    padding: 2,
    borderRadius: 4,
    marginLeft: 2,
  },
  kebabButtonHovered: {
    backgroundColor: theme.colors.surface2,
  },
  shortcutBadge: {
    minWidth: 18,
    height: 18,
    paddingHorizontal: theme.spacing[1],
    alignItems: "center",
    justifyContent: "center",
    borderRadius: theme.borderRadius.sm,
    borderWidth: 1,
    borderColor: theme.colors.surface2,
    backgroundColor: theme.colors.surface0,
    flexShrink: 0,
  },
  shortcutBadgeText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.medium,
    lineHeight: 14,
  },
}));

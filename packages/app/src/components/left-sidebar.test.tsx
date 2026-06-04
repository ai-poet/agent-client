/**
 * @vitest-environment jsdom
 */
import React from "react";
import { act } from "@testing-library/react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LeftSidebar } from "./left-sidebar";

const {
  panelState,
  useSidebarWorkspacesListMock,
  useNavigationRecentWorkspaceSelectionMock,
  openProjectPickerMock,
  navigateToPreparedWorkspaceTabMock,
  resolveNewChatTargetMock,
  routerPushMock,
  theme,
} = vi.hoisted(() => {
  (globalThis as typeof globalThis & { __DEV__?: boolean }).__DEV__ = false;

  const panelState = {
    isOpen: false,
    showMobileAgent: vi.fn(),
  };

  return {
    panelState,
    useSidebarWorkspacesListMock: vi.fn(),
    useNavigationRecentWorkspaceSelectionMock: vi.fn(),
    openProjectPickerMock: vi.fn(),
    navigateToPreparedWorkspaceTabMock: vi.fn(),
    resolveNewChatTargetMock: vi.fn(),
    routerPushMock: vi.fn(),
    theme: {
      spacing: { 0: 0, 0.5: 2, 1: 4, 1.5: 6, 2: 8, 3: 12, 4: 16, 5: 20 },
      iconSize: { sm: 14, md: 18, lg: 22 },
      borderWidth: { 1: 1 },
      borderRadius: { sm: 4, md: 6, lg: 8, full: 999 },
      fontSize: { xs: 11, sm: 13, base: 15 },
      fontWeight: { normal: "400", medium: "500", semibold: "600" },
      shadow: { md: {} },
      colors: {
        surfaceSidebar: "#111",
        surface1: "#111",
        surface2: "#222",
        surface3: "#333",
        surface4: "#444",
        foreground: "#fff",
        foregroundMuted: "#aaa",
        border: "#555",
        borderAccent: "#666",
        accent: "#0a84ff",
        accentForeground: "#fff",
        palette: {
          green: { 400: "#30d158" },
          amber: { 500: "#ffd60a" },
          red: { 500: "#ff453a" },
        },
      },
    },
  };
});

vi.mock("react-native-unistyles", () => ({
  StyleSheet: {
    absoluteFillObject: {},
    create: (factory: unknown) => (typeof factory === "function" ? factory(theme) : factory),
  },
  useUnistyles: () => ({ theme }),
}));

vi.mock("react-native-reanimated", () => ({
  default: {
    View: "div",
  },
  Extrapolation: { CLAMP: "clamp" },
  Keyframe: class {
    duration() {
      return this;
    }
  },
  interpolate: () => 0,
  runOnJS: (fn: (...args: unknown[]) => unknown) => fn,
  useAnimatedStyle: (factory: () => unknown) => factory(),
  useSharedValue: (value: unknown) => ({ value }),
}));

vi.mock("react-native-gesture-handler", () => {
  const chain = {
    enabled: () => chain,
    hitSlop: () => chain,
    manualActivation: () => chain,
    onTouchesDown: () => chain,
    onTouchesMove: () => chain,
    onStart: () => chain,
    onUpdate: () => chain,
    onEnd: () => chain,
    onFinalize: () => chain,
    withRef: () => chain,
  };
  return {
    Gesture: { Pan: () => chain },
    GestureDetector: ({ children }: { children: React.ReactNode }) => children,
  };
});

function normalizeStyle(style: unknown): Record<string, unknown> | undefined {
  if (Array.isArray(style)) {
    return Object.assign(
      {},
      ...style.filter((item) => typeof item === "object" && item !== null && !Array.isArray(item)),
    );
  }
  return typeof style === "object" && style !== null
    ? (style as Record<string, unknown>)
    : undefined;
}

function mapNativeProps(props: Record<string, unknown>): Record<string, unknown> {
  const {
    accessibilityLabel,
    accessibilityRole,
    accessible,
    children,
    collapsable,
    disabled,
    nativeID,
    numberOfLines,
    onPress,
    showsVerticalScrollIndicator,
    style,
    testID,
    ...rest
  } = props;
  const resolvedStyle =
    typeof style === "function" ? style({ hovered: false, pressed: false }) : style;
  return {
    ...rest,
    ...(normalizeStyle(resolvedStyle) ? { style: normalizeStyle(resolvedStyle) } : {}),
    ...(typeof accessibilityLabel === "string" ? { "aria-label": accessibilityLabel } : {}),
    ...(accessibilityRole === "button" ? { role: "button" } : {}),
    ...(typeof testID === "string" ? { "data-testid": testID } : {}),
    children,
    disabled: Boolean(disabled) || undefined,
    onClick:
      typeof onPress === "function"
        ? (event: React.MouseEvent) => onPress({ stopPropagation: () => event.stopPropagation() })
        : undefined,
  };
}

vi.mock("react-native", () => ({
  Modal: ({ children, visible }: { children: React.ReactNode; visible: boolean }) =>
    visible ? React.createElement("div", null, children) : null,
  Platform: {
    OS: "web",
    select: (specifics: Record<string, unknown>) => specifics.web ?? specifics.default,
  },
  Pressable: (props: Record<string, unknown>) => {
    const children =
      typeof props.children === "function"
        ? props.children({ hovered: false, pressed: false, open: false })
        : props.children;
    return React.createElement("button", mapNativeProps({ ...props, children }));
  },
  ScrollView: ({ children, contentContainerStyle, ...props }: Record<string, unknown>) =>
    React.createElement(
      "div",
      mapNativeProps({
        ...props,
        style: contentContainerStyle ?? props.style,
        children,
      }),
    ),
  Text: (props: Record<string, unknown>) => React.createElement("span", mapNativeProps(props)),
  useWindowDimensions: () => ({ width: 390, height: 844 }),
  View: React.forwardRef<HTMLDivElement, Record<string, unknown>>((props, ref) =>
    React.createElement("div", { ...mapNativeProps(props), ref }),
  ),
  StyleSheet: {
    absoluteFillObject: {},
    create: (styles: unknown) => styles,
  },
}));

vi.mock("lucide-react-native", () => {
  const createIcon = (name: string) => {
    function MockIcon(props: Record<string, unknown>) {
      return React.createElement("span", { ...props, "data-icon": name });
    }
    MockIcon.displayName = name;
    return MockIcon;
  };
  return {
    ArrowDownNarrowWide: createIcon("ArrowDownNarrowWide"),
    Check: createIcon("Check"),
    CheckCircle: createIcon("CheckCircle"),
    ChevronsDownUp: createIcon("ChevronsDownUp"),
    Cloud: createIcon("Cloud"),
    Folder: createIcon("Folder"),
    FolderPlus: createIcon("FolderPlus"),
    MessageSquarePlus: createIcon("MessageSquarePlus"),
    Box: createIcon("Box"),
    MessagesSquare: createIcon("MessagesSquare"),
    Plus: createIcon("Plus"),
    Settings: createIcon("Settings"),
  };
});

vi.mock("expo-router", () => ({
  router: { push: routerPushMock },
  usePathname: () => "/hosts/srv",
}));

vi.mock("react-native-safe-area-context", () => ({
  useSafeAreaInsets: () => ({ top: 0, right: 0, bottom: 0, left: 0 }),
}));

vi.mock("@/constants/layout", () => ({
  useIsCompactFormFactor: () => true,
}));

vi.mock("@/constants/platform", () => ({
  isWeb: true,
  isNative: false,
}));

vi.mock("@/config/branding", () => ({
  CLOUD_NAME: "Paseo Cloud",
}));

vi.mock("@/stores/panel-store", () => ({
  MIN_SIDEBAR_WIDTH: 260,
  MAX_SIDEBAR_WIDTH: 420,
  selectIsAgentListOpen: (state: typeof panelState) => state.isOpen,
  usePanelStore: (selector: (state: typeof panelState) => unknown) => selector(panelState),
}));

vi.mock("@/runtime/host-runtime", () => ({
  useHosts: () => [{ serverId: "srv", label: "Local" }],
  useHostRuntimeSnapshot: () => ({ connectionStatus: "online" }),
}));

vi.mock("@/hooks/use-sidebar-workspaces-list", () => ({
  useSidebarWorkspacesList: useSidebarWorkspacesListMock,
}));

vi.mock("@/hooks/use-sidebar-shortcut-model", () => ({
  useSidebarShortcutModel: () => ({
    collapsedProjectKeys: new Set<string>(),
    shortcutIndexByWorkspaceKey: new Map<string, number>(),
    toggleProjectCollapsed: vi.fn(),
  }),
}));

vi.mock("@/contexts/sidebar-animation-context", () => ({
  useSidebarAnimation: () => ({
    translateX: { value: 0 },
    backdropOpacity: { value: 0 },
    windowWidth: 390,
    animateToOpen: vi.fn(),
    animateToClose: vi.fn(),
    isGesturing: { value: false },
    gestureAnimatingRef: { current: false },
    closeGestureRef: { current: undefined },
  }),
}));

vi.mock("@/hooks/use-shortcut-keys", () => ({
  useShortcutKeys: () => null,
}));

vi.mock("@/utils/desktop-window", () => ({
  useWindowControlsPadding: () => ({ top: 0 }),
}));

vi.mock("@/utils/host-routes", () => ({
  buildHostSkillsRoute: (serverId: string) => `/h/${serverId}/skills`,
  buildHostSessionsRoute: (serverId: string) => `/hosts/${serverId}/sessions`,
  buildPaseoCloudRoute: () => "/settings/paseo-cloud",
  buildSettingsRoute: () => "/settings",
  mapPathnameToServer: (_pathname: string, serverId: string) => `/hosts/${serverId}`,
  parseServerIdFromPathname: () => "srv",
}));

vi.mock("@/stores/draft-keys", () => ({
  generateDraftId: () => "draft-test",
}));

vi.mock("@/stores/session-store", () => ({
  useSessionStore: {
    getState: () => ({ sessions: {} }),
  },
}));

vi.mock("@/utils/new-agent-routing", () => ({
  buildNewAgentRoute: (serverId: string, workingDir?: string | null) =>
    `/h/${serverId}/workspace/${workingDir || "."}`,
  resolveNewChatTarget: resolveNewChatTargetMock,
}));

vi.mock("@/utils/workspace-navigation", () => ({
  navigateToPreparedWorkspaceTab: navigateToPreparedWorkspaceTabMock,
}));

vi.mock("@/hooks/use-sub2api-locale", () => ({
  useSub2APILocale: () => "en",
}));

vi.mock("@/hooks/use-open-project-picker", () => ({
  useOpenProjectPicker: () => openProjectPickerMock,
}));

vi.mock("@/stores/navigation-active-workspace-store", () => ({
  useNavigationRecentWorkspaceSelection: useNavigationRecentWorkspaceSelectionMock,
}));

vi.mock("@/components/sidebar/sidebar-header-row", () => ({
  SidebarHeaderRow: ({ label }: { label: string }) => React.createElement("div", null, label),
}));

vi.mock("./sidebar-workspace-list", () => ({
  SidebarWorkspaceList: ({ projects }: { projects: { projectName: string }[] }) =>
    React.createElement(
      "div",
      { "data-testid": "sidebar-workspace-list" },
      projects.map((project) => project.projectName).join(","),
    ),
}));

vi.mock("./sidebar-agent-list-skeleton", () => ({
  SidebarAgentListSkeleton: () => React.createElement("div", null, "Loading"),
}));

vi.mock("@/components/ui/tooltip", () => ({
  Tooltip: ({ children }: { children: React.ReactNode }) =>
    React.createElement("div", null, children),
  TooltipContent: ({ children }: { children: React.ReactNode }) =>
    React.createElement("div", null, children),
  TooltipTrigger: ({ children }: { children: React.ReactNode }) =>
    React.createElement("div", null, children),
}));

vi.mock("@/components/ui/dropdown-menu", () => ({
  DropdownMenu: ({ children }: { children: React.ReactNode }) =>
    React.createElement("div", null, children),
  DropdownMenuTrigger: ({
    children,
  }: {
    children: React.ReactNode | ((state: unknown) => React.ReactNode);
  }) =>
    React.createElement("button", null, typeof children === "function" ? children({}) : children),
  DropdownMenuContent: ({ children }: { children: React.ReactNode }) =>
    React.createElement("div", null, children),
  DropdownMenuItem: ({ children }: { children: React.ReactNode }) =>
    React.createElement("div", null, children),
}));

vi.mock("@/components/ui/shortcut", () => ({
  Shortcut: () => React.createElement("span", null),
}));

vi.mock("@/components/ui/combobox", () => ({
  Combobox: () => null,
  ComboboxItem: ({ label }: { label: string }) => React.createElement("div", null, label),
}));

vi.mock("@/components/sidebar-user-menu", () => ({
  SidebarUserMenu: () => React.createElement("div", null),
}));

vi.stubGlobal("React", React);

describe("LeftSidebar", () => {
  let root: Root | null = null;
  let container: HTMLElement | null = null;

  beforeEach(() => {
    panelState.isOpen = false;
    panelState.showMobileAgent.mockReset();
    routerPushMock.mockReset();
    openProjectPickerMock.mockReset();
    navigateToPreparedWorkspaceTabMock.mockReset();
    resolveNewChatTargetMock.mockReset();
    useNavigationRecentWorkspaceSelectionMock.mockReset();
    useNavigationRecentWorkspaceSelectionMock.mockReturnValue(null);
    resolveNewChatTargetMock.mockReturnValue({
      kind: "workspace",
      serverId: "srv",
      workspaceId: "workspace-1",
    });
    useSidebarWorkspacesListMock.mockReset();
    useSidebarWorkspacesListMock.mockReturnValue({
      projects: [
        {
          projectKey: "project-1",
          projectName: "Project 1",
          workspaces: [
            {
              serverId: "srv",
              workspaceId: "workspace-visible",
            },
          ],
        },
      ],
      isInitialLoad: false,
      isRevalidating: false,
      refreshAll: vi.fn(),
    });
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    if (root) {
      act(() => {
        root?.unmount();
      });
    }
    root = null;
    container?.remove();
    container = null;
  });

  it("keeps the mobile workspace list subscribed while the sidebar is hidden", async () => {
    await act(async () => {
      root?.render(<LeftSidebar />);
    });

    expect(useSidebarWorkspacesListMock).toHaveBeenLastCalledWith({
      serverId: "srv",
      enabled: true,
    });
  });

  it("renders new chat as the primary action without the old chat section label", async () => {
    await act(async () => {
      root?.render(<LeftSidebar />);
    });

    expect(container?.querySelector('[data-testid="sidebar-new-chat"]')).not.toBeNull();
    expect(container?.textContent).toContain("New chat");
    expect(container?.textContent).not.toContain("Chat");
  });

  it("navigates to the Skill Library page from the sidebar skills action", async () => {
    await act(async () => {
      root?.render(<LeftSidebar />);
    });

    await act(async () => {
      container
        ?.querySelector<HTMLButtonElement>('[data-testid="sidebar-skill-library"]')
        ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(routerPushMock).toHaveBeenCalledWith("/h/srv/skills");
  });

  it("opens a focused draft tab for the current workspace", async () => {
    await act(async () => {
      root?.render(<LeftSidebar selectedAgentId="srv:agent-1" />);
    });

    await act(async () => {
      container
        ?.querySelector<HTMLButtonElement>('[data-testid="sidebar-new-chat"]')
        ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(panelState.showMobileAgent).toHaveBeenCalledTimes(1);
    expect(navigateToPreparedWorkspaceTabMock).toHaveBeenCalledWith({
      serverId: "srv",
      workspaceId: "workspace-1",
      target: { kind: "draft", draftId: "draft-test" },
      navigationMethod: "navigate",
    });
    expect(resolveNewChatTargetMock).toHaveBeenCalledWith(
      expect.objectContaining({
        fallbackWorkspace: {
          serverId: "srv",
          workspaceId: "workspace-visible",
        },
      }),
    );
  });

  it("falls back to the new-agent route when no current workspace is available", async () => {
    resolveNewChatTargetMock.mockReturnValue({
      kind: "fallback",
      serverId: "srv",
      workingDir: "/repo/other",
    });

    await act(async () => {
      root?.render(<LeftSidebar />);
    });

    await act(async () => {
      container
        ?.querySelector<HTMLButtonElement>('[data-testid="sidebar-new-chat"]')
        ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(navigateToPreparedWorkspaceTabMock).not.toHaveBeenCalled();
    expect(routerPushMock).toHaveBeenCalledWith("/h/srv/workspace//repo/other");
  });

  it("opens the project picker instead of routing to a placeholder when no workspace exists", async () => {
    resolveNewChatTargetMock.mockReturnValue({
      kind: "fallback",
      serverId: "srv",
      workingDir: null,
    });

    await act(async () => {
      root?.render(<LeftSidebar />);
    });

    await act(async () => {
      container
        ?.querySelector<HTMLButtonElement>('[data-testid="sidebar-new-chat"]')
        ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(navigateToPreparedWorkspaceTabMock).not.toHaveBeenCalled();
    expect(routerPushMock).not.toHaveBeenCalled();
    expect(openProjectPickerMock).toHaveBeenCalledTimes(1);
  });
});

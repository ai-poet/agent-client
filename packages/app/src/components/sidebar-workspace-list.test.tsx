/**
 * @vitest-environment jsdom
 */
import { act } from "@testing-library/react";
import type { DaemonClient } from "@server/client/daemon-client";
import type { WorkspaceScriptPayload } from "@server/shared/messages";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRoot, type Root } from "react-dom/client";
import React from "react";
import type { ReactElement } from "react";
import { getSidebarRenderableProjects } from "@/utils/sidebar-renderable-projects";

vi.hoisted(() => {
  (globalThis as unknown as { __DEV__: boolean }).__DEV__ = false;
});

import {
  createSidebarWorkspaceEntry,
  type SidebarProjectEntry,
} from "@/hooks/use-sidebar-workspaces-list";
import { useSidebarWorkspacesList } from "@/hooks/use-sidebar-workspaces-list";
import { patchWorkspaceScripts } from "@/contexts/session-workspace-scripts";
import {
  getHostRuntimeStore,
  type HostRuntimeController,
  type HostRuntimeSnapshot,
} from "@/runtime/host-runtime";
import type { HostProfile } from "@/types/host-connection";
import { useSessionStore, type WorkspaceDescriptor } from "@/stores/session-store";
import { useSidebarOrderStore } from "@/stores/sidebar-order-store";
import { useWorkspaceFields } from "@/stores/session-store-hooks";
import {
  activateNavigationWorkspaceSelection,
  syncNavigationActiveWorkspace,
  useIsNavigationProjectActive,
  useIsNavigationWorkspaceSelected,
} from "@/stores/navigation-active-workspace-store";

const { theme } = vi.hoisted(() => ({
  theme: {
    spacing: { 1: 4, 2: 8, 3: 12, 4: 16, 5: 20 },
    iconSize: { sm: 14, md: 18 },
    borderWidth: { 1: 1 },
    borderRadius: { md: 6, lg: 8, full: 999 },
    fontSize: { xs: 11, sm: 13 },
    fontWeight: { normal: "400", medium: "500" },
    opacity: { 50: 0.5 },
    shadow: { md: {} },
    colors: {
      surface0: "#000",
      surface1: "#111",
      surface2: "#222",
      surface3: "#333",
      surface4: "#444",
      surfaceSidebarHover: "#222",
      foreground: "#fff",
      foregroundMuted: "#aaa",
      border: "#333",
      borderAccent: "#444",
      accent: "#0a84ff",
      destructive: "#ff453a",
      palette: {
        blue: { 500: "#0a84ff" },
        green: { 500: "#30d158" },
        purple: { 500: "#bf5af2" },
        red: { 500: "#ff453a" },
      },
    },
  },
}));

vi.mock("react-native-unistyles", () => ({
  StyleSheet: {
    create: (factory: unknown) => (typeof factory === "function" ? factory(theme) : factory),
  },
  useUnistyles: () => ({ theme, rt: { breakpoint: "lg" } }),
}));

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
    children,
    disabled,
    onPress,
    style,
    testID,
    ...rest
  } = props;
  const resolvedStyle =
    typeof style === "function" ? style({ hovered: false, pressed: false, open: false }) : style;
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
  ActivityIndicator: (props: Record<string, unknown>) =>
    React.createElement("span", mapNativeProps(props)),
  Image: (props: Record<string, unknown>) => React.createElement("img", mapNativeProps(props)),
  Platform: { OS: "web", select: (options: Record<string, unknown>) => options.web },
  Pressable: (props: Record<string, unknown>) => {
    const children =
      typeof props.children === "function"
        ? props.children({ hovered: false, pressed: false, open: false })
        : props.children;
    return React.createElement("button", mapNativeProps({ ...props, children }));
  },
  ScrollView: React.forwardRef<HTMLDivElement, Record<string, unknown>>((props, ref) =>
    React.createElement("div", { ...mapNativeProps(props), ref }),
  ),
  StatusBar: { currentHeight: 0 },
  Text: (props: Record<string, unknown>) => React.createElement("span", mapNativeProps(props)),
  View: React.forwardRef<HTMLDivElement, Record<string, unknown>>((props, ref) =>
    React.createElement("div", { ...mapNativeProps(props), ref }),
  ),
}));

vi.mock("@/constants/layout", () => ({
  useIsCompactFormFactor: () => false,
}));

vi.mock("@/constants/platform", () => ({
  getIsElectron: () => false,
  isNative: false,
  isWeb: true,
}));

vi.mock("expo-haptics", () => ({
  impactAsync: vi.fn(async () => {}),
  notificationAsync: vi.fn(async () => {}),
  ImpactFeedbackStyle: { Light: "light" },
  NotificationFeedbackType: { Success: "success" },
}));

vi.mock("lucide-react-native", () => {
  const createIcon = (name: string) => (props: Record<string, unknown>) =>
    React.createElement("span", { ...props, "data-icon": name });
  return {
    Archive: createIcon("Archive"),
    ChevronDown: createIcon("ChevronDown"),
    ChevronRight: createIcon("ChevronRight"),
    CircleAlert: createIcon("CircleAlert"),
    Copy: createIcon("Copy"),
    ExternalLink: createIcon("ExternalLink"),
    FolderGit2: createIcon("FolderGit2"),
    GitPullRequest: createIcon("GitPullRequest"),
    Globe: createIcon("Globe"),
    Monitor: createIcon("Monitor"),
    MoreVertical: createIcon("MoreVertical"),
    Pencil: createIcon("Pencil"),
    Plus: createIcon("Plus"),
    SquareTerminal: createIcon("SquareTerminal"),
    Trash2: createIcon("Trash2"),
  };
});

vi.mock("react-native-reanimated", () => ({
  default: {
    View: "div",
  },
  Easing: { linear: "linear" },
  makeMutable: (value: unknown) => ({ value }),
  runOnJS: (fn: (...args: unknown[]) => unknown) => fn,
  useAnimatedStyle: (factory: () => unknown) => factory(),
  useSharedValue: (value: unknown) => ({ value }),
  withRepeat: (value: unknown) => value,
  withTiming: (value: unknown) => value,
}));

vi.mock("react-native-gesture-handler", () => ({
  Gesture: {},
}));

vi.mock("@/components/ui/tooltip", () => ({
  Tooltip: ({ children }: { children: React.ReactNode }) =>
    React.createElement("div", null, children),
  TooltipContent: ({ children }: { children: React.ReactNode }) =>
    React.createElement("div", null, children),
  TooltipTrigger: ({ children }: { children: React.ReactNode }) =>
    React.createElement(React.Fragment, null, children),
}));

vi.mock("@/components/ui/dropdown-menu", () => ({
  DropdownMenu: ({ children }: { children: React.ReactNode }) =>
    React.createElement("div", null, children),
  DropdownMenuContent: ({ children }: { children: React.ReactNode }) =>
    React.createElement("div", null, children),
  DropdownMenuItem: ({
    children,
    onSelect,
    testID,
  }: {
    children: React.ReactNode;
    onSelect?: () => void;
    testID?: string;
  }) => React.createElement("button", { "data-testid": testID, onClick: onSelect }, children),
  DropdownMenuTrigger: ({
    children,
    testID,
  }: {
    children: React.ReactNode | ((state: unknown) => React.ReactNode);
    testID?: string;
  }) =>
    React.createElement(
      "button",
      { "data-testid": testID },
      typeof children === "function" ? children({ hovered: false, pressed: false }) : children,
    ),
}));

vi.mock("@/components/ui/context-menu", () => ({
  ContextMenu: ({ children }: { children: React.ReactNode }) =>
    React.createElement("div", null, children),
  ContextMenuContent: ({ children }: { children: React.ReactNode }) =>
    React.createElement("div", null, children),
  ContextMenuItem: ({ children }: { children: React.ReactNode }) =>
    React.createElement("button", null, children),
  ContextMenuTrigger: ({ children }: { children: React.ReactNode }) =>
    React.createElement(React.Fragment, null, children),
  useContextMenu: () => null,
}));

vi.mock("@/components/onboarding-guide-target", () => ({
  OnboardingGuideTarget: ({ children }: { children: React.ReactNode }) =>
    React.createElement(React.Fragment, null, children),
}));

vi.mock("@/components/synced-loader", () => ({
  SyncedLoader: () => React.createElement("span", null),
}));

vi.mock("@/components/diff-stat", () => ({
  DiffStat: () => React.createElement("span", null),
}));

vi.mock("@/components/ui/shortcut", () => ({
  Shortcut: () => React.createElement("span", null),
}));

vi.mock("@/components/workspace-hover-card", () => ({
  WorkspaceHoverCard: ({ children }: { children: React.ReactNode }) =>
    React.createElement(React.Fragment, null, children),
}));

vi.mock("@/components/icons/github-icon", () => ({
  GitHubIcon: () => React.createElement("span", null),
}));

vi.mock("@react-native-async-storage/async-storage", () => ({
  default: {
    getItem: vi.fn(),
    setItem: vi.fn(),
    removeItem: vi.fn(),
  },
}));

vi.mock("expo-router", () => ({
  router: {
    navigate: vi.fn(),
    replace: vi.fn(),
  },
  usePathname: () => "/h/sidebar-render-count/workspace/a-main",
}));

vi.mock("expo-clipboard", () => ({
  setStringAsync: vi.fn(async () => true),
}));

vi.mock("expo-constants", () => ({
  default: {
    expoConfig: { version: "0.0.0-test" },
    manifest: null,
    nativeAppVersion: null,
  },
}));

vi.mock("react-native-draggable-flatlist", () => ({
  NestableScrollContainer: ({ children }: { children: React.ReactNode }) =>
    React.createElement("div", null, children),
}));

vi.mock("./draggable-list", () => ({
  DraggableList: ({
    data,
    keyExtractor,
    renderItem,
    testID,
    ListFooterComponent,
    ListHeaderComponent,
    ListEmptyComponent,
  }: {
    data: unknown[];
    keyExtractor: (item: unknown, index: number) => string;
    renderItem: (input: {
      item: unknown;
      index: number;
      drag: () => void;
      isActive: boolean;
    }) => React.ReactNode;
    testID?: string;
    ListFooterComponent?: React.ReactNode;
    ListHeaderComponent?: React.ReactNode;
    ListEmptyComponent?: React.ReactNode;
  }) =>
    React.createElement(
      "div",
      { "data-testid": testID },
      ListHeaderComponent,
      data.length === 0 ? ListEmptyComponent : null,
      data.map((item, index) =>
        React.createElement(
          React.Fragment,
          { key: keyExtractor(item, index) },
          renderItem({ item, index, drag: vi.fn(), isActive: false }),
        ),
      ),
      ListFooterComponent,
    ),
}));

vi.mock("@/hooks/use-sub2api-locale", () => ({
  useSub2APILocale: () => "en",
}));

const SERVER_ID = "sidebar-render-count";

type RenderCounts = {
  frame: number;
  headers: Record<string, number>;
  rows: Record<string, number>;
  projectSelection: Record<string, number>;
  rowSelection: Record<string, number>;
};

const runningScript: WorkspaceScriptPayload = {
  scriptName: "web",
  type: "service",
  hostname: "web.paseo.localhost",
  port: 3000,
  proxyUrl: "http://web.paseo.localhost:6767",
  lifecycle: "running",
  health: "healthy",
  exitCode: null,
  terminalId: null,
};

function workspace(input: {
  id: string;
  projectId: string;
  projectDisplayName: string;
  name: string;
  status?: WorkspaceDescriptor["status"];
  scripts?: WorkspaceDescriptor["scripts"];
}): WorkspaceDescriptor {
  return {
    id: input.id,
    projectId: input.projectId,
    projectDisplayName: input.projectDisplayName,
    projectRootPath: `/repo/${input.projectId}`,
    workspaceDirectory: `/repo/${input.projectId}/${input.id}`,
    projectKind: "git",
    workspaceKind: input.name === "main" ? "local_checkout" : "worktree",
    name: input.name,
    status: input.status ?? "done",
    diffStat: null,
    scripts: input.scripts ?? [],
  };
}

function createWorkspaces(): WorkspaceDescriptor[] {
  return [
    workspace({
      id: "a-main",
      projectId: "project-a",
      projectDisplayName: "Project A",
      name: "main",
      scripts: [runningScript],
    }),
    workspace({
      id: "a-one",
      projectId: "project-a",
      projectDisplayName: "Project A",
      name: "one",
    }),
    workspace({
      id: "a-two",
      projectId: "project-a",
      projectDisplayName: "Project A",
      name: "two",
    }),
    workspace({
      id: "b-main",
      projectId: "project-b",
      projectDisplayName: "Project B",
      name: "main",
    }),
    workspace({
      id: "b-one",
      projectId: "project-b",
      projectDisplayName: "Project B",
      name: "one",
    }),
    workspace({
      id: "b-two",
      projectId: "project-b",
      projectDisplayName: "Project B",
      name: "two",
    }),
  ];
}

function makeHost(): HostProfile {
  const now = "2026-04-19T00:00:00.000Z";
  return {
    serverId: SERVER_ID,
    label: "Render Count Host",
    lifecycle: {},
    connections: [],
    preferredConnectionId: null,
    createdAt: now,
    updatedAt: now,
  };
}

function initializeSidebarState(workspaces: WorkspaceDescriptor[]): void {
  act(() => {
    getHostRuntimeStore().syncHosts([makeHost()]);
    useSessionStore.getState().initializeSession(SERVER_ID, null as unknown as DaemonClient);
    useSessionStore
      .getState()
      .setWorkspaces(SERVER_ID, new Map(workspaces.map((entry) => [entry.id, entry])));
    useSessionStore.getState().setHasHydratedWorkspaces(SERVER_ID, true);
    useSidebarOrderStore.setState({
      projectOrderByServerId: {
        [SERVER_ID]: ["project-a", "project-b"],
      },
      workspaceOrderByServerAndProject: {
        [`${SERVER_ID}::project-a`]: [
          `${SERVER_ID}:a-main`,
          `${SERVER_ID}:a-one`,
          `${SERVER_ID}:a-two`,
        ],
        [`${SERVER_ID}::project-b`]: [
          `${SERVER_ID}:b-main`,
          `${SERVER_ID}:b-one`,
          `${SERVER_ID}:b-two`,
        ],
      },
    });
  });
}

function cleanupSidebarState(): void {
  act(() => {
    syncNavigationActiveWorkspace({ current: null });
    getHostRuntimeStore().syncHosts([]);
    useSessionStore.getState().clearSession(SERVER_ID);
    useSidebarOrderStore.setState({
      projectOrderByServerId: {},
      workspaceOrderByServerAndProject: {},
    });
  });
}

function resetCounts(counts: RenderCounts): void {
  counts.frame = 0;
  counts.headers = {};
  counts.rows = {};
  counts.projectSelection = {};
  counts.rowSelection = {};
}

function incrementRecord(record: Record<string, number>, key: string): void {
  record[key] = (record[key] ?? 0) + 1;
}

function ProjectHeaderProbe({
  project,
  counts,
}: {
  project: SidebarProjectEntry;
  counts: RenderCounts;
}): null {
  incrementRecord(counts.headers, project.projectKey);
  return null;
}

function WorkspaceRowProbe({
  serverId,
  workspaceId,
  counts,
}: {
  serverId: string;
  workspaceId: string;
  counts: RenderCounts;
}): null {
  const workspace = useWorkspaceFields(serverId, workspaceId, (entry) =>
    createSidebarWorkspaceEntry({ serverId, workspace: entry }),
  );
  if (workspace) {
    incrementRecord(counts.rows, workspace.workspaceId);
  }
  return null;
}

function ProjectActiveProbe({
  serverId,
  project,
  counts,
}: {
  serverId: string;
  project: SidebarProjectEntry;
  counts: RenderCounts;
}): null {
  useIsNavigationProjectActive({
    serverId,
    workspaceIds: project.workspaces.map((workspace) => workspace.workspaceId),
  });
  incrementRecord(counts.projectSelection, project.projectKey);
  return null;
}

function WorkspaceSelectionProbe({
  serverId,
  workspaceId,
  counts,
}: {
  serverId: string;
  workspaceId: string;
  counts: RenderCounts;
}): null {
  useIsNavigationWorkspaceSelected({ serverId, workspaceId });
  incrementRecord(counts.rowSelection, workspaceId);
  return null;
}

function SidebarFrameProbe({ counts }: { counts: RenderCounts }): ReactElement {
  counts.frame += 1;
  const { projects } = useSidebarWorkspacesList({ serverId: SERVER_ID });

  return (
    <>
      {projects.map((project) => (
        <div key={project.projectKey}>
          <ProjectHeaderProbe project={project} counts={counts} />
          <ProjectActiveProbe serverId={SERVER_ID} project={project} counts={counts} />
          {project.workspaces.map((workspace) => (
            <React.Fragment key={workspace.workspaceKey}>
              <WorkspaceRowProbe
                serverId={workspace.serverId}
                workspaceId={workspace.workspaceId}
                counts={counts}
              />
              <WorkspaceSelectionProbe
                serverId={workspace.serverId}
                workspaceId={workspace.workspaceId}
                counts={counts}
              />
            </React.Fragment>
          ))}
        </div>
      ))}
    </>
  );
}

function getHostController(): HostRuntimeController {
  const controllers = (
    getHostRuntimeStore() as unknown as {
      controllers: Map<string, HostRuntimeController>;
    }
  ).controllers;
  const controller = controllers.get(SERVER_ID);
  if (!controller) {
    throw new Error("Host runtime controller was not initialized");
  }
  return controller;
}

function updateControllerSnapshot(
  patch: Partial<Omit<HostRuntimeSnapshot, "serverId" | "clientGeneration">>,
): void {
  (
    getHostController() as unknown as {
      updateSnapshot: (
        patch: Partial<Omit<HostRuntimeSnapshot, "serverId" | "clientGeneration">>,
      ) => void;
    }
  ).updateSnapshot(patch);
}

async function renderProbe(counts: RenderCounts): Promise<{ root: Root; container: HTMLElement }> {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(<SidebarFrameProbe counts={counts} />);
  });
  resetCounts(counts);
  return { root, container };
}

async function captureSidebarProjects(): Promise<SidebarProjectEntry[]> {
  let capturedProjects: SidebarProjectEntry[] = [];

  function SidebarProjectsCapture(): null {
    capturedProjects = useSidebarWorkspacesList({ serverId: SERVER_ID }).projects;
    return null;
  }

  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);

  await act(async () => {
    root.render(<SidebarProjectsCapture />);
  });

  act(() => {
    root.unmount();
  });
  container.remove();

  return capturedProjects;
}

describe("sidebar workspace render isolation", () => {
  let root: Root | null = null;
  let container: HTMLElement | null = null;

  beforeEach(async () => {
    initializeSidebarState(createWorkspaces());
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
    cleanupSidebarState();
  });

  it("re-renders only the changed workspace row for a status update", async () => {
    const counts: RenderCounts = {
      frame: 0,
      headers: {},
      rows: {},
      projectSelection: {},
      rowSelection: {},
    };
    ({ root, container } = await renderProbe(counts));

    act(() => {
      useSessionStore.getState().mergeWorkspaces(SERVER_ID, [
        {
          ...createWorkspaces()[1]!,
          status: "running",
        },
      ]);
    });

    expect(counts.frame).toBe(0);
    expect(counts.headers).toEqual({});
    expect(counts.rows).toEqual({ "a-one": 1 });
  });

  it("does not re-render the sidebar for a host-runtime probe tick with no content change", async () => {
    const counts: RenderCounts = {
      frame: 0,
      headers: {},
      rows: {},
      projectSelection: {},
      rowSelection: {},
    };
    ({ root, container } = await renderProbe(counts));

    act(() => {
      const probeByConnectionId = getHostController().getSnapshot().probeByConnectionId;
      updateControllerSnapshot({
        probeByConnectionId: new Map(probeByConnectionId),
      });
    });

    expect(counts).toEqual({
      frame: 0,
      headers: {},
      rows: {},
      projectSelection: {},
      rowSelection: {},
    });
  });

  it("does not re-render for a deep-equal scripts patch", async () => {
    const counts: RenderCounts = {
      frame: 0,
      headers: {},
      rows: {},
      projectSelection: {},
      rowSelection: {},
    };
    ({ root, container } = await renderProbe(counts));

    act(() => {
      useSessionStore.getState().setWorkspaces(SERVER_ID, (current) =>
        patchWorkspaceScripts(current, {
          workspaceId: "a-main",
          scripts: [{ ...runningScript }],
        }),
      );
    });

    expect(counts).toEqual({
      frame: 0,
      headers: {},
      rows: {},
      projectSelection: {},
      rowSelection: {},
    });
  });

  it("isolates active selection updates to affected row and project boolean probes", async () => {
    const counts: RenderCounts = {
      frame: 0,
      headers: {},
      rows: {},
      projectSelection: {},
      rowSelection: {},
    };

    act(() => {
      activateNavigationWorkspaceSelection({
        serverId: SERVER_ID,
        workspaceId: "a-one",
      });
    });
    ({ root, container } = await renderProbe(counts));

    act(() => {
      activateNavigationWorkspaceSelection({
        serverId: SERVER_ID,
        workspaceId: "b-two",
      });
    });

    expect(counts.frame).toBe(0);
    expect(counts.headers).toEqual({});
    expect(counts.rows).toEqual({});
    expect(counts.projectSelection).toEqual({
      "project-a": 1,
      "project-b": 1,
    });
    expect(counts.rowSelection).toEqual({
      "a-one": 1,
      "b-two": 1,
    });
  });
});

describe("getSidebarRenderableProjects", () => {
  it("hides creating placeholders when a same-name real workspace exists", () => {
    const project: SidebarProjectEntry = {
      projectKey: "project-a",
      projectName: "Project A",
      projectKind: "git",
      iconWorkingDir: "/repo/project-a",
      workspaces: [
        {
          workspaceKey: `${SERVER_ID}:__creating_worktree__:demo`,
          serverId: SERVER_ID,
          workspaceId: "__creating_worktree__:demo",
          projectRootPath: "/repo/project-a",
          workspaceDirectory: "/repo/project-a/.paseo/pending/demo",
          projectKind: "git",
          workspaceKind: "worktree",
          name: "feature/demo",
          statusBucket: "running",
          diffStat: null,
          scripts: [],
          hasRunningScripts: false,
        },
        {
          workspaceKey: `${SERVER_ID}:workspace-feature`,
          serverId: SERVER_ID,
          workspaceId: "workspace-feature",
          projectRootPath: "/repo/project-a",
          workspaceDirectory: "/repo/project-a/workspace-feature",
          projectKind: "git",
          workspaceKind: "worktree",
          name: "feature/demo",
          statusBucket: "done",
          diffStat: null,
          scripts: [],
          hasRunningScripts: false,
        },
      ],
    };

    const result = getSidebarRenderableProjects([project]);

    expect(result).toHaveLength(1);
    expect(result[0]?.workspaces).toHaveLength(1);
    expect(result[0]?.workspaces[0]?.workspaceId).toBe("workspace-feature");
  });

  it("keeps creating placeholders when there is no same-name real workspace", () => {
    const project: SidebarProjectEntry = {
      projectKey: "project-a",
      projectName: "Project A",
      projectKind: "git",
      iconWorkingDir: "/repo/project-a",
      workspaces: [
        {
          workspaceKey: `${SERVER_ID}:__creating_worktree__:demo`,
          serverId: SERVER_ID,
          workspaceId: "__creating_worktree__:demo",
          projectRootPath: "/repo/project-a",
          workspaceDirectory: "/repo/project-a/.paseo/pending/demo",
          projectKind: "git",
          workspaceKind: "worktree",
          name: "feature/new",
          statusBucket: "running",
          diffStat: null,
          scripts: [],
          hasRunningScripts: false,
        },
        {
          workspaceKey: `${SERVER_ID}:workspace-main`,
          serverId: SERVER_ID,
          workspaceId: "workspace-main",
          projectRootPath: "/repo/project-a",
          workspaceDirectory: "/repo/project-a/workspace-main",
          projectKind: "git",
          workspaceKind: "local_checkout",
          name: "main",
          statusBucket: "done",
          diffStat: null,
          scripts: [],
          hasRunningScripts: false,
        },
      ],
    };

    const result = getSidebarRenderableProjects([project]);

    expect(result).toHaveLength(1);
    expect(result[0]?.workspaces).toHaveLength(2);
    expect(result[0]?.workspaces.map((workspace) => workspace.workspaceId)).toEqual([
      "__creating_worktree__:demo",
      "workspace-main",
    ]);
  });

  it("hides creating placeholders when real workspace directory already points to the same slug", () => {
    const project: SidebarProjectEntry = {
      projectKey: "project-a",
      projectName: "Project A",
      projectKind: "git",
      iconWorkingDir: "/repo/project-a",
      workspaces: [
        {
          workspaceKey: `${SERVER_ID}:__creating_worktree__:cowardly-lynx`,
          serverId: SERVER_ID,
          workspaceId: "__creating_worktree__:cowardly-lynx",
          projectRootPath: "/repo/project-a",
          workspaceDirectory: "/repo/project-a/.paseo/pending/cowardly-lynx",
          projectKind: "git",
          workspaceKind: "worktree",
          name: "pending-cowardly-lynx",
          statusBucket: "running",
          diffStat: null,
          scripts: [],
          hasRunningScripts: false,
        },
        {
          workspaceKey: `${SERVER_ID}:workspace-real`,
          serverId: SERVER_ID,
          workspaceId: "workspace-real",
          projectRootPath: "/repo/project-a",
          workspaceDirectory: "/repo/project-a/.paseo/worktrees/cowardly-lynx",
          projectKind: "git",
          workspaceKind: "worktree",
          name: "cowardly-lynx",
          statusBucket: "running",
          diffStat: null,
          scripts: [],
          hasRunningScripts: false,
        },
      ],
    };

    const result = getSidebarRenderableProjects([project]);

    expect(result).toHaveLength(1);
    expect(result[0]?.workspaces).toHaveLength(1);
    expect(result[0]?.workspaces[0]?.workspaceId).toBe("workspace-real");
  });
});

describe("useSidebarWorkspacesList", () => {
  afterEach(() => {
    cleanupSidebarState();
  });

  it("keeps real workspace names in live sidebar data so placeholders can be filtered", async () => {
    initializeSidebarState([
      {
        ...workspace({
          id: "__creating_worktree__:eager-squid",
          projectId: "project-a",
          projectDisplayName: "Project A",
          name: "eager-squid",
          status: "running",
        }),
        workspaceDirectory: "/repo/project-a/.paseo/pending/eager-squid",
      },
      {
        ...workspace({
          id: "workspace-real",
          projectId: "project-a",
          projectDisplayName: "Project A",
          name: "eager-squid",
        }),
        workspaceDirectory: "/repo/project-a/.paseo/worktrees/eager-squid",
      },
    ]);

    const projects = await captureSidebarProjects();
    const renderableProjects = getSidebarRenderableProjects(projects);

    expect(renderableProjects).toHaveLength(1);
    expect(renderableProjects[0]?.workspaces.map((workspace) => workspace.workspaceId)).toEqual([
      "workspace-real",
    ]);
  });
});

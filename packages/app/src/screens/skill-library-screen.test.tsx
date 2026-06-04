import React from "react";
import { act } from "react";
import { JSDOM } from "jsdom";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ContextHubManagedSkillEntry,
  ContextHubMarketplaceSkillEntry,
} from "@server/shared/messages";
import { SkillLibraryScreen } from "./skill-library-screen";

const {
  theme,
  defaultWorkspace,
  mockClient,
  mockSessionState,
  getMockWorkspaceList,
  toastSuccessMock,
  toastErrorMock,
} = vi.hoisted(() => {
  const theme = {
    spacing: { 1: 4, 2: 8, 3: 12, 4: 16, 5: 20, 8: 32, 10: 40, 16: 64 },
    borderRadius: { sm: 4, md: 6 },
    fontSize: { xs: 11, sm: 13, base: 15, lg: 18, "4xl": 32 },
    fontWeight: { normal: "400", semibold: "600" },
    colors: {
      surface0: "#000",
      surface1: "#111",
      surface2: "#222",
      foreground: "#fff",
      foregroundMuted: "#aaa",
      border: "#555",
      borderAccent: "#444",
      accentBright: "#0a84ff",
      success: "#30d158",
      palette: { black: "#000" },
    },
  };

  const marketplaceSkill: ContextHubMarketplaceSkillEntry = {
    id: "skillsmp:openai-codex-codex-skills-code-review-skill-md",
    name: "Code Review",
    description: "Review code changes with repo context.",
    version: null,
    trustLevel: "verified",
    vettingStatus: "trusted-source",
    capabilities: ["review"],
    permissions: {
      network: false,
      filesystem: false,
      subprocess: false,
      envVars: [],
    },
    platformCompatibility: ["Claude", "Codex"],
    downloadCount: 88313,
    downloads7d: 0,
    daysSinceUpdate: 3,
    installed: false,
  };

  const localSkill: ContextHubManagedSkillEntry = {
    id: "bundled:paseo-chat",
    name: "paseo-chat",
    description: "Use chat rooms through the Paseo CLI.",
    source: "bundled",
    scope: "global",
    workspaceId: null,
    path: "/repo/skills/paseo-chat/SKILL.md",
    readOnly: true,
    content: null,
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
  };

  const mockClient = {
    isConnected: true,
    skillsList: vi.fn(async () => ({ skills: [localSkill], error: null })),
    skillsMarketplaceList: vi.fn(async () => ({ skills: [marketplaceSkill], error: null })),
    skillsMarketplaceInstall: vi.fn(async () => ({
      skill: null,
      installed: true,
      conflict: false,
      error: null,
    })),
  };

  const defaultWorkspace = {
    id: "workspace-1",
    projectId: "project-1",
    projectDisplayName: "Paseo",
    projectRootPath: "/repo",
    workspaceDirectory: "/repo",
    projectKind: "repository",
    workspaceKind: "main",
    name: "main",
    status: "active",
    diffStat: null,
    scripts: [],
  };
  type MockWorkspace = typeof defaultWorkspace;

  const mockSessionState: {
    sessions: Record<string, { workspaces: Map<string, MockWorkspace> }>;
  } = {
    sessions: {
      server: {
        workspaces: new Map([["workspace-1", defaultWorkspace]]),
      },
    },
  };

  const workspaceListCache = new Map<
    string,
    {
      workspaces: Map<string, MockWorkspace> | undefined;
      list: MockWorkspace[];
    }
  >();
  const getMockWorkspaceList = (serverId: string) => {
    const workspaces = mockSessionState.sessions[serverId]?.workspaces;
    const cached = workspaceListCache.get(serverId);
    if (cached?.workspaces === workspaces) {
      return cached.list;
    }
    const list = Array.from(workspaces?.values() ?? []);
    workspaceListCache.set(serverId, { workspaces, list });
    return list;
  };

  return {
    theme,
    defaultWorkspace,
    mockClient,
    mockSessionState,
    getMockWorkspaceList,
    toastSuccessMock: vi.fn(),
    toastErrorMock: vi.fn(),
  };
});

vi.mock("react-native-unistyles", () => ({
  StyleSheet: {
    create: (factory: unknown) => (typeof factory === "function" ? factory(theme) : factory),
  },
  useUnistyles: () => ({ theme }),
}));

vi.mock("@/constants/platform", () => ({
  isWeb: true,
}));

vi.mock("lucide-react-native", () => {
  const createIcon = (name: string) => (props: Record<string, unknown>) =>
    React.createElement("span", { ...props, "data-icon": name });
  return {
    Box: createIcon("Box"),
    Check: createIcon("Check"),
    ChevronDown: createIcon("ChevronDown"),
    Download: createIcon("Download"),
    RefreshCcw: createIcon("RefreshCcw"),
    Search: createIcon("Search"),
    ShieldCheck: createIcon("ShieldCheck"),
  };
});

vi.mock("@/components/headers/menu-header", () => ({
  MenuHeader: ({ rightContent }: { rightContent?: React.ReactNode }) => (
    <header>{rightContent}</header>
  ),
}));

vi.mock("@/contexts/toast-context", () => ({
  useToast: () => ({
    show: toastSuccessMock,
    error: toastErrorMock,
  }),
}));

vi.mock("@/hooks/use-sub2api-locale", () => ({
  useSub2APILocale: () => "en",
}));

vi.mock("@/runtime/host-runtime", () => ({
  useHostRuntimeClient: () => mockClient,
}));

vi.mock("@/stores/navigation-active-workspace-store", () => ({
  useNavigationActiveWorkspaceSelection: () => null,
}));

vi.mock("@/stores/session-store-hooks", () => ({
  useWorkspaceList: (serverId: string) => getMockWorkspaceList(serverId),
}));

vi.mock("@/utils/error-messages", () => ({
  toErrorMessage: (error: unknown) => (error instanceof Error ? error.message : String(error)),
}));

vi.mock("react-native", async () => {
  const actual = await vi.importActual<Record<string, unknown>>("react-native");
  return actual;
});

let root: Root | null = null;
let container: HTMLElement | null = null;

beforeEach(() => {
  const dom = new JSDOM("<!doctype html><html><body></body></html>");
  vi.stubGlobal("React", React);
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("window", dom.window);
  vi.stubGlobal("document", dom.window.document);
  vi.stubGlobal("HTMLElement", dom.window.HTMLElement);
  vi.stubGlobal("Node", dom.window.Node);
  vi.stubGlobal("navigator", dom.window.navigator);

  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  mockSessionState.sessions.server.workspaces = new Map([["workspace-1", defaultWorkspace]]);
  mockClient.skillsList.mockClear();
  mockClient.skillsMarketplaceList.mockClear();
  mockClient.skillsMarketplaceInstall.mockClear();
  toastSuccessMock.mockClear();
  toastErrorMock.mockClear();
});

afterEach(() => {
  if (root) {
    act(() => {
      root?.unmount();
    });
  }
  root = null;
  container = null;
  vi.unstubAllGlobals();
});

function renderScreen() {
  act(() => {
    root?.render(<SkillLibraryScreen serverId="server" />);
  });
}

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

function click(element: Element) {
  act(() => {
    element.dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
  });
}

async function findByTestId(testID: string): Promise<HTMLElement> {
  for (let attempt = 0; attempt < 10; attempt += 1) {
    await flush();
    const element = document.querySelector(`[data-testid="${testID}"]`) as HTMLElement | null;
    if (element) return element;
  }
  throw new Error(`Missing element with testID ${testID}`);
}

describe("SkillLibraryScreen marketplace", () => {
  it("renders marketplace search results with trust, permissions, and install state", async () => {
    renderScreen();

    const row = await findByTestId(
      "skill-marketplace-row-skillsmp:openai-codex-codex-skills-code-review-skill-md",
    );

    expect(row.textContent).toContain("Code Review");
    expect(row.textContent).toContain("Review code changes with repo context.");
    expect(row.textContent).toContain("verified");
    expect(row.textContent).toContain("No elevated permissions");
    expect(row.textContent).toContain("88313 downloads");
    expect(row.textContent).toContain("0/7d");
    const section = await findByTestId("skill-marketplace-letter-section-C");
    const rail = await findByTestId("skill-marketplace-alphabet-rail");
    const letterC = await findByTestId("skill-marketplace-letter-C");
    expect(section.textContent).toContain("C");
    expect(rail.textContent).toContain("ABCDEFGHIJKLMNOPQRSTUVWXYZ");
    expect(letterC.getAttribute("aria-disabled")).not.toBe("true");
    expect(mockClient.skillsList).toHaveBeenCalledWith({
      workspaceId: "workspace-1",
      cwd: "/repo",
    });
    expect(mockClient.skillsMarketplaceList).toHaveBeenCalledWith({
      query: undefined,
      limit: 50,
      minTrust: "verified",
      workspaceId: "workspace-1",
      cwd: "/repo",
    });
  });

  it("renders local and bundled skills separately from marketplace results", async () => {
    renderScreen();

    const row = await findByTestId("local-skill-row-paseo-chat");

    expect(row.textContent).toContain("paseo-chat");
    expect(row.textContent).toContain("Use chat rooms through the Paseo CLI.");
    expect(row.textContent).toContain("Built-in");
    expect(row.textContent).toContain("Global");
    expect(row.textContent).toContain("Built-in");
  });

  it("disables install when no workspace is selected", async () => {
    mockSessionState.sessions.server.workspaces = new Map();
    renderScreen();

    const installButton = await findByTestId(
      "skill-marketplace-install-skillsmp:openai-codex-codex-skills-code-review-skill-md",
    );

    expect(installButton.getAttribute("aria-disabled")).toBe("true");
    expect(installButton.textContent).toContain("Install");
  });

  it("installs into the selected workspace and refreshes the marketplace", async () => {
    renderScreen();
    const installButton = await findByTestId(
      "skill-marketplace-install-skillsmp:openai-codex-codex-skills-code-review-skill-md",
    );

    click(installButton);
    await flush();

    expect(mockClient.skillsMarketplaceInstall).toHaveBeenCalledWith({
      workspaceId: "workspace-1",
      cwd: "/repo",
      skillId: "skillsmp:openai-codex-codex-skills-code-review-skill-md",
      name: "Code Review",
      version: undefined,
      overwrite: false,
    });
    expect(toastSuccessMock).toHaveBeenCalledWith(
      "Code Review installed. Reload running agents to pick it up.",
      { variant: "success" },
    );
    expect(mockClient.skillsList).toHaveBeenCalledTimes(2);
    expect(mockClient.skillsMarketplaceList).toHaveBeenCalledTimes(2);
  });
});

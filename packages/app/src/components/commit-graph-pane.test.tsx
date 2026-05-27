/**
 * @vitest-environment jsdom
 */

import { JSDOM } from "jsdom";
import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CommitGraphPane } from "./commit-graph-pane";

const { graphState, clipboardMock, toast } = vi.hoisted(() => ({
  graphState: {
    graph: null as any,
    isLoading: false,
    isFetching: false,
    isError: false,
    error: null as Error | null,
    refetch: vi.fn(),
  },
  clipboardMock: vi.fn(async () => true),
  toast: {
    copied: vi.fn(),
    error: vi.fn(),
  },
}));

const { theme } = vi.hoisted(() => ({
  theme: {
    colorScheme: "dark",
    spacing: { 1: 4, 2: 8, 3: 12, 4: 16, 6: 24 },
    borderWidth: { 1: 1 },
    borderRadius: { base: 4, md: 6, lg: 8 },
    fontSize: { xs: 11, sm: 13, base: 15 },
    fontWeight: { normal: "400", medium: "600" },
    colors: {
      accent: "#20744A",
      background: "#20242a",
      border: "#3b4048",
      borderAccent: "#4a5059",
      foreground: "#d4d7de",
      foregroundMuted: "#9aa1ad",
      palette: { white: "#fff" },
      popoverForeground: "#fff",
      surface0: "#20242a",
      surface1: "#252a31",
      surface2: "#303640",
    },
  },
}));

function makeCommit(overrides: Partial<any>) {
  return {
    hash: "aaaaaaa",
    fullHash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    message: "Initial commit",
    author: "Ada",
    authorEmail: "ada@example.com",
    date: 1_779_840_000,
    parents: [],
    branchTips: [],
    tags: [],
    isMerge: false,
    ...overrides,
  };
}

function makeGraph() {
  const head = makeCommit({
    hash: "c333333",
    fullHash: "c333333333333333333333333333333333333333",
    message: "fix: update API localization",
    parents: ["c222222222222222222222222222222222222222"],
    branchTips: ["main", "origin/HEAD"],
    tags: [],
  });
  const selected = makeCommit({
    hash: "c222222",
    fullHash: "c222222222222222222222222222222222222222",
    message: "feat: add connect API link",
    parents: ["c111111111111111111111111111111111111111"],
    branchTips: ["feature"],
    tags: ["v0.1.27"],
  });
  const root = makeCommit({
    hash: "c111111",
    fullHash: "c111111111111111111111111111111111111111",
    message: "chore: update sponsors",
    author: "Grace",
  });
  return {
    commits: [head, selected, root],
    branches: [
      { name: "main", isRemote: false, isCurrent: true, tipCommit: head.fullHash },
      { name: "feature", isRemote: false, isCurrent: false, tipCommit: selected.fullHash },
    ],
    headCommit: head.fullHash,
    rootCommits: [root.fullHash],
  };
}

vi.mock("expo-clipboard", () => ({
  setStringAsync: clipboardMock,
}));

vi.mock("react-native-unistyles", () => ({
  StyleSheet: {
    create: (factory: unknown) => (typeof factory === "function" ? factory(theme) : factory),
  },
  useUnistyles: () => ({ theme }),
}));

vi.mock("react-native", () => {
  const normalizeStyle = (style: unknown) => {
    if (Array.isArray(style)) {
      return Object.assign(
        {},
        ...style.filter(
          (item) => typeof item === "object" && item !== null && !Array.isArray(item),
        ),
      );
    }
    return typeof style === "object" && style !== null ? style : undefined;
  };

  const mapProps = (props: Record<string, unknown>) => {
    const {
      accessibilityLabel,
      accessibilityRole,
      children,
      contentContainerStyle,
      horizontal,
      onChangeText,
      onLayout,
      onPress,
      placeholder,
      placeholderTextColor,
      showsHorizontalScrollIndicator,
      showsVerticalScrollIndicator,
      style,
      testID,
      value,
      numberOfLines,
      selectable,
      pointerEvents,
      ...rest
    } = props;
    return {
      ...rest,
      ...(normalizeStyle(style) ? { style: normalizeStyle(style) } : {}),
      ...(typeof accessibilityLabel === "string" ? { "aria-label": accessibilityLabel } : {}),
      ...(accessibilityRole === "button" ? { role: "button" } : {}),
      ...(typeof testID === "string" ? { "data-testid": testID } : {}),
      children,
      placeholder,
      value,
      onChange:
        typeof onChangeText === "function"
          ? (event: React.ChangeEvent<HTMLInputElement>) => onChangeText(event.target.value)
          : undefined,
      onClick:
        typeof onPress === "function"
          ? (event: React.MouseEvent) =>
              onPress({
                nativeEvent: { metaKey: event.metaKey, ctrlKey: event.ctrlKey },
              })
          : undefined,
      ref: undefined,
    };
  };

  return {
    ActivityIndicator: (props: Record<string, unknown>) =>
      React.createElement("span", mapProps(props)),
    Pressable: (props: Record<string, unknown>) => {
      const children =
        typeof props.children === "function"
          ? props.children({ hovered: false, pressed: false, open: false })
          : props.children;
      return React.createElement("button", mapProps({ ...props, children }));
    },
    ScrollView: React.forwardRef<HTMLDivElement, Record<string, unknown>>((props, ref) =>
      React.createElement("div", { ...mapProps(props), ref }),
    ),
    Text: (props: Record<string, unknown>) => React.createElement("span", mapProps(props)),
    TextInput: (props: Record<string, unknown>) => React.createElement("input", mapProps(props)),
    View: (props: Record<string, unknown>) => {
      const mappedProps = mapProps(props);
      React.useEffect(() => {
        if (typeof props.onLayout === "function") {
          props.onLayout({ nativeEvent: { layout: { width: 720, height: 480 } } });
        }
      }, [props.onLayout]);
      return React.createElement("div", mappedProps);
    },
  };
});

vi.mock("lucide-react-native", () => {
  const createIcon = (name: string) => (props: Record<string, unknown>) =>
    React.createElement("span", { ...props, "data-icon": name });
  return {
    Copy: createIcon("Copy"),
    Crosshair: createIcon("Crosshair"),
    Eye: createIcon("Eye"),
    EyeOff: createIcon("EyeOff"),
    GitBranch: createIcon("GitBranch"),
    GitCommitHorizontal: createIcon("GitCommitHorizontal"),
    GitCompare: createIcon("GitCompare"),
    RefreshCcw: createIcon("RefreshCcw"),
    Search: createIcon("Search"),
    Tag: createIcon("Tag"),
    X: createIcon("X"),
  };
});

vi.mock("@/hooks/use-commit-graph-query", () => ({
  useCommitGraphQuery: () => graphState,
}));

vi.mock("@/contexts/toast-context", () => ({
  useToast: () => toast,
}));

vi.mock("@/components/ui/context-menu", () => ({
  ContextMenu: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  ContextMenuContent: ({ children, testID }: { children: React.ReactNode; testID?: string }) => (
    <div data-testid={testID}>{children}</div>
  ),
  ContextMenuItem: ({ children, disabled, onSelect, testID }: any) => (
    <button type="button" data-testid={testID} disabled={disabled} onClick={onSelect}>
      {children}
    </button>
  ),
  ContextMenuSeparator: () => <div role="separator" />,
  ContextMenuTrigger: ({ children, onPress, testID, accessibilityLabel, style }: any) => {
    const resolvedStyle =
      typeof style === "function" ? style({ hovered: false, pressed: false, open: false }) : style;
    const normalizedStyle = Array.isArray(resolvedStyle)
      ? Object.assign({}, ...resolvedStyle.filter((item) => item && typeof item === "object"))
      : resolvedStyle;
    return (
      <button
        type="button"
        data-testid={testID}
        aria-label={accessibilityLabel}
        style={normalizedStyle}
        onClick={(event) =>
          onPress?.({ nativeEvent: { metaKey: event.metaKey, ctrlKey: event.ctrlKey } })
        }
      >
        {children}
      </button>
    );
  },
}));

vi.mock("@/components/ui/tooltip", () => ({
  Tooltip: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  TooltipContent: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  TooltipTrigger: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

vi.mock("@/constants/theme", () => ({
  Fonts: { mono: "monospace" },
}));

vi.mock("@/constants/platform", () => ({
  isWeb: true,
}));

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

  graphState.graph = makeGraph();
  graphState.isLoading = false;
  graphState.isFetching = false;
  graphState.isError = false;
  graphState.error = null;
  graphState.refetch.mockReset();
  clipboardMock.mockClear();
  toast.copied.mockClear();
  toast.error.mockClear();

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
  container = null;
  vi.unstubAllGlobals();
});

function renderPane() {
  act(() => {
    root?.render(<CommitGraphPane serverId="server-1" cwd="/repo" />);
  });
}

function click(testID: string, init?: MouseEventInit) {
  const element = document.querySelector(`[data-testid="${testID}"]`) as HTMLElement;
  act(() => {
    element.dispatchEvent(new window.MouseEvent("click", { bubbles: true, ...init }));
  });
}

async function flushPromises() {
  await act(async () => {
    await Promise.resolve();
  });
}

describe("CommitGraphPane", () => {
  it("renders graph table rows with refs and columns", () => {
    renderPane();

    expect(document.body.textContent).toContain("Description");
    expect(document.body.textContent).toContain("Date");
    expect(document.body.textContent).toContain("Author");
    expect(document.body.textContent).toContain("Commit");
    expect(document.body.textContent).toContain("fix: update API localization");
    expect(document.body.textContent).toContain("main");
    expect(document.body.textContent).toContain("v0.1.27");
  });

  it("uses a compact adaptive table without a horizontal scroller", () => {
    renderPane();

    expect(document.querySelector('[data-testid="commit-graph-horizontal-scroll"]')).toBeNull();

    const graphPane = document.querySelector('[data-testid="commit-graph-pane"]') as HTMLElement;
    expect(graphPane.textContent).toContain("Git Graph");

    const row = document.querySelector('[data-testid="commit-graph-row-c222222"]') as HTMLElement;
    expect(row.style.width).toBe("100%");
    expect(row.style.height).toBe("28px");
  });

  it("renders a detail card under the selected commit row", () => {
    renderPane();

    click("commit-graph-row-c222222");

    expect(document.querySelector('[data-testid="commit-graph-detail"]')).not.toBeNull();
    expect(document.body.textContent).toContain("Commit: c222222222222222222222222222222222222222");
    expect(document.body.textContent).toContain("Changed files");
  });

  it("copies a commit hash from the context menu action", async () => {
    renderPane();

    click("commit-graph-copy-hash-c222222");
    await flushPromises();

    expect(clipboardMock).toHaveBeenCalledWith("c222222222222222222222222222222222222222");
    expect(toast.copied).toHaveBeenCalledWith("commit hash");
  });

  it("shows loading and empty states", () => {
    graphState.isLoading = true;
    renderPane();
    expect(document.querySelector('[data-testid="commit-graph-loading"]')).not.toBeNull();

    graphState.isLoading = false;
    graphState.graph = { commits: [], branches: [], headCommit: null, rootCommits: [] };
    act(() => {
      root?.render(<CommitGraphPane serverId="server-1" cwd="/repo-empty" />);
    });
    expect(document.querySelector('[data-testid="commit-graph-empty"]')).not.toBeNull();
  });
});

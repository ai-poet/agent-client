/**
 * @vitest-environment jsdom
 */
import React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { JSDOM } from "jsdom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { GitDiffPane } from "./git-diff-pane";

const { commitMock } = vi.hoisted(() => ({
  commitMock: vi.fn(),
}));

const { diffFiles } = vi.hoisted(() => ({
  diffFiles: [
    {
      path: "src/app.ts",
      isNew: false,
      isDeleted: false,
      additions: 2,
      deletions: 1,
      hunks: [],
    },
    {
      path: "README.md",
      isNew: false,
      isDeleted: false,
      additions: 1,
      deletions: 0,
      hunks: [],
    },
  ],
}));

const { theme } = vi.hoisted(() => ({
  theme: {
    spacing: { 1: 4, 2: 8, 3: 12, 4: 16, 6: 24, 8: 32, 16: 64 },
    borderWidth: { 1: 1 },
    borderRadius: { base: 4, md: 6, lg: 8 },
    fontSize: { xs: 11, sm: 13, base: 15, lg: 18 },
    lineHeight: { diff: 18 },
    fontWeight: { normal: "400", medium: "600" },
    opacity: { 50: 0.5 },
    colors: {
      accent: "#2563eb",
      border: "#ddd",
      borderAccent: "#ddd",
      destructive: "#dc2626",
      diffAddition: "#15803d",
      diffDeletion: "#b91c1c",
      foreground: "#111",
      foregroundMuted: "#666",
      palette: { white: "#fff" },
      surface0: "#fff",
      surface1: "#fafafa",
      surface2: "#f4f4f5",
      surface3: "#e4e4e7",
      surfaceDiffEmpty: "#f6f6f6",
    },
  },
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
      accessibilityState,
      children,
      disabled,
      editable,
      onChangeText,
      onPress,
      onPressIn,
      onPressOut,
      placeholder,
      placeholderTextColor,
      style,
      testID,
      value,
      onLayout,
      numberOfLines,
      hitSlop,
      cancelable,
      autoCapitalize,
      autoCorrect,
      returnKeyType,
      onSubmitEditing,
      onHoverIn,
      onHoverOut,
      ...rest
    } = props;
    return {
      ...rest,
      ...(normalizeStyle(style) ? { style: normalizeStyle(style) } : {}),
      ...(typeof accessibilityLabel === "string" ? { "aria-label": accessibilityLabel } : {}),
      ...(accessibilityRole === "checkbox" ? { role: "checkbox" } : {}),
      ...(typeof accessibilityState === "object" && accessibilityState
        ? { "aria-checked": String((accessibilityState as { checked?: boolean }).checked) }
        : {}),
      ...(typeof testID === "string" ? { "data-testid": testID } : {}),
      children,
      disabled: disabled || editable === false || undefined,
      placeholder,
      value,
      onChange:
        typeof onChangeText === "function"
          ? (event: React.ChangeEvent<HTMLInputElement>) => onChangeText(event.target.value)
          : undefined,
      onClick:
        typeof onPress === "function"
          ? (event: React.MouseEvent) => onPress({ stopPropagation: () => event.stopPropagation() })
          : undefined,
    };
  };

  return {
    ActivityIndicator: (props: Record<string, unknown>) =>
      React.createElement("span", mapProps(props)),
    FlatList: ({ data, renderItem, keyExtractor, testID }: any) =>
      React.createElement(
        "div",
        { "data-testid": testID },
        data.map((item: unknown, index: number) =>
          React.createElement(
            React.Fragment,
            { key: keyExtractor?.(item, index) ?? index },
            renderItem({ item, index }),
          ),
        ),
      ),
    Pressable: (props: Record<string, unknown>) => {
      const children =
        typeof props.children === "function"
          ? props.children({ hovered: false, pressed: false, open: false })
          : props.children;
      const mapped = mapProps({ ...props, children });
      return React.createElement(props.accessibilityRole === "checkbox" ? "button" : "div", mapped);
    },
    Text: (props: Record<string, unknown>) => React.createElement("span", mapProps(props)),
    TextInput: React.forwardRef<HTMLInputElement, Record<string, unknown>>((props, ref) =>
      React.createElement("input", { ...mapProps(props), ref }),
    ),
    View: (props: Record<string, unknown>) => React.createElement("div", mapProps(props)),
  };
});

vi.mock("lucide-react-native", () => {
  const createIcon = (name: string) => (props: Record<string, unknown>) =>
    React.createElement("span", { ...props, "data-icon": name });
  return {
    AlignJustify: createIcon("AlignJustify"),
    Archive: createIcon("Archive"),
    CheckSquare: createIcon("CheckSquare"),
    ChevronDown: createIcon("ChevronDown"),
    Columns2: createIcon("Columns2"),
    Download: createIcon("Download"),
    GitBranch: createIcon("GitBranch"),
    GitCommitHorizontal: createIcon("GitCommitHorizontal"),
    GitMerge: createIcon("GitMerge"),
    ListChevronsDownUp: createIcon("ListChevronsDownUp"),
    ListChevronsUpDown: createIcon("ListChevronsUpDown"),
    Pilcrow: createIcon("Pilcrow"),
    RefreshCcw: createIcon("RefreshCcw"),
    Square: createIcon("Square"),
    Upload: createIcon("Upload"),
    WrapText: createIcon("WrapText"),
  };
});

vi.mock("@/constants/layout", () => ({
  WORKSPACE_SECONDARY_HEADER_HEIGHT: 44,
  useIsCompactFormFactor: () => false,
}));

vi.mock("@/constants/platform", () => ({
  isNative: false,
  isWeb: true,
}));

vi.mock("@/constants/theme", () => ({
  Fonts: { mono: "monospace" },
}));

vi.mock("@/hooks/use-app-locale", () => ({
  useAppLocale: () => "en",
}));

vi.mock("@/hooks/use-changes-preferences", () => ({
  useChangesPreferences: () => ({
    preferences: { hideWhitespace: false, layout: "unified", wrapLines: false },
    updatePreferences: vi.fn(),
  }),
}));

vi.mock("@/hooks/use-checkout-status-query", () => ({
  useCheckoutStatusQuery: () => ({
    status: {
      aheadBehind: null,
      aheadOfOrigin: 0,
      baseRef: "main",
      behindOfOrigin: 0,
      currentBranch: "feature",
      cwd: "/repo",
      error: null,
      hasRemote: false,
      isDirty: true,
      isGit: true,
      isPaseoOwnedWorktree: false,
      remoteUrl: null,
      repoRoot: "/repo",
    },
    isLoading: false,
    isError: false,
    error: null,
  }),
}));

vi.mock("@/hooks/use-checkout-diff-query", () => ({
  useCheckoutDiffQuery: () => ({
    files: diffFiles,
    payloadError: null,
    isLoading: false,
  }),
}));

vi.mock("@/hooks/use-checkout-pr-status-query", () => ({
  useCheckoutPrStatusQuery: () => ({
    status: null,
    githubFeaturesEnabled: true,
    payloadError: null,
  }),
}));

vi.mock("@/stores/checkout-git-actions-store", () => ({
  useCheckoutGitActionsStore: (selector: (state: any) => unknown) =>
    selector({
      getStatus: () => "idle",
      commit: commitMock,
      pull: vi.fn(),
      push: vi.fn(),
      createPr: vi.fn(),
      mergeBranch: vi.fn(),
      mergeFromBase: vi.fn(),
      archiveWorktree: vi.fn(),
    }),
}));

vi.mock("@/stores/panel-store", () => ({
  usePanelStore: (selector: (state: any) => unknown) =>
    selector({
      commitFocusRequestByCheckout: {},
      diffExpandedPathsByWorkspace: {},
      setDiffExpandedPathsForWorkspace: vi.fn(),
    }),
}));

vi.mock("@/contexts/toast-context", () => ({
  useToast: () => ({ error: vi.fn(), show: vi.fn() }),
}));

vi.mock("@/contexts/git-commit-dialog-context", () => ({
  useGitCommitDialog: () => ({ openCommitDialog: vi.fn() }),
}));

vi.mock("@/components/diff-scroll", () => ({
  DiffScroll: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

vi.mock("@/components/use-web-scrollbar", () => ({
  useWebScrollViewScrollbar: () => ({
    onContentSizeChange: vi.fn(),
    onLayout: vi.fn(),
    onScroll: vi.fn(),
    overlay: null,
  }),
}));

vi.mock("@/components/ui/dropdown-menu", () => ({
  DropdownMenu: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  DropdownMenuContent: ({ children, testID }: { children: React.ReactNode; testID?: string }) => (
    <div data-testid={testID}>{children}</div>
  ),
  DropdownMenuItem: ({ children, onSelect, testID }: any) => (
    <button type="button" data-testid={testID} onClick={onSelect}>
      {children}
    </button>
  ),
  DropdownMenuSeparator: () => <div role="separator" />,
  DropdownMenuTrigger: ({ children, testID, accessibilityLabel }: any) => (
    <button type="button" data-testid={testID} aria-label={accessibilityLabel}>
      {typeof children === "function"
        ? children({ hovered: false, pressed: false, open: false })
        : children}
    </button>
  ),
}));

vi.mock("@/components/ui/tooltip", () => ({
  Tooltip: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  TooltipContent: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  TooltipTrigger: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

vi.mock("@/components/ui/shortcut", () => ({
  Shortcut: () => null,
}));

vi.mock("@/hooks/use-shortcut-keys", () => ({
  useShortcutKeys: () => null,
}));

vi.mock("@/components/icons/github-icon", () => ({
  GitHubIcon: () => React.createElement("span", { "data-icon": "GitHub" }),
}));

vi.mock("expo-router", () => ({
  useRouter: () => ({ replace: vi.fn() }),
}));

vi.mock("@/utils/new-agent-routing", () => ({
  buildNewAgentRoute: () => "/new-agent",
  resolveNewAgentWorkingDir: (cwd: string) => cwd,
}));

vi.mock("@/utils/open-external-url", () => ({
  openExternalUrl: vi.fn(),
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
  vi.stubGlobal("navigator", dom.window.navigator);

  commitMock.mockResolvedValue(undefined);
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
  commitMock.mockReset();
  vi.unstubAllGlobals();
});

function renderPane() {
  act(() => {
    root?.render(<GitDiffPane serverId="server-1" cwd="/repo" hideHeaderRow />);
  });
}

function changeMessage(value: string) {
  const input = document.querySelector(
    '[data-testid="changes-inline-commit-message"]',
  ) as HTMLInputElement;
  act(() => {
    Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value")?.set?.call(
      input,
      value,
    );
    input.dispatchEvent(new window.Event("input", { bubbles: true }));
  });
}

function click(testID: string) {
  const element = document.querySelector(`[data-testid="${testID}"]`) as HTMLElement;
  act(() => {
    element.click();
  });
}

describe("GitDiffPane inline commit", () => {
  it("renders an inline commit form with all files selected by default", () => {
    renderPane();

    expect(document.querySelector('[data-testid="changes-inline-commit"]')).not.toBeNull();
    expect(document.querySelector('[data-testid="changes-inline-commit-message"]')).not.toBeNull();
    expect(document.body.textContent).toContain("2/2 files selected");
    expect(
      document.querySelector('[data-testid="diff-file-0-select"]')?.getAttribute("aria-checked"),
    ).toBe("true");
    expect(
      document.querySelector('[data-testid="diff-file-1-select"]')?.getAttribute("aria-checked"),
    ).toBe("true");
  });

  it("commits only the selected files", async () => {
    renderPane();

    changeMessage("Ship selected file");
    click("diff-file-1-select");
    click("changes-inline-commit-submit");

    await vi.waitFor(() => {
      expect(commitMock).toHaveBeenCalledWith({
        serverId: "server-1",
        cwd: "/repo",
        message: "Ship selected file",
        addAll: false,
        paths: ["src/app.ts"],
      });
    });
  });

  it("does not commit an empty message", () => {
    renderPane();

    click("changes-inline-commit-submit");

    expect(commitMock).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("Enter a commit message");
  });

  it("does not commit when no files are selected", () => {
    renderPane();

    changeMessage("Nothing selected");
    click("changes-inline-toggle-all-files");
    click("changes-inline-commit-submit");

    expect(commitMock).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("Select at least one file");
  });
});

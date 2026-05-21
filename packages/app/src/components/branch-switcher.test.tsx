/**
 * @vitest-environment jsdom
 */
import React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { JSDOM } from "jsdom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BranchSwitcher } from "./branch-switcher";

const { theme, handleBranchSelectMock, setIsOpenMock } = vi.hoisted(() => ({
  theme: {
    spacing: { 1: 4, 2: 8, 3: 12 },
    iconSize: { sm: 14, md: 18 },
    borderRadius: { sm: 4, md: 6, lg: 8, "2xl": 16 },
    fontSize: { sm: 13, lg: 18 },
    fontWeight: { medium: "500" },
    shadow: { md: {} },
    colors: {
      surface0: "#000",
      surface1: "#111",
      surface2: "#222",
      foreground: "#fff",
      foregroundMuted: "#aaa",
      border: "#333",
      palette: { zinc: { 600: "#52525b" } },
    },
  },
  handleBranchSelectMock: vi.fn(),
  setIsOpenMock: vi.fn(),
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

function mapProps(props: Record<string, unknown>): Record<string, unknown> {
  const {
    accessibilityLabel,
    accessibilityRole,
    children,
    collapsable,
    disabled,
    hitSlop,
    onPress,
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
  Pressable: (props: Record<string, unknown>) => {
    const children =
      typeof props.children === "function"
        ? props.children({ hovered: false, pressed: false, open: false })
        : props.children;
    return React.createElement("button", mapProps({ ...props, children }));
  },
  Text: (props: Record<string, unknown>) => React.createElement("span", mapProps(props)),
  View: React.forwardRef<HTMLDivElement, Record<string, unknown>>((props, ref) =>
    React.createElement("div", { ...mapProps(props), ref }),
  ),
}));

vi.mock("react-native-unistyles", () => ({
  StyleSheet: {
    create: (factory: unknown) => (typeof factory === "function" ? factory(theme) : factory),
  },
  useUnistyles: () => ({ theme }),
}));

vi.mock("@/constants/layout", () => ({
  useIsCompactFormFactor: () => false,
}));

vi.mock("@/constants/platform", () => ({
  isWeb: true,
  isNative: false,
}));

vi.mock("lucide-react-native", () => {
  const createIcon = (name: string) => (props: Record<string, unknown>) =>
    React.createElement("span", { ...props, "data-icon": name });
  return {
    ChevronDown: createIcon("ChevronDown"),
    GitBranch: createIcon("GitBranch"),
    GitBranchPlus: createIcon("GitBranchPlus"),
  };
});

vi.mock("@/runtime/host-runtime", () => ({
  useHostRuntimeClient: () => ({}),
  useHostRuntimeIsConnected: () => true,
}));

vi.mock("@/contexts/toast-context", () => ({
  useToast: () => ({ error: vi.fn(), show: vi.fn() }),
}));

vi.mock("@/hooks/use-app-locale", () => ({
  useAppLocale: () => "en",
}));

vi.mock("@/hooks/use-branch-switcher", () => ({
  useBranchSwitcher: () => ({
    branchOptions: [
      { id: "main", label: "main" },
      { id: "feature/base", label: "feature/base" },
    ],
    isOpen: true,
    setIsOpen: setIsOpenMock,
    handleBranchSelect: handleBranchSelectMock,
    invalidateStashAndCheckout: vi.fn(),
  }),
}));

vi.mock("@/components/ui/combobox", () => ({
  Combobox: ({
    options,
    value,
    renderOption,
    onSelect,
  }: {
    options: Array<{ id: string; label: string }>;
    value: string;
    renderOption: (input: {
      option: { id: string; label: string };
      selected: boolean;
      active: boolean;
      onPress: () => void;
    }) => React.ReactElement;
    onSelect: (id: string) => void;
  }) => (
    <div data-testid="branch-combobox">
      {options.map((option, index) =>
        renderOption({
          option,
          selected: option.id === value,
          active: index === 0,
          onPress: () => onSelect(option.id),
        }),
      )}
    </div>
  ),
  ComboboxItem: ({
    label,
    onPress,
    leadingSlot,
    trailingSlot,
  }: {
    label: string;
    onPress: () => void;
    leadingSlot?: React.ReactNode;
    trailingSlot?: React.ReactNode;
  }) => (
    <div role="button" data-testid={`branch-row-${label}`} onClick={onPress}>
      {leadingSlot}
      <span>{label}</span>
      {trailingSlot}
    </div>
  ),
}));

vi.mock("@/components/headers/screen-title", () => ({
  ScreenTitle: ({ children, testID }: { children: React.ReactNode; testID?: string }) => (
    <span data-testid={testID}>{children}</span>
  ),
}));

vi.mock("@/components/ui/tooltip", () => ({
  Tooltip: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  TooltipTrigger: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  TooltipContent: ({ children }: { children: React.ReactNode }) => <span>{children}</span>,
}));

let root: Root | null = null;
let container: HTMLElement | null = null;
let queryClient: QueryClient | null = null;

beforeEach(() => {
  const dom = new JSDOM("<!doctype html><html><body></body></html>");
  vi.stubGlobal("React", React);
  vi.stubGlobal("window", dom.window);
  vi.stubGlobal("document", dom.window.document);
  vi.stubGlobal("HTMLElement", dom.window.HTMLElement);
  vi.stubGlobal("Node", dom.window.Node);
  vi.stubGlobal("navigator", dom.window.navigator);
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);

  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  queryClient = new QueryClient();
  handleBranchSelectMock.mockReset();
  setIsOpenMock.mockReset();
});

afterEach(() => {
  if (root) {
    act(() => {
      root?.unmount();
    });
  }
  root = null;
  container = null;
  queryClient = null;
  vi.unstubAllGlobals();
});

function renderBranchSwitcher(onCreateWorktreeFromBranch = vi.fn()) {
  act(() => {
    root?.render(
      <QueryClientProvider client={queryClient!}>
        <BranchSwitcher
          currentBranchName="main"
          title="main"
          serverId="server"
          workspaceId="/repo"
          isGitCheckout
          onCreateWorktreeFromBranch={onCreateWorktreeFromBranch}
        />
      </QueryClientProvider>,
    );
  });
  return onCreateWorktreeFromBranch;
}

describe("BranchSwitcher", () => {
  it("switches branches when pressing a branch row", () => {
    renderBranchSwitcher();

    const branchRow = document.querySelector(
      '[data-testid="branch-row-feature/base"]',
    ) as HTMLElement | null;

    act(() => {
      branchRow?.click();
    });

    expect(handleBranchSelectMock).toHaveBeenCalledWith("feature/base");
  });

  it("creates a worktree from a branch without selecting the row", () => {
    const onCreateWorktreeFromBranch = renderBranchSwitcher();
    const createButton = document.querySelector(
      '[data-testid="branch-create-worktree-feature/base"]',
    ) as HTMLButtonElement | null;

    act(() => {
      createButton?.click();
    });

    expect(onCreateWorktreeFromBranch).toHaveBeenCalledWith("feature/base");
    expect(handleBranchSelectMock).not.toHaveBeenCalled();
  });
});

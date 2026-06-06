/**
 * @vitest-environment jsdom
 */
import React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RichMarkdown } from "./rich-markdown";

const { clipboardMock, theme } = vi.hoisted(() => ({
  clipboardMock: vi.fn(async () => {}),
  theme: {
    colorScheme: "dark",
    spacing: { 1: 4, 2: 8, 3: 12, 4: 16 },
    fontSize: { xs: 12, sm: 14, base: 16, lg: 18, xl: 20 },
    fontWeight: { medium: "500", semibold: "600", bold: "700" },
    borderRadius: { sm: 2, md: 6, lg: 8, full: 9999 },
    colors: {
      foreground: "#fafafa",
      foregroundMuted: "#a1a1aa",
      surface1: "#1e2120",
      surface2: "#272a29",
      border: "#252b2a",
      borderAccent: "#2f3534",
      primary: "#fafafa",
      accent: "#20744a",
      accentBright: "#7ccba0",
      accentForeground: "#ffffff",
      destructive: "#ef4444",
    },
  },
}));

vi.mock("@/constants/platform", () => ({
  getIsElectron: () => true,
  isWeb: true,
  isNative: false,
}));

vi.mock("react-native-unistyles", () => ({
  useUnistyles: () => ({ theme }),
}));

vi.mock("expo-clipboard", () => ({
  setStringAsync: clipboardMock,
}));

vi.mock("lucide-react-native", () => {
  const Icon = ({ size }: { size?: number }) => <span data-icon-size={size ?? 0} />;
  return {
    Check: Icon,
    Copy: Icon,
  };
});

describe("RichMarkdown desktop renderer", () => {
  let root: Root | null = null;
  let container: HTMLElement | null = null;
  let queryClient: QueryClient | null = null;

  beforeEach(() => {
    Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
      value: true,
      configurable: true,
    });
    clipboardMock.mockClear();
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
      },
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
    queryClient?.clear();
    queryClient = null;
    root = null;
    container?.remove();
    container = null;
    document.getElementById("paseo-rich-markdown-style")?.remove();
  });

  function renderMarkdown(text: string) {
    act(() => {
      root?.render(
        <QueryClientProvider client={queryClient!}>
          <RichMarkdown text={text} variant="assistant" fallback={<span>fallback</span>} />
        </QueryClientProvider>,
      );
    });
  }

  it("renders task list checked and unchecked states", () => {
    renderMarkdown("- [x] Done\n- [ ] Todo");

    expect(container?.querySelector('[data-testid="rich-markdown-task-checked"]')).not.toBeNull();
    expect(container?.querySelector('[data-testid="rich-markdown-task-open"]')).not.toBeNull();
    expect(container?.textContent).toContain("Done");
    expect(container?.textContent).toContain("Todo");
    expect(container?.textContent).not.toContain("[x]");
    expect(container?.textContent).not.toContain("[ ]");
  });

  it("renders code blocks with language labels and copies raw code", async () => {
    renderMarkdown("```ts\nconst answer: number = 42;\n```");

    expect(container?.querySelector('[data-testid="rich-markdown-code-block"]')).not.toBeNull();
    expect(container?.textContent).toContain("ts");
    expect(container?.textContent).toContain("const");

    await act(async () => {
      (container?.querySelector('button[aria-label="Copy code"]') as HTMLButtonElement).click();
    });

    expect(clipboardMock).toHaveBeenCalledWith("const answer: number = 42;");
  });

  it("wraps tables in a horizontal scroll container", () => {
    renderMarkdown("| Column A | Column B |\n| --- | --- |\n| One | Two |");

    const tableScroll = container?.querySelector('[data-testid="rich-markdown-table-scroll"]');
    expect(tableScroll).not.toBeNull();
    expect(tableScroll?.querySelector("table")).not.toBeNull();
  });

  it("renders valid LaTeX and keeps invalid LaTeX readable", () => {
    renderMarkdown("Good $x^2$ and bad $\\frac{1$");

    expect(container?.querySelector(".katex")).not.toBeNull();
    expect(container?.textContent).toContain("\\frac{1");
  });
});

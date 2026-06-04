/**
 * @vitest-environment jsdom
 */
import React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { JSDOM } from "jsdom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getAppMessages } from "@/i18n/sub2api";
import { ToolCallDetailsContent } from "./tool-call-details";

const { theme } = vi.hoisted(() => ({
  theme: {
    spacing: { 1: 4, 2: 8, 3: 12, 4: 16 },
    borderWidth: { 1: 1 },
    borderRadius: { base: 6, full: 999 },
    fontSize: { xs: 11, sm: 13, base: 15 },
    fontWeight: { normal: "400", semibold: "600" },
    colors: {
      border: "#333",
      destructive: "#ef4444",
      foreground: "#fff",
      foregroundMuted: "#aaa",
      surface1: "#111",
      surface2: "#222",
      surface3: "#333",
    },
  },
}));

vi.mock("@/constants/platform", () => ({
  isWeb: true,
}));

vi.mock("react-native-unistyles", () => ({
  StyleSheet: {
    create: (factory: unknown) => (typeof factory === "function" ? factory(theme) : factory),
  },
}));

vi.mock("react-native-gesture-handler", () => ({
  ScrollView: ({ children }: { children: React.ReactNode }) =>
    React.createElement("div", {}, children),
}));

vi.mock("@/hooks/use-web-scrollbar-style", () => ({
  useWebScrollbarStyle: () => undefined,
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

function render(element: React.ReactElement) {
  act(() => {
    root?.render(element);
  });
}

describe("ToolCallDetailsContent", () => {
  it("renders localized unknown detail and error section labels", () => {
    render(
      <ToolCallDetailsContent
        detail={{
          type: "unknown",
          input: { command: "npm test" },
          output: { ok: true },
        }}
        errorText="boom"
        detailsText={getAppMessages("zh").agentTools.details}
      />,
    );

    expect(container?.textContent).toContain("输入");
    expect(container?.textContent).toContain("输出");
    expect(container?.textContent).toContain("错误");
    expect(container?.textContent).toContain("npm test");
    expect(container?.textContent).toContain("boom");
  });
});

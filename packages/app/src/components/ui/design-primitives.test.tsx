/**
 * @vitest-environment jsdom
 */
import React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { JSDOM } from "jsdom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { theme, clipboardState } = vi.hoisted(() => ({
  theme: {
    spacing: { 1: 4, 2: 8, 3: 12, 4: 16, 6: 24 },
    iconSize: { xs: 10, sm: 14, md: 18, lg: 20 },
    borderRadius: { md: 6, lg: 8, full: 999 },
    fontSize: { xs: 11, sm: 13, base: 15, lg: 18, xl: 20 },
    fontWeight: { normal: "400", medium: "500", semibold: "600" },
    borderWidth: { 1: 1 },
    opacity: { 50: 0.5 },
    colors: {
      foreground: "#fff",
      foregroundMuted: "#aaa",
      surface2: "#222",
      surface3: "#333",
      border: "#444",
      borderAccent: "#555",
      accent: "#0a0",
      accentForeground: "#fff",
      destructive: "#d00",
      statusSuccess: "#16a34a",
      statusWarning: "#f59e0b",
      palette: {
        white: "#fff",
        green: { 400: "#4ade80", 800: "#166534", 900: "#14532d" },
        red: { 500: "#ef4444", 800: "#991b1b", 900: "#7f1d1d" },
      },
    },
  },
  clipboardState: { copied: [] as string[], shouldFail: false },
}));

/** Flattens RN's array styles so assertions can observe which variant styles applied. */
function serializeStyle(style: unknown): string {
  const parts = (Array.isArray(style) ? style : [style]).filter(
    (entry): entry is Record<string, unknown> =>
      typeof entry === "object" && entry !== null && entry !== undefined,
  );
  return JSON.stringify(Object.assign({}, ...parts));
}

function mapNativeProps(props: Record<string, unknown>): Record<string, unknown> {
  const {
    accessibilityLabel,
    accessibilityRole,
    accessibilityState,
    children,
    disabled,
    onPress,
    style,
    testID,
    ...rest
  } = props;
  return {
    ...rest,
    ...(typeof accessibilityLabel === "string" ? { "aria-label": accessibilityLabel } : {}),
    ...(accessibilityRole === "button" ? { role: "button" } : {}),
    ...(typeof testID === "string" ? { "data-testid": testID } : {}),
    ...(style === undefined ? {} : { "data-style": serializeStyle(style) }),
    children,
    disabled: Boolean(disabled) || undefined,
    onClick: typeof onPress === "function" ? () => onPress() : undefined,
  };
}

vi.mock("react-native", () => ({
  ActivityIndicator: () => React.createElement("span", { "data-testid": "spinner" }),
  Pressable: (props: Record<string, unknown>) => {
    const children =
      typeof props.children === "function"
        ? (props.children as (state: unknown) => React.ReactNode)({
            hovered: false,
            pressed: false,
          })
        : props.children;
    return React.createElement("button", mapNativeProps({ ...props, children }));
  },
  Text: (props: Record<string, unknown>) =>
    React.createElement("span", mapNativeProps(props), props.children as React.ReactNode),
  View: (props: Record<string, unknown>) =>
    React.createElement("div", mapNativeProps(props), props.children as React.ReactNode),
}));

vi.mock("react-native-unistyles", () => ({
  StyleSheet: {
    create: (factory: unknown) =>
      typeof factory === "function" ? (factory as (t: unknown) => unknown)(theme) : factory,
  },
  useUnistyles: () => ({ theme }),
}));

vi.mock("expo-clipboard", () => ({
  setStringAsync: async (value: string) => {
    if (clipboardState.shouldFail) {
      throw new Error("clipboard unavailable");
    }
    clipboardState.copied.push(value);
  },
}));

vi.mock("lucide-react-native", () => ({
  Check: () => React.createElement("span", { "data-testid": "check-icon" }),
  Copy: () => React.createElement("span", { "data-testid": "copy-icon" }),
}));

import { Button } from "./button";
import { FactRow } from "./fact-row";
import { SettingsRow } from "./settings-row";
import { StatCard } from "./stat-card";
import { StatusBadge } from "./status-badge";
import { StatusPanel } from "./status-panel";

let container: HTMLDivElement;
let root: Root;

function render(node: React.ReactNode) {
  act(() => {
    root.render(node);
  });
}

beforeEach(() => {
  const dom = new JSDOM("<!doctype html><html><body></body></html>");
  // The JSX transform here is classic, so components need React in global scope.
  vi.stubGlobal("React", React);
  vi.stubGlobal("window", dom.window);
  vi.stubGlobal("document", dom.window.document);
  vi.stubGlobal("HTMLElement", dom.window.HTMLElement);
  vi.stubGlobal("Node", dom.window.Node);
  vi.stubGlobal("navigator", dom.window.navigator);
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);

  clipboardState.copied = [];
  clipboardState.shouldFail = false;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => {
    root.unmount();
  });
  container.remove();
  vi.useRealTimers();
});

describe("SettingsRow", () => {
  it("renders as an information row when no control is supplied", () => {
    render(<SettingsRow title="Network path" description="Direct first, then the mirror." />);

    expect(container.textContent).toContain("Network path");
    expect(container.textContent).toContain("Direct first, then the mirror.");
    expect(container.querySelectorAll("button")).toHaveLength(0);
  });

  it("renders the supplied control alongside the text", () => {
    render(
      <SettingsRow title="Theme">
        <Button>Change</Button>
      </SettingsRow>,
    );

    expect(container.querySelector("button")?.textContent).toBe("Change");
  });
});

describe("StatusBadge", () => {
  it("distinguishes pending from muted so 'checking' does not read as 'not installed'", () => {
    render(<StatusBadge label="Checking" variant="pending" />);
    const pendingPill = container.querySelector("div")?.getAttribute("data-style");

    render(<StatusBadge label="Not installed" variant="muted" />);
    const mutedPill = container.querySelector("div")?.getAttribute("data-style");

    expect(pendingPill).toBeTruthy();
    expect(pendingPill).not.toBe(mutedPill);
  });

  it("renders an icon slot before the label", () => {
    render(
      <StatusBadge
        label="Updating"
        variant="warning"
        icon={<span data-testid="badge-icon">*</span>}
      />,
    );

    expect(container.querySelector('[data-testid="badge-icon"]')).not.toBeNull();
  });
});

describe("Button busy state", () => {
  it("swaps the icon for a spinner in place and blocks presses", () => {
    const onPress = vi.fn();
    render(
      <Button busy onPress={onPress} leftIcon={() => <span data-testid="leading">i</span>}>
        Installing
      </Button>,
    );

    expect(container.querySelector('[data-testid="spinner"]')).not.toBeNull();
    expect(container.querySelector('[data-testid="leading"]')).toBeNull();

    const button = container.querySelector("button");
    expect(button?.disabled).toBe(true);
    act(() => {
      button?.click();
    });
    expect(onPress).not.toHaveBeenCalled();
  });

  it("shows the icon and stays pressable when not busy", () => {
    const onPress = vi.fn();
    render(
      <Button onPress={onPress} leftIcon={() => <span data-testid="leading">i</span>}>
        Install
      </Button>,
    );

    expect(container.querySelector('[data-testid="spinner"]')).toBeNull();
    act(() => {
      container.querySelector("button")?.click();
    });
    expect(onPress).toHaveBeenCalledTimes(1);
  });
});

describe("FactRow", () => {
  it("renders a label and value without a copy button by default", () => {
    render(<FactRow label="Version" value="1.0.4" />);

    expect(container.textContent).toContain("Version");
    expect(container.textContent).toContain("1.0.4");
    expect(container.querySelectorAll("button")).toHaveLength(0);
  });

  it("copies the value and reverts the confirmation after the feedback window", async () => {
    vi.useFakeTimers();
    render(<FactRow label="Install command" value="npm i -g @xai-official/grok" copyable />);

    expect(container.querySelector('[data-testid="copy-icon"]')).not.toBeNull();

    await act(async () => {
      container.querySelector("button")?.click();
      await Promise.resolve();
    });

    expect(clipboardState.copied).toEqual(["npm i -g @xai-official/grok"]);
    expect(container.querySelector('[data-testid="check-icon"]')).not.toBeNull();

    act(() => {
      vi.advanceTimersByTime(1400);
    });
    expect(container.querySelector('[data-testid="copy-icon"]')).not.toBeNull();
  });

  it("keeps rendering when the clipboard rejects", async () => {
    clipboardState.shouldFail = true;
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    render(<FactRow label="Config" value="~/.grok" copyable />);

    await act(async () => {
      container.querySelector("button")?.click();
      await Promise.resolve();
    });

    expect(container.textContent).toContain("~/.grok");
    expect(container.querySelector('[data-testid="check-icon"]')).toBeNull();
    consoleError.mockRestore();
  });
});

describe("StatCard", () => {
  it("renders the hint slot for methodology disclosure", () => {
    render(<StatCard label="Duration" value="1h 12m" hint="Accumulated per completed turn" />);

    expect(container.textContent).toContain("Duration");
    expect(container.textContent).toContain("1h 12m");
    expect(container.textContent).toContain("Accumulated per completed turn");
  });
});

describe("StatusPanel", () => {
  it("shows a spinner instead of the icon while loading", () => {
    render(<StatusPanel title="Loading models" loading icon={<span data-testid="icon">i</span>} />);

    expect(container.querySelector('[data-testid="spinner"]')).not.toBeNull();
    expect(container.querySelector('[data-testid="icon"]')).toBeNull();
  });

  it("renders a recovery action alongside the description", () => {
    render(
      <StatusPanel
        title="Couldn't reach the daemon"
        description="Check that it is running."
        error
        action={<Button>Retry</Button>}
      />,
    );

    expect(container.textContent).toContain("Couldn't reach the daemon");
    expect(container.querySelector("button")?.textContent).toBe("Retry");
  });
});

/**
 * @vitest-environment jsdom
 */
import React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { OnboardingGuideRow } from "./onboarding-guide-row";

const { theme } = vi.hoisted(() => ({
  theme: {
    spacing: { 1: 4, 2: 8, 3: 12, 4: 16 },
    borderRadius: { md: 6, lg: 8 },
    borderWidth: { 1: 1 },
    fontSize: { xs: 12, sm: 14, base: 16 },
    fontWeight: { normal: "400" },
    opacity: { 50: 0.5 },
    iconSize: { sm: 14, md: 18 },
    colors: {
      accent: "#2563eb",
      accentForeground: "#fff",
      border: "#ddd",
      borderAccent: "#aaa",
      destructive: "#dc2626",
      foreground: "#111",
      foregroundMuted: "#666",
      palette: { white: "#fff" },
      surface1: "#fafafa",
      surface2: "#f0f0f0",
      surface3: "#e5e7eb",
    },
  },
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
    disabled,
    onHoverIn,
    onHoverOut,
    onPress,
    style,
    testID,
  } = props;
  const resolvedStyle =
    typeof style === "function" ? style({ hovered: false, pressed: false }) : style;
  return {
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
  View: (props: Record<string, unknown>) => React.createElement("div", mapProps(props)),
}));

vi.mock("react-native-unistyles", () => ({
  StyleSheet: {
    create: (factory: unknown) => (typeof factory === "function" ? factory(theme) : factory),
  },
  useUnistyles: () => ({ theme }),
}));

vi.mock("lucide-react-native", () => ({
  BookOpen: (...args: unknown[]) =>
    React.createElement("span", {
      ...((args[0] as Record<string, unknown> | undefined) ?? {}),
      "data-icon": "BookOpen",
    }),
}));

let root: Root | null = null;
let container: HTMLElement | null = null;

beforeEach(() => {
  vi.stubGlobal("React", React);
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
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
  container?.remove();
  root = null;
  container = null;
  vi.unstubAllGlobals();
});

describe("OnboardingGuideRow", () => {
  it("calls the replay handler from settings", () => {
    const onReplay = vi.fn();

    act(() => {
      root?.render(
        <OnboardingGuideRow
          title="Onboarding guide"
          hint="Review the core flow."
          actionLabel="Replay"
          accessibilityLabel="Replay onboarding guide"
          onReplay={onReplay}
        />,
      );
    });

    act(() => {
      (
        container?.querySelector('[data-testid="settings-replay-onboarding-guide"]') as HTMLElement
      ).click();
    });

    expect(onReplay).toHaveBeenCalledTimes(1);
  });
});

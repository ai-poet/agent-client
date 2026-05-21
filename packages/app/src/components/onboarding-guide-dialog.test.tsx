/**
 * @vitest-environment jsdom
 */
import React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { OnboardingGuideDialog } from "./onboarding-guide-dialog";
import { useOnboardingGuideStore } from "@/stores/onboarding-guide-store";

const { theme, updateSettingsMock } = vi.hoisted(() => ({
  updateSettingsMock: vi.fn(),
  theme: {
    spacing: { 1: 4, 2: 8, 3: 12, 4: 16, 6: 24 },
    iconSize: { sm: 14, md: 18 },
    borderRadius: { lg: 8, "2xl": 16, full: 999 },
    borderWidth: { 1: 1 },
    fontSize: { xs: 12, sm: 14, lg: 18 },
    fontWeight: { medium: "500", normal: "400" },
    opacity: { 50: 0.5 },
    colors: {
      accent: "#2563eb",
      accentForeground: "#fff",
      border: "#ddd",
      foreground: "#111",
      foregroundMuted: "#666",
      palette: { white: "#fff" },
      surface0: "#fff",
      surface1: "#fafafa",
      surface2: "#f0f0f0",
      surface3: "#e5e7eb",
      destructive: "#dc2626",
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

vi.mock("lucide-react-native", () => {
  const createIcon = (name: string) => (props: Record<string, unknown>) =>
    React.createElement("span", { ...props, "data-icon": name });
  return {
    BotMessageSquare: createIcon("BotMessageSquare"),
    CheckCircle2: createIcon("CheckCircle2"),
    FolderOpen: createIcon("FolderOpen"),
    GitBranchPlus: createIcon("GitBranchPlus"),
    SlidersHorizontal: createIcon("SlidersHorizontal"),
  };
});

vi.mock("@/components/adaptive-modal-sheet", () => ({
  AdaptiveModalSheet: ({
    title,
    visible,
    children,
    testID,
  }: {
    title: string;
    visible: boolean;
    children: React.ReactNode;
    testID?: string;
  }) =>
    visible ? (
      <div data-testid={testID}>
        <h2>{title}</h2>
        {children}
      </div>
    ) : null,
}));

vi.mock("@/hooks/use-sub2api-locale", () => ({
  useSub2APILocale: () => "en",
}));

vi.mock("@/hooks/use-settings", () => ({
  useAppSettings: () => ({
    settings: { language: "en", onboardingGuideCompleted: false },
    isLoading: false,
    error: null,
    updateSettings: updateSettingsMock,
    resetSettings: vi.fn(),
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
  updateSettingsMock.mockReset();
  useOnboardingGuideStore.setState({
    open: true,
    source: "manual",
    stepIndex: 0,
    autoPrompted: false,
  });
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

function renderDialog() {
  act(() => {
    root?.render(<OnboardingGuideDialog />);
  });
}

describe("OnboardingGuideDialog", () => {
  it("renders the first onboarding step", () => {
    renderDialog();

    expect(container?.textContent).toContain("Quick start");
    expect(container?.textContent).toContain("Open a project");
  });

  it("moves forward and backward through steps", () => {
    renderDialog();

    act(() => {
      (container?.querySelector('[data-testid="onboarding-guide-next"]') as HTMLElement).click();
    });

    expect(container?.textContent).toContain("Message your agent");

    act(() => {
      (
        container?.querySelector('[data-testid="onboarding-guide-previous"]') as HTMLElement
      ).click();
    });

    expect(container?.textContent).toContain("Open a project");
  });

  it("marks onboarding complete when finishing", () => {
    useOnboardingGuideStore.setState({ stepIndex: 4 });
    renderDialog();

    act(() => {
      (container?.querySelector('[data-testid="onboarding-guide-finish"]') as HTMLElement).click();
    });

    expect(updateSettingsMock).toHaveBeenCalledWith({ onboardingGuideCompleted: true });
    expect(useOnboardingGuideStore.getState().open).toBe(false);
  });

  it("opens manually at the first step without route context", () => {
    useOnboardingGuideStore.setState({ open: false, stepIndex: 3 });
    act(() => {
      useOnboardingGuideStore.getState().openGuide({ source: "manual" });
    });
    renderDialog();

    expect(useOnboardingGuideStore.getState().source).toBe("manual");
    expect(container?.textContent).toContain("Open a project");
  });
});

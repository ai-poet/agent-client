/**
 * @vitest-environment jsdom
 */
import React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AgentList } from "./agent-list";
import type { AggregatedAgent } from "@/hooks/use-aggregated-agents";

const { desktopMotionState, theme } = vi.hoisted(() => ({
  desktopMotionState: {
    enabled: true,
  },
  theme: {
    spacing: { 1: 4, 2: 8, 3: 12, 4: 16, 6: 24 },
    iconSize: { xs: 10, sm: 14, md: 18 },
    borderRadius: { lg: 8, full: 999, "2xl": 16 },
    fontSize: { xs: 11, sm: 13, base: 15, lg: 18 },
    fontWeight: { medium: "500", semibold: "600" },
    colors: {
      foreground: "#fff",
      foregroundMuted: "#aaa",
      surface1: "#111",
      surface2: "#222",
      primary: "#2563eb",
      primaryForeground: "#fff",
      palette: {
        amber: { 500: "#f59e0b" },
        red: { 300: "#fca5a5" },
      },
    },
  },
}));

function mapNativeProps(props: Record<string, unknown>): Record<string, unknown> {
  const {
    accessibilityLabel,
    accessibilityRole,
    children,
    disabled,
    onLongPress,
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
    children,
    disabled: Boolean(disabled) || undefined,
    onClick:
      typeof onPress === "function"
        ? (event: React.MouseEvent) => onPress({ stopPropagation: () => event.stopPropagation() })
        : undefined,
    onDoubleClick:
      typeof onLongPress === "function"
        ? (event: React.MouseEvent) =>
            onLongPress({ stopPropagation: () => event.stopPropagation() })
        : undefined,
  };
}

vi.mock("react-native", () => ({
  FlatList: ({ data, renderItem, keyExtractor, ListFooterComponent }: any) =>
    React.createElement(
      "div",
      {},
      data.map((item: unknown, index: number) =>
        React.createElement(
          React.Fragment,
          { key: keyExtractor?.(item, index) ?? index },
          renderItem({ item, index }),
        ),
      ),
      ListFooterComponent ?? null,
    ),
  Modal: ({ children, visible }: { children: React.ReactNode; visible: boolean }) =>
    visible ? React.createElement("div", {}, children) : null,
  Platform: {
    OS: "web",
  },
  Pressable: (props: Record<string, unknown>) => {
    const children =
      typeof props.children === "function"
        ? props.children({ hovered: false, pressed: false })
        : props.children;
    return React.createElement("button", mapNativeProps({ ...props, children }));
  },
  RefreshControl: () => null,
  Text: (props: Record<string, unknown>) =>
    React.createElement("span", mapNativeProps(props), props.children as React.ReactNode),
  View: (props: Record<string, unknown>) =>
    React.createElement("div", mapNativeProps(props), props.children as React.ReactNode),
}));

vi.mock("react-native-safe-area-context", () => ({
  useSafeAreaInsets: () => ({ top: 0, right: 0, bottom: 0, left: 0 }),
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

vi.mock("@/hooks/use-desktop-agent-motion-enabled", () => ({
  useDesktopAgentMotionEnabled: () => desktopMotionState.enabled,
}));

vi.mock("@/components/provider-icons", () => ({
  getProviderIcon: () => () => React.createElement("span", { "data-testid": "provider-icon" }),
}));

vi.mock("lucide-react-native", () => ({
  Archive: () => React.createElement("span", { "data-testid": "archive-icon" }),
}));

vi.mock("expo-router", () => ({
  router: {
    navigate: vi.fn(),
  },
}));

function makeAgent(overrides: Partial<AggregatedAgent> = {}): AggregatedAgent {
  return {
    id: "agent-1",
    serverId: "server-1",
    serverLabel: "Localhost",
    title: "Desktop motion polish",
    status: "running",
    lastActivityAt: new Date("2026-06-03T10:00:00.000Z"),
    cwd: "/Users/me/project",
    provider: "codex",
    pendingPermissionCount: 2,
    requiresAttention: true,
    attentionReason: "permission",
    attentionTimestamp: new Date("2026-06-03T10:01:00.000Z"),
    archivedAt: null,
    createdAt: new Date("2026-06-03T09:00:00.000Z"),
    labels: {},
    ...overrides,
  };
}

async function renderAgentList(agent: AggregatedAgent) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);

  await act(async () => {
    root.render(
      <AgentList
        agents={[agent]}
        selectedAgentId={`${agent.serverId}:${agent.id}`}
        showAttentionIndicator
      />,
    );
  });

  return { container, root };
}

describe("AgentList desktop motion", () => {
  let root: Root | null = null;
  let container: HTMLElement | null = null;

  beforeEach(() => {
    desktopMotionState.enabled = true;
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
  });

  it("keeps row test IDs and status badges stable when desktop motion is enabled", async () => {
    const rendered = await renderAgentList(makeAgent());
    root = rendered.root;
    container = rendered.container;

    const row = container.querySelector('[data-testid="agent-row-server-1-agent-1"]');

    expect(row).not.toBeNull();
    expect(container.textContent).toContain("Desktop motion polish");
    expect(container.textContent).toContain("2 pending");
    expect(container.textContent).toContain("Attention");
  });
});

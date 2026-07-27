/**
 * @vitest-environment jsdom
 */
import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentModelDefinition } from "@server/server/agent/agent-sdk-types";

const { platformState, selectorProps, theme } = vi.hoisted(() => ({
  platformState: { isWeb: true },
  selectorProps: [] as Array<Record<string, unknown>>,
  theme: {
    spacing: { 1: 4, 2: 8, 3: 12, 4: 16 },
    iconSize: { sm: 14, md: 18, lg: 22 },
    borderRadius: { lg: 8, full: 999, "2xl": 16 },
    fontSize: { sm: 13, base: 15 },
    fontWeight: { normal: "400", semibold: "600" },
    colors: {
      foreground: "#111",
      foregroundMuted: "#777",
      surface0: "#fff",
      surface2: "#eee",
      border: "#ddd",
      palette: {
        blue: { 400: "#60a5fa", 500: "#3b82f6" },
        green: { 500: "#22c55e" },
        purple: { 500: "#a855f7" },
        red: { 500: "#ef4444" },
        yellow: { 400: "#facc15" },
      },
    },
  },
}));

function mapNativeProps(props: Record<string, unknown>): Record<string, unknown> {
  const {
    accessibilityLabel,
    accessibilityRole,
    children,
    collapsable,
    ellipsizeMode,
    numberOfLines,
    onPress,
    pointerEvents,
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
    onClick: typeof onPress === "function" ? onPress : undefined,
  };
}

vi.mock("react-native", () => ({
  Keyboard: { dismiss: vi.fn() },
  Platform: {
    OS: "web",
    select: (specifics: Record<string, unknown>) => specifics.web ?? specifics.default,
  },
  Pressable: (props: Record<string, unknown>) => {
    const children =
      typeof props.children === "function"
        ? props.children({ hovered: false, pressed: false })
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
    create: (factory: unknown) => (typeof factory === "function" ? factory(theme) : factory),
  },
  useUnistyles: () => ({ theme }),
}));

vi.mock("@/constants/platform", () => ({
  get isWeb() {
    return platformState.isWeb;
  },
  get isNative() {
    return !platformState.isWeb;
  },
}));

vi.mock("lucide-react-native", () => {
  const Icon = () => React.createElement("span");
  return {
    Brain: Icon,
    ChevronDown: Icon,
    Cloud: Icon,
    ListTodo: Icon,
    Maximize2: Icon,
    Settings2: Icon,
    ShieldAlert: Icon,
    ShieldCheck: Icon,
    ShieldOff: Icon,
    Zap: Icon,
  };
});

vi.mock("@/components/provider-icons", () => ({
  getProviderIcon: () => () => React.createElement("span", { "data-testid": "provider-icon" }),
}));

vi.mock("@/components/combined-model-selector", () => ({
  CombinedModelSelector: (props: Record<string, unknown>) => {
    selectorProps.push(props);
    const renderTrigger = props.renderTrigger as
      | ((input: {
          selectedModelLabel: string;
          onPress: () => void;
          disabled: boolean;
          isOpen: boolean;
        }) => React.ReactNode)
      | undefined;
    return React.createElement(
      "div",
      { "data-testid": "combined-model-selector" },
      renderTrigger?.({
        selectedModelLabel: "Opus 5",
        onPress: () => undefined,
        disabled: false,
        isOpen: false,
      }),
    );
  },
}));

vi.mock("@/components/ui/combobox", () => ({
  Combobox: () => null,
  ComboboxItem: () => null,
}));

vi.mock("@/components/ui/dropdown-menu", () => ({
  DropdownMenu: ({ children }: { children: React.ReactNode }) => children,
  DropdownMenuContent: ({ children }: { children: React.ReactNode }) => children,
  DropdownMenuItem: ({ children }: { children: React.ReactNode }) => children,
  DropdownMenuTrigger: (props: Record<string, unknown>) =>
    React.createElement("button", mapNativeProps(props)),
}));

vi.mock("@/components/ui/tooltip", () => ({
  Tooltip: ({ children }: { children: React.ReactNode }) => children,
  TooltipContent: ({ children }: { children: React.ReactNode }) => children,
  TooltipTrigger: ({ children }: { children: React.ReactNode }) => children,
}));

vi.mock("@/components/adaptive-modal-sheet", () => ({
  AdaptiveModalSheet: ({ children, visible }: { children: React.ReactNode; visible: boolean }) =>
    visible ? React.createElement("div", {}, children) : null,
}));

vi.mock("@/hooks/use-form-preferences", () => ({
  buildFavoriteModelKey: ({ provider, modelId }: { provider: string; modelId: string }) =>
    `${provider}:${modelId}`,
  mergeProviderPreferences: ({ preferences }: { preferences: unknown }) => preferences,
  toggleFavoriteModel: ({ preferences }: { preferences: unknown }) => preferences,
  useFormPreferences: () => ({
    preferences: { favoriteModels: [] },
    updatePreferences: vi.fn(async () => undefined),
  }),
}));

vi.mock("@/hooks/use-cloud-model-routing", () => ({
  useCloudModelRouting: () => ({
    cloudGroups: [
      {
        provider: "claude",
        groupId: 1,
        groupLabel: "Claude Route",
        platform: "anthropic",
        models: [
          { id: "claude-opus-5", label: "Opus 5" },
          { id: "claude-opus-5[1m]", label: "Opus 5 1M" },
        ],
      },
      {
        provider: "codex",
        groupId: 2,
        groupLabel: "Codex Route",
        platform: "openai",
        models: [{ id: "gpt-5.4", label: "GPT-5.4" }],
      },
    ],
  }),
}));

vi.mock("@/hooks/use-app-locale", () => ({
  useAppLocale: () => "en",
}));

vi.mock("@/hooks/use-providers-snapshot", () => ({
  useProvidersSnapshot: () => ({
    entries: [],
    isLoading: false,
    refetchIfStale: vi.fn(),
  }),
}));

vi.mock("@/stores/session-store", () => ({
  useSessionStore: vi.fn(),
}));

vi.mock("@/contexts/toast-context", () => ({
  useToast: () => ({ error: vi.fn() }),
}));

import { DraftAgentStatusBar } from "./agent-status-bar";

const providerDefinitions = [
  {
    id: "claude",
    label: "Claude",
    description: "Claude provider",
    defaultModeId: "default",
    modes: [],
  },
  {
    id: "codex",
    label: "Codex",
    description: "Codex provider",
    defaultModeId: "auto",
    modes: [],
  },
];
const claudeModels = [
  { provider: "claude" as const, id: "claude-opus-5", label: "Opus 5", isDefault: true },
  { provider: "claude" as const, id: "claude-opus-5[1m]", label: "Opus 5 1M" },
];
const codexModels = [
  { provider: "codex" as const, id: "gpt-5.4", label: "GPT-5.4", isDefault: true },
];

function renderStatusBar(root: Root) {
  const onSelectProviderAndModel = vi.fn();
  act(() => {
    root.render(
      <DraftAgentStatusBar
        providerDefinitions={providerDefinitions}
        selectableProviderIds={["claude", "codex"]}
        selectedProvider="claude"
        onSelectProvider={vi.fn()}
        modeOptions={[]}
        selectedMode=""
        onSelectMode={vi.fn()}
        models={claudeModels}
        selectedModel="claude-opus-5[1m]"
        onSelectModel={vi.fn()}
        isModelLoading={false}
        allProviderModels={
          new Map<string, AgentModelDefinition[]>([
            ["claude", claudeModels],
            ["codex", codexModels],
          ])
        }
        isAllModelsLoading={false}
        onSelectProviderAndModel={onSelectProviderAndModel}
        thinkingOptions={[]}
        selectedThinkingOptionId=""
        onSelectThinkingOption={vi.fn()}
      />,
    );
  });
  return { onSelectProviderAndModel };
}

describe("DraftAgentStatusBar provider and model controls", () => {
  let container: HTMLElement;
  let root: Root;

  beforeEach(() => {
    platformState.isWeb = true;
    selectorProps.length = 0;
    vi.stubGlobal("React", React);
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
    vi.unstubAllGlobals();
  });

  it("renders separate desktop provider and model selectors with provider-scoped data", () => {
    const { onSelectProviderAndModel } = renderStatusBar(root);

    expect(container.querySelector('[data-testid="agent-provider-selector"]')).not.toBeNull();
    expect(container.querySelector('[data-testid="combined-model-selector"]')).not.toBeNull();
    expect(container.querySelector('[data-testid="agent-context-selector"]')).not.toBeNull();

    const props = selectorProps.at(-1)!;
    expect(Array.from((props.allProviderModels as Map<string, unknown>).keys())).toEqual([
      "claude",
    ]);
    expect(
      (props.cloudGroups as Array<{ provider: string }>).map((group) => group.provider),
    ).toEqual(["claude"]);
    expect(
      (props.allProviderModels as Map<string, AgentModelDefinition[]>)
        .get("claude")
        ?.map((model) => model.id),
    ).toEqual(["claude-opus-5"]);
    expect(props.selectedModel).toBe("claude-opus-5");
    expect(
      (props.cloudGroups as Array<{ models: Array<{ id: string }> }>)[0]?.models.map(
        (model) => model.id,
      ),
    ).toEqual(["claude-opus-5"]);

    act(() => {
      (props.onSelect as (provider: string, modelId: string) => void)("claude", "claude-opus-5");
    });
    expect(onSelectProviderAndModel).toHaveBeenCalledWith("claude", "claude-opus-5[1m]");
  });

  it("renders separate provider and model rows in the mobile preferences sheet", () => {
    platformState.isWeb = false;
    renderStatusBar(root);

    const preferencesButton = container.querySelector('[data-testid="agent-preferences-button"]');
    expect(preferencesButton).not.toBeNull();
    act(() => {
      preferencesButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(container.querySelector('[data-testid="agent-preferences-provider"]')).not.toBeNull();
    expect(container.querySelector('[data-testid="agent-preferences-model"]')).not.toBeNull();
    expect(container.querySelector('[data-testid="agent-preferences-context"]')).not.toBeNull();
  });
});

/**
 * @vitest-environment jsdom
 */
import { act } from "@testing-library/react";
import React from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { WorkspaceDraftAgentTab } from "./workspace-draft-agent-tab";

const { composerSubmitMock, createFlowSubmitMock, draftInput, theme, workspaceAuthority } =
  vi.hoisted(() => {
    const createFlowSubmitMock = vi.fn(async () => undefined);
    return {
      composerSubmitMock: vi.fn(),
      createFlowSubmitMock,
      draftInput: {
        text: "Build a focused view",
        setText: vi.fn(),
        attachments: [],
        setAttachments: vi.fn(),
        cwd: "/repo/client",
        clear: vi.fn(),
        composerState: {
          providerDefinitions: [{ id: "codex", label: "Codex", modes: [] }],
          selectedProvider: "codex",
          setProviderFromUser: vi.fn(),
          modeOptions: [],
          selectedMode: "",
          setModeFromUser: vi.fn(),
          availableModels: [{ provider: "codex", id: "gpt-5.4", label: "GPT-5.4" }],
          selectedModel: "gpt-5.4",
          setModelFromUser: vi.fn(),
          isModelLoading: false,
          allProviderModels: new Map(),
          isAllModelsLoading: false,
          availableThinkingOptions: [],
          selectedThinkingOptionId: "",
          setThinkingOptionFromUser: vi.fn(),
          setProviderAndModelFromUser: vi.fn(),
          persistFormPreferences: vi.fn(async () => undefined),
          effectiveModelId: "gpt-5.4",
          effectiveThinkingOptionId: "",
          featureValues: {},
          commandDraftConfig: undefined,
          statusControls: {
            providerDefinitions: [{ id: "codex", label: "Codex", modes: [] }],
            selectedProvider: "codex",
            onSelectProvider: vi.fn(),
            modeOptions: [],
            selectedMode: "",
            onSelectMode: vi.fn(),
            models: [{ provider: "codex", id: "gpt-5.4", label: "GPT-5.4" }],
            selectedModel: "gpt-5.4",
            onSelectModel: vi.fn(),
            isModelLoading: false,
            allProviderModels: new Map(),
            isAllModelsLoading: false,
            onSelectProviderAndModel: vi.fn(),
            thinkingOptions: [],
            selectedThinkingOptionId: "",
            onSelectThinkingOption: vi.fn(),
            features: [],
            onSetFeature: vi.fn(),
          },
        },
      },
      theme: {
        spacing: { 1: 4, 2: 8, 3: 12, 4: 16, 6: 24, 8: 32 },
        borderRadius: { md: 6, lg: 8 },
        fontSize: { xs: 11, sm: 13, "4xl": 36 },
        fontWeight: { medium: "500" },
        shadow: { md: {} },
        colors: {
          surface0: "#fff",
          surface1: "#f8f8f8",
          surface2: "#f3f3f3",
          foreground: "#111",
          foregroundMuted: "#777",
          border: "#ddd",
          destructive: "#d00",
        },
      },
      workspaceAuthority: {
        ok: true,
        authority: {
          workspaceId: "workspace-1",
          workspaceDirectory: "/repo/client",
          workspace: {
            id: "workspace-1",
            projectId: "project-1",
            projectDisplayName: "client",
            projectRootPath: "/repo",
            workspaceDirectory: "/repo/client",
            projectKind: "repository",
            workspaceKind: "main",
            name: "main",
            status: "ready",
            diffStat: null,
            scripts: [],
            worktreePersona: null,
          },
        },
      },
    };
  });

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

function mapNativeProps(props: Record<string, unknown>): Record<string, unknown> {
  const {
    accessibilityLabel,
    accessibilityRole,
    children,
    contentContainerStyle,
    keyboardShouldPersistTaps,
    onPress,
    style,
    testID,
    ...rest
  } = props;
  return {
    ...rest,
    ...(normalizeStyle(style) ? { style: normalizeStyle(style) } : {}),
    ...(normalizeStyle(contentContainerStyle)
      ? { "data-content-style": JSON.stringify(normalizeStyle(contentContainerStyle)) }
      : {}),
    ...(typeof accessibilityLabel === "string" ? { "aria-label": accessibilityLabel } : {}),
    ...(accessibilityRole === "button" ? { role: "button" } : {}),
    ...(typeof testID === "string" ? { "data-testid": testID } : {}),
    children,
    onClick:
      typeof onPress === "function"
        ? (event: React.MouseEvent) => onPress({ stopPropagation: () => event.stopPropagation() })
        : undefined,
  };
}

vi.mock("react-native-unistyles", () => ({
  StyleSheet: {
    create: (factory: unknown) => (typeof factory === "function" ? factory(theme) : factory),
  },
  useUnistyles: () => ({ theme }),
}));

vi.mock("react-native", () => ({
  ActivityIndicator: (props: Record<string, unknown>) =>
    React.createElement("span", mapNativeProps(props)),
  Keyboard: { dismiss: vi.fn() },
  ScrollView: ({ children, ...props }: Record<string, unknown>) =>
    React.createElement("div", mapNativeProps(props), children as React.ReactNode),
  Text: ({ children, ...props }: Record<string, unknown>) =>
    React.createElement("span", mapNativeProps(props), children as React.ReactNode),
  View: ({ children, ...props }: Record<string, unknown>) =>
    React.createElement("div", mapNativeProps(props), children as React.ReactNode),
}));

vi.mock("react-native-safe-area-context", () => ({
  useSafeAreaInsets: () => ({ top: 0, right: 0, bottom: 0, left: 0 }),
}));

vi.mock("@/components/composer", () => ({
  Composer: (props: {
    onSubmitMessage?: (payload: { text: string; attachments: []; cwd: string }) => Promise<void>;
    value: string;
    cwd: string;
  }) =>
    React.createElement(
      "button",
      {
        "data-testid": "workspace-draft-composer",
        onClick: async () => {
          composerSubmitMock();
          await props.onSubmitMessage?.({ text: props.value, attachments: [], cwd: props.cwd });
        },
      },
      "Composer",
    ),
}));

vi.mock("@/components/file-drop-zone", () => ({
  FileDropZone: ({ children }: { children: React.ReactNode }) =>
    React.createElement("div", null, children),
}));

vi.mock("@/components/agent-stream-view", () => ({
  AgentStreamView: () => React.createElement("div", { "data-testid": "agent-stream-view" }),
}));

vi.mock("@/hooks/use-app-locale", () => ({
  useAppLocale: () => "zh",
}));

vi.mock("@/hooks/use-agent-input-draft", () => ({
  useAgentInputDraft: () => draftInput,
}));

vi.mock("@/hooks/use-draft-agent-create-flow", () => ({
  useDraftAgentCreateFlow: () => ({
    formErrorMessage: "",
    isSubmitting: false,
    optimisticStreamItems: [],
    draftAgent: null,
    handleCreateFromInput: createFlowSubmitMock,
  }),
}));

vi.mock("@/runtime/host-runtime", () => ({
  useHostRuntimeClient: () => ({ createAgent: vi.fn() }),
  useHostRuntimeIsConnected: () => true,
}));

vi.mock("@/screens/workspace/workspace-draft-agent-config", () => ({
  buildWorkspaceDraftAgentConfig: vi.fn(),
}));

vi.mock("@/screens/workspace/workspace-draft-setup-loading", () => ({
  shouldShowWorkspaceDraftSetupLoading: () => false,
}));

vi.mock("@/stores/draft-keys", () => ({
  buildDraftStoreKey: () => "draft-key",
}));

vi.mock("@/stores/workspace-setup-store", () => ({
  useWorkspaceSetupStore: (selector: (state: { snapshots: Record<string, unknown> }) => unknown) =>
    selector({ snapshots: {} }),
}));

vi.mock("@/stores/session-store-hooks", () => ({
  useWorkspaceExecutionAuthority: () => workspaceAuthority,
}));

vi.mock("@/stores/workspace-tabs-store", () => ({
  buildWorkspaceTabPersistenceKey: () => "srv:workspace-1",
}));

vi.mock("@/utils/encode-images", () => ({
  encodeImages: vi.fn(async () => []),
}));

vi.mock("@/screens/workspace/workspace-draft-pane-focus", () => ({
  shouldAutoFocusWorkspaceDraftComposer: () => true,
}));

vi.mock("@/constants/platform", () => ({
  isWeb: true,
}));

vi.stubGlobal("React", React);

describe("WorkspaceDraftAgentTab", () => {
  let root: Root | null = null;
  let container: HTMLElement | null = null;

  beforeEach(() => {
    composerSubmitMock.mockClear();
    createFlowSubmitMock.mockClear();
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
    container?.remove();
    container = null;
  });

  it("renders the centered draft prompt for the current workspace", () => {
    act(() => {
      root?.render(
        <WorkspaceDraftAgentTab
          serverId="srv"
          workspaceId="workspace-1"
          tabId="draft_draft-1"
          draftId="draft-1"
          isPaneFocused={true}
          onCreated={vi.fn()}
          onOpenWorkspaceFile={vi.fn()}
        />,
      );
    });

    expect(
      container?.querySelector('[data-testid="workspace-draft-hero-title"]')?.textContent,
    ).toBe("要在 client 中构建什么？");
    expect(container?.querySelector('[data-testid="workspace-draft-composer"]')).not.toBeNull();
    expect(container?.querySelector('[data-testid="agent-stream-view"]')).toBeNull();
  });

  it("keeps composer submit wired to the draft create flow", async () => {
    act(() => {
      root?.render(
        <WorkspaceDraftAgentTab
          serverId="srv"
          workspaceId="workspace-1"
          tabId="draft_draft-1"
          draftId="draft-1"
          isPaneFocused={true}
          onCreated={vi.fn()}
          onOpenWorkspaceFile={vi.fn()}
        />,
      );
    });

    await act(async () => {
      container
        ?.querySelector<HTMLButtonElement>('[data-testid="workspace-draft-composer"]')
        ?.click();
    });

    expect(composerSubmitMock).toHaveBeenCalledTimes(1);
    expect(createFlowSubmitMock).toHaveBeenCalledWith({
      text: "Build a focused view",
      attachments: [],
      cwd: "/repo/client",
    });
  });
});

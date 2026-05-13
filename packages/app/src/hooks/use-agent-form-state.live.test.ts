import { renderHook, waitFor } from "@testing-library/react";
import { JSDOM } from "jsdom";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentProvider, ProviderSnapshotEntry } from "@server/server/agent/agent-sdk-types";
import type { FormPreferences } from "./use-form-preferences";
import { useAgentFormState } from "./use-agent-form-state";

const { mocks } = vi.hoisted(() => ({
  mocks: {
    preferences: {} as FormPreferences,
    isPreferencesLoading: false,
    snapshotEntries: [] as ProviderSnapshotEntry[],
    updatePreferences: vi.fn(),
    refreshSnapshot: vi.fn(),
    refetchSnapshotIfStale: vi.fn(),
  },
}));

vi.mock("@/runtime/host-runtime", () => ({
  useHosts: () => [{ serverId: "host-1" }],
}));

vi.mock("./use-form-preferences", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./use-form-preferences")>();
  return {
    ...actual,
    useFormPreferences: () => ({
      preferences: mocks.preferences,
      isLoading: mocks.isPreferencesLoading,
      updatePreferences: mocks.updatePreferences,
    }),
  };
});

vi.mock("./use-providers-snapshot", () => ({
  useProvidersSnapshot: () => ({
    entries: mocks.snapshotEntries,
    isLoading: false,
    isFetching: false,
    isRefreshing: false,
    error: null,
    supportsSnapshot: true,
    refresh: mocks.refreshSnapshot,
    refetchIfStale: mocks.refetchSnapshotIfStale,
  }),
}));

function codexReadyEntry(): ProviderSnapshotEntry {
  return {
    provider: "codex",
    status: "ready",
    label: "Codex",
    description: "Codex test provider",
    defaultModeId: "auto",
    modes: [
      { id: "auto", label: "Auto", icon: "ShieldAlert", colorTier: "moderate" },
      {
        id: "full-access",
        label: "Full Access",
        icon: "ShieldAlert",
        colorTier: "dangerous",
      },
    ],
    models: [
      {
        provider: "codex",
        id: "gpt-5.4",
        label: "gpt-5.4",
        isDefault: true,
        defaultThinkingOptionId: "low",
        thinkingOptions: [
          { id: "low", label: "Low" },
          { id: "xhigh", label: "XHigh" },
        ],
      },
    ],
  };
}

function claudeLoadingEntry(): ProviderSnapshotEntry {
  return {
    provider: "claude",
    status: "loading",
    label: "Claude",
    description: "Claude test provider",
    defaultModeId: "default",
    modes: [{ id: "default", label: "Default", icon: "ShieldCheck", colorTier: "safe" }],
  };
}

function claudeReadyEntry(): ProviderSnapshotEntry {
  return {
    provider: "claude",
    status: "ready",
    label: "Claude",
    description: "Claude test provider",
    defaultModeId: "default",
    modes: [{ id: "default", label: "Default", icon: "ShieldCheck", colorTier: "safe" }],
    models: [
      {
        provider: "claude",
        id: "claude-opus-4-7[1m]",
        label: "Opus 4.7 1M",
      },
      {
        provider: "claude",
        id: "claude-sonnet-4-6",
        label: "Sonnet 4.6",
        isDefault: true,
      },
    ],
  };
}

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    value: true,
    configurable: true,
    writable: true,
  });
});

describe("useAgentFormState live preference hydration", () => {
  beforeEach(() => {
    const dom = new JSDOM("<!doctype html><html><body></body></html>", {
      url: "http://localhost",
    });

    Object.defineProperty(globalThis, "document", {
      value: dom.window.document,
      configurable: true,
    });
    Object.defineProperty(globalThis, "window", {
      value: dom.window,
      configurable: true,
    });
    Object.defineProperty(globalThis, "navigator", {
      value: dom.window.navigator,
      configurable: true,
    });

    mocks.preferences = {};
    mocks.isPreferencesLoading = false;
    mocks.snapshotEntries = [codexReadyEntry(), claudeReadyEntry()];
    mocks.updatePreferences.mockReset();
    mocks.refreshSnapshot.mockReset();
    mocks.refetchSnapshotIfStale.mockReset();
  });

  it("hydrates from stored preferences once and ignores later preference writes from other composers", async () => {
    mocks.preferences = {
      provider: "codex",
      providerPreferences: {
        codex: {
          model: "gpt-5.4",
          mode: "full-access",
          thinkingByModel: {
            "gpt-5.4": "xhigh",
          },
        },
      },
    };

    const { result, rerender } = renderHook(() =>
      useAgentFormState({
        initialServerId: "host-1",
        isVisible: true,
        onlineServerIds: ["host-1"],
      }),
    );

    await waitFor(() => {
      expect(result.current.selectedProvider).toBe("codex");
      expect(result.current.selectedModel).toBe("gpt-5.4");
      expect(result.current.selectedMode).toBe("full-access");
      expect(result.current.selectedThinkingOptionId).toBe("xhigh");
    });

    mocks.preferences = {
      provider: "claude",
      providerPreferences: {
        claude: {
          model: "claude-sonnet-4-6",
          mode: "default",
        },
        codex: {
          model: "gpt-5.4",
          mode: "auto",
          thinkingByModel: {
            "gpt-5.4": "low",
          },
        },
      },
    };
    rerender();

    await waitFor(() => {
      expect(result.current.selectedProvider).toBe("codex");
      expect(result.current.selectedModel).toBe("gpt-5.4");
      expect(result.current.selectedMode).toBe("full-access");
      expect(result.current.selectedThinkingOptionId).toBe("xhigh");
    });
  });

  it("uses the latest preferences when a separate composer hydrates later", async () => {
    mocks.preferences = {
      provider: "claude",
      providerPreferences: {
        claude: {
          model: "claude-sonnet-4-6",
          mode: "default",
        },
      },
    };

    const { result } = renderHook(() =>
      useAgentFormState({
        initialServerId: "host-1",
        isVisible: true,
        onlineServerIds: ["host-1"],
      }),
    );

    await waitFor(() => {
      expect(result.current.selectedProvider).toBe("claude" as AgentProvider);
      expect(result.current.selectedModel).toBe("claude-sonnet-4-6");
      expect(result.current.selectedMode).toBe("default");
    });
  });

  it("selects Claude while loading and fills the first listed model when the snapshot becomes ready", async () => {
    mocks.snapshotEntries = [codexReadyEntry(), claudeLoadingEntry()];

    const { result, rerender } = renderHook(() =>
      useAgentFormState({
        initialServerId: "host-1",
        isVisible: true,
        onlineServerIds: ["host-1"],
      }),
    );

    await waitFor(() => {
      expect(result.current.selectedProvider).toBe("claude" as AgentProvider);
      expect(result.current.selectedModel).toBe("");
    });
    expect(mocks.refetchSnapshotIfStale).toHaveBeenCalledWith("claude");

    mocks.snapshotEntries = [codexReadyEntry(), claudeReadyEntry()];
    rerender();

    await waitFor(() => {
      expect(result.current.selectedProvider).toBe("claude" as AgentProvider);
      expect(result.current.selectedModel).toBe("claude-opus-4-7[1m]");
    });
  });

  it("actively ensures the selected provider for visible create-flow composers", async () => {
    renderHook(() =>
      useAgentFormState({
        initialServerId: "host-1",
        initialValues: { workingDir: "/opened-project" },
        isVisible: true,
        onlineServerIds: ["host-1"],
      }),
    );

    await waitFor(() => {
      expect(mocks.refetchSnapshotIfStale).toHaveBeenCalledWith("claude");
    });
  });

  it("does not actively ensure hidden or non-create-flow composers", async () => {
    const { rerender } = renderHook(
      ({ isVisible, isCreateFlow }) =>
        useAgentFormState({
          initialServerId: "host-1",
          isVisible,
          isCreateFlow,
          onlineServerIds: ["host-1"],
        }),
      {
        initialProps: { isVisible: false, isCreateFlow: true },
      },
    );

    rerender({ isVisible: true, isCreateFlow: false });

    await waitFor(() => {
      expect(mocks.refetchSnapshotIfStale).not.toHaveBeenCalled();
    });
  });
});

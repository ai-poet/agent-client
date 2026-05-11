import { act, renderHook, waitFor } from "@testing-library/react";
import { JSDOM } from "jsdom";
import { describe, expect, it, vi } from "vitest";

vi.mock("expo-router", () => ({
  useRouter: () => ({
    push: vi.fn(),
  }),
}));

const { modelCliMocks } = vi.hoisted(() => ({
  modelCliMocks: {
    getModelCliRuntimeStatus: vi.fn(),
    installGitBashRuntime: vi.fn(),
    installNode22Runtime: vi.fn(),
    installCodexCli: vi.fn(),
    installClaudeCodeCli: vi.fn(),
  },
}));

vi.mock("@/desktop/daemon/desktop-daemon", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/desktop/daemon/desktop-daemon")>();
  return {
    ...actual,
    getModelCliRuntimeStatus: modelCliMocks.getModelCliRuntimeStatus,
    installGitBashRuntime: modelCliMocks.installGitBashRuntime,
    installNode22Runtime: modelCliMocks.installNode22Runtime,
    installCodexCli: modelCliMocks.installCodexCli,
    installClaudeCodeCli: modelCliMocks.installClaudeCodeCli,
  };
});

vi.mock("@/desktop/electron/invoke", () => ({
  invokeDesktopCommand: vi.fn(async () => ({
    providers: [
      {
        id: "claude-key",
        name: "Claude",
        type: "custom",
        endpoint: "https://example.com",
        apiKey: "claude",
        isDefault: false,
        target: "claude",
      },
      {
        id: "codex-key",
        name: "Codex",
        type: "custom",
        endpoint: "https://example.com",
        apiKey: "codex",
        isDefault: false,
        target: "codex",
      },
    ],
    activeProviderId: null,
    activeClaudeProviderId: "claude-key",
    activeCodexProviderId: "codex-key",
  })),
}));

vi.mock("@/hooks/use-settings", () => ({
  useAppSettings: () => ({ settings: { accessMode: "byok" } }),
}));

vi.mock("@/hooks/use-sub2api-auth", () => ({
  useSub2APIAuth: () => ({
    auth: null,
    getAccessToken: vi.fn(),
  }),
}));

import {
  describeManagedCloudAvailability,
  formatCliInstallFailureMessage,
  getMissingCliDependencyNames,
  summarizeManagedCloudAvailability,
  useSetupChecks,
} from "@/hooks/use-setup-checks";
import type { Sub2APIGroup, Sub2APIKey } from "@/lib/sub2api-client";
import type { ModelCliRuntimeStatus } from "@/desktop/daemon/desktop-daemon";

function makeGroup(id: number, platform: Sub2APIGroup["platform"]): Sub2APIGroup {
  return {
    id,
    name: `${platform}-${id}`,
    description: "",
    platform,
    rate_multiplier: 1,
    status: "active",
    subscription_type: "standard",
    allow_messages_dispatch: false,
  };
}

function makeKey(
  id: number,
  group: Sub2APIGroup | null,
  status: Sub2APIKey["status"] = "active",
): Sub2APIKey {
  return {
    id,
    user_id: 1,
    key: `sk-${id}`,
    name: `key-${id}`,
    group_id: group?.id ?? null,
    status,
    quota: 0,
    quota_used: 0,
    rate_limit_5h: 0,
    rate_limit_1d: 0,
    rate_limit_7d: 0,
    usage_5h: 0,
    usage_1d: 0,
    usage_7d: 0,
    created_at: "",
    updated_at: "",
    group,
  };
}

function makeRuntimeStatus(overrides?: Partial<ModelCliRuntimeStatus>): ModelCliRuntimeStatus {
  return {
    git: {
      installed: true,
      version: "2.54.0",
      bashPath: "C:\\Users\\alice\\.paseo\\toolchains\\PortableGit\\bin\\bash.exe",
      error: null,
    },
    node: {
      installed: true,
      version: "22.20.0",
      major: 22,
      npmVersion: "10.9.0",
      satisfies: true,
      manager: "shell",
      error: null,
    },
    claude: {
      command: "claude",
      packageName: "@anthropic-ai/claude-code",
      installed: true,
      version: "2.1.138",
      error: null,
    },
    codex: {
      command: "codex",
      packageName: "@openai/codex",
      installed: true,
      version: "0.130.0",
      error: null,
    },
    ...overrides,
  };
}

describe("use-setup-checks availability helpers", () => {
  it("counts route-ready groups and active keys for Claude Code and Codex", () => {
    const claudeGroup = makeGroup(1, "anthropic");
    const codexGroup = makeGroup(2, "openai");
    const summary = summarizeManagedCloudAvailability(
      [makeKey(1, claudeGroup), makeKey(2, codexGroup)],
      [claudeGroup, codexGroup],
    );

    expect(summary).toEqual({
      claude: { groups: 1, activeKeys: 1 },
      codex: { groups: 1, activeKeys: 1 },
    });
  });

  it("treats a single supported route as usable but partial", () => {
    const claudeGroup = makeGroup(1, "anthropic");
    const summary = summarizeManagedCloudAvailability([makeKey(1, claudeGroup)], [claudeGroup]);

    expect(describeManagedCloudAvailability(summary, 1, 1)).toMatchObject({
      status: "passed",
    });
    expect(describeManagedCloudAvailability(summary, 1, 1).description).toContain("Claude Code");
    expect(describeManagedCloudAvailability(summary, 1, 1).description).toContain("Codex");
  });

  it("fails with a helpful message when keys exist but none are compatible", () => {
    const geminiGroup = makeGroup(3, "gemini");
    const summary = summarizeManagedCloudAvailability([makeKey(1, geminiGroup)], [geminiGroup]);

    expect(describeManagedCloudAvailability(summary, 1, 1)).toEqual({
      status: "failed",
      description: "Your current API keys are not bound to Claude Code or Codex compatible routes",
      error: "Assign an anthropic or openai group in Paseo Cloud before continuing.",
      fixLabel: "Manage Routes",
    });
  });

  it("lists Git Bash separately when the Windows bootstrap stack is incomplete", () => {
    const status: ModelCliRuntimeStatus = {
      git: {
        installed: false,
        version: null,
        bashPath: null,
        error: "Git Bash was not found.",
      },
      node: {
        installed: false,
        version: null,
        major: null,
        npmVersion: null,
        satisfies: false,
        manager: "shell",
        error: "Node.js was not found.",
      },
      claude: {
        command: "claude",
        packageName: "@anthropic-ai/claude-code",
        installed: false,
        version: null,
        error: "Claude Code was not found.",
      },
      codex: {
        command: "codex",
        packageName: "@openai/codex",
        installed: false,
        version: null,
        error: "Codex was not found.",
      },
    };

    expect(getMissingCliDependencyNames(status)).toEqual([
      "Git Bash",
      "Node.js 22",
      "Claude Code",
      "Codex",
    ]);
  });

  it("formats install failures with missing tools and a concise error", () => {
    const status: ModelCliRuntimeStatus = {
      git: {
        installed: true,
        version: "2.54.0",
        bashPath: "C:/Program Files/Git/bin/bash.exe",
        error: null,
      },
      node: {
        installed: true,
        version: "22.20.0",
        major: 22,
        npmVersion: "10.9.0",
        satisfies: true,
        manager: "shell",
        error: null,
      },
      claude: {
        command: "claude",
        packageName: "@anthropic-ai/claude-code",
        installed: false,
        version: null,
        error: "Claude Code was not found.",
      },
      codex: {
        command: "codex",
        packageName: "@openai/codex",
        installed: true,
        version: "0.130.0",
        error: null,
      },
    };

    expect(
      formatCliInstallFailureMessage(
        new Error(
          "Error invoking remote method 'paseo:invoke': Error: Install failed: npmmirror npm registry timed out while installing Claude Code.",
        ),
        status,
      ),
    ).toBe(
      "Install failed: npmmirror npm registry timed out while installing Claude Code. Missing: Claude Code",
    );
  });

  it("does not append stale missing tools when the desktop error already includes them", () => {
    const staleStatus: ModelCliRuntimeStatus = {
      git: {
        installed: false,
        version: null,
        bashPath: null,
        error: "Git Bash was not found.",
      },
      node: {
        installed: false,
        version: null,
        major: null,
        npmVersion: null,
        satisfies: false,
        manager: "shell",
        error: "Node.js was not found.",
      },
      claude: {
        command: "claude",
        packageName: "@anthropic-ai/claude-code",
        installed: false,
        version: null,
        error: "Claude Code was not found.",
      },
      codex: {
        command: "codex",
        packageName: "@openai/codex",
        installed: false,
        version: null,
        error: "Codex was not found.",
      },
    };

    expect(
      formatCliInstallFailureMessage(
        new Error("Install failed: npm official registry timed out. Missing: Claude Code"),
        staleStatus,
      ),
    ).toBe("Install failed: npm official registry timed out. Missing: Claude Code");
  });

  it("runs CLI installation as visible ordered steps", async () => {
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
    Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
      value: true,
      configurable: true,
      writable: true,
    });
    Object.defineProperty(dom.window, "paseoDesktop", {
      value: {
        invoke: vi.fn(),
      },
      configurable: true,
    });

    const missingStatus = makeRuntimeStatus({
      git: { installed: false, version: null, bashPath: null, error: "Git Bash missing" },
      node: {
        installed: false,
        version: null,
        major: null,
        npmVersion: null,
        satisfies: false,
        manager: "shell",
        error: "Node missing",
      },
      claude: {
        command: "claude",
        packageName: "@anthropic-ai/claude-code",
        installed: false,
        version: null,
        error: "Claude missing",
      },
      codex: {
        command: "codex",
        packageName: "@openai/codex",
        installed: false,
        version: null,
        error: "Codex missing",
      },
    });
    const readyStatus = makeRuntimeStatus();
    const descriptions: string[] = [];
    modelCliMocks.installGitBashRuntime.mockResolvedValue({ status: readyStatus, output: "" });
    modelCliMocks.installNode22Runtime.mockResolvedValue({ status: readyStatus, output: "" });
    modelCliMocks.installCodexCli.mockResolvedValue({ status: readyStatus, output: "" });
    modelCliMocks.installClaudeCodeCli.mockResolvedValue({ status: readyStatus, output: "" });
    modelCliMocks.getModelCliRuntimeStatus.mockResolvedValue(missingStatus);

    const { result } = renderHook(() => useSetupChecks());

    await act(async () => {
      try {
        await result.current.runAllChecks();
      } catch {
        // runAllChecks swallows CLI install readiness failures internally.
      }
    });
    await waitFor(() => {
      expect(result.current.checks.find((check) => check.id === "cliConfig")?.status).toBe(
        "failed",
      );
    });

    await act(async () => {
      const promise = result.current.fixCheck("cliConfig");
      await waitFor(() => {
        expect(modelCliMocks.installGitBashRuntime).toHaveBeenCalled();
      });
      descriptions.push(result.current.checks.find((check) => check.id === "cliConfig")?.description ?? "");
      await waitFor(() => {
        expect(modelCliMocks.installNode22Runtime).toHaveBeenCalled();
      });
      descriptions.push(result.current.checks.find((check) => check.id === "cliConfig")?.description ?? "");
      await waitFor(() => {
        expect(modelCliMocks.installCodexCli).toHaveBeenCalled();
      });
      descriptions.push(result.current.checks.find((check) => check.id === "cliConfig")?.description ?? "");
      await waitFor(() => {
        expect(modelCliMocks.installClaudeCodeCli).toHaveBeenCalled();
      });
      descriptions.push(result.current.checks.find((check) => check.id === "cliConfig")?.description ?? "");
      await promise;
    });

    expect(descriptions.join("\n")).toContain("Git Bash");
    expect(descriptions.join("\n")).toContain("Node.js 22");
    expect(descriptions.join("\n")).toContain("Codex");
    expect(descriptions.join("\n")).toContain("Claude Code");
    expect(modelCliMocks.installGitBashRuntime).toHaveBeenCalledBefore(
      modelCliMocks.installNode22Runtime,
    );
    expect(modelCliMocks.installNode22Runtime).toHaveBeenCalledBefore(
      modelCliMocks.installCodexCli,
    );
    expect(modelCliMocks.installCodexCli).toHaveBeenCalledBefore(
      modelCliMocks.installClaudeCodeCli,
    );
  });
});

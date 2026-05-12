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
  getCliInstallSteps,
  getMissingCliDependencyNames,
  summarizeManagedCloudAvailability,
} from "@/hooks/use-setup-checks";
import { getSub2APIMessages } from "@/i18n/sub2api";
import type { Sub2APIGroup, Sub2APIKey } from "@/lib/sub2api-client";
import type { ModelCliRuntimeStatus } from "@/desktop/daemon/desktop-daemon";

vi.mock("react-native", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react-native")>();
  return {
    ...actual,
    Platform: {
      ...actual.Platform,
      OS: "web",
    },
  };
});

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
        getSub2APIMessages("en").setupCheck,
      ),
    ).toBe(
      "Install failed: npmmirror npm registry timed out while installing Claude Code. Missing: Claude Code",
    );
  });

  it("formats install failures with localized missing tool prefix", () => {
    const status = makeRuntimeStatus({
      node: {
        installed: false,
        version: null,
        major: null,
        npmVersion: null,
        satisfies: false,
        manager: "shell",
        error: "Node.js was not found.",
      },
    });

    expect(
      formatCliInstallFailureMessage(
        new Error("Automatic Node.js 22 installation failed."),
        status,
        getSub2APIMessages("zh").setupCheck,
      ),
    ).toBe("安装失败。请重试或手动安装。缺少：Node.js 22");
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
        getSub2APIMessages("en").setupCheck,
      ),
    ).toBe("Install failed: npm official registry timed out. Missing: Claude Code");
  });

  it("describes CLI installation as ordered visible steps", () => {
    expect(
      getCliInstallSteps(getSub2APIMessages("en").setupCheck).map((step) => ({
        id: step.id,
        label: step.label,
        installingDescription: step.installingDescription,
      })),
    ).toEqual([
      {
        id: "git",
        label: "Git Bash",
        installingDescription: "Installing Git Bash...",
      },
      {
        id: "node",
        label: "Node.js 22",
        installingDescription: "Installing Node.js 22...",
      },
      {
        id: "codex",
        label: "Codex",
        installingDescription: "Installing Codex CLI...",
      },
      {
        id: "claude",
        label: "Claude Code",
        installingDescription: "Installing Claude Code CLI...",
      },
    ]);
  });

  it("describes CLI installation steps in Chinese", () => {
    expect(
      getCliInstallSteps(getSub2APIMessages("zh").setupCheck).map((step) => ({
        id: step.id,
        label: step.label,
        installingDescription: step.installingDescription,
      })),
    ).toEqual([
      {
        id: "git",
        label: "Git Bash",
        installingDescription: "正在安装 Git Bash...",
      },
      {
        id: "node",
        label: "Node.js 22",
        installingDescription: "正在安装 Node.js 22...",
      },
      {
        id: "codex",
        label: "Codex",
        installingDescription: "正在安装 Codex CLI...",
      },
      {
        id: "claude",
        label: "Claude Code",
        installingDescription: "正在安装 Claude Code CLI...",
      },
    ]);
  });
});

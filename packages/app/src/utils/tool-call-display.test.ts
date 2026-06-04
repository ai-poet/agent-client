import { describe, expect, it } from "vitest";

import { buildToolCallDisplayModel } from "./tool-call-display";
import type { ToolCallDisplayInput } from "./tool-call-display";

describe("tool-call-display", () => {
  it("builds display model from canonical shell detail", () => {
    const display = buildToolCallDisplayModel({
      name: "shell",
      status: "running",
      error: null,
      detail: {
        type: "shell",
        command: "npm test",
      },
    });

    expect(display).toEqual({
      displayName: "Shell",
      summary: "npm test",
    });
  });

  it("builds display model from canonical read detail", () => {
    const display = buildToolCallDisplayModel({
      name: "read_file",
      status: "completed",
      error: null,
      detail: {
        type: "read",
        filePath: "/tmp/repo/src/index.ts",
      },
      cwd: "/tmp/repo",
    });

    expect(display).toEqual({
      displayName: "Read",
      summary: "src/index.ts",
    });
  });

  it("uses sub-agent detail for task label and description", () => {
    const display = buildToolCallDisplayModel({
      name: "task",
      status: "running",
      error: null,
      detail: {
        type: "sub_agent",
        subAgentType: "Explore",
        description: "Inspect repository structure",
        log: "[Read] README.md",
        actions: [
          {
            index: 1,
            toolName: "Read",
            summary: "README.md",
          },
        ],
      },
    });

    expect(display).toEqual({
      displayName: "Explore",
      summary: "Inspect repository structure",
    });
  });

  it("falls back to humanized tool name for unknown tools", () => {
    const display = buildToolCallDisplayModel({
      name: "custom_tool_name",
      status: "completed",
      error: null,
      detail: {
        type: "unknown",
        input: null,
        output: null,
      },
    });

    expect(display).toEqual({
      displayName: "Custom Tool Name",
    });
  });

  it("builds display model from worktree setup detail", () => {
    const display = buildToolCallDisplayModel({
      name: "paseo_worktree_setup",
      status: "running",
      error: null,
      detail: {
        type: "worktree_setup",
        worktreePath: "/tmp/repo/.paseo/worktrees/repo/branch",
        branchName: "feature-branch",
        log: "==> [1/1] Running: npm install\n",
        commands: [
          {
            index: 1,
            command: "npm install",
            cwd: "/tmp/repo/.paseo/worktrees/repo/branch",
            log: "",
            status: "running",
            exitCode: null,
          },
        ],
      },
    });

    expect(display).toEqual({
      displayName: "Worktree Setup",
      summary: "feature-branch",
    });
  });

  it("does not derive command summary from unknown raw detail", () => {
    const display = buildToolCallDisplayModel({
      name: "exec_command",
      status: "running",
      error: null,
      detail: {
        type: "unknown",
        input: { command: "npm run test" },
        output: null,
      },
    });

    expect(display).toEqual({
      displayName: "Exec Command",
    });
  });

  it("returns formatted errorText from the same display pipeline", () => {
    const display = buildToolCallDisplayModel({
      name: "shell",
      status: "failed",
      error: { message: "boom" },
      detail: {
        type: "unknown",
        input: { command: "false" },
        output: null,
      },
    });

    expect(display.errorText).toBe('{\n  "message": "boom"\n}');
  });

  it("shows terminal interaction with only the fixed label when no command is available", () => {
    const display = buildToolCallDisplayModel({
      name: "terminal",
      status: "completed",
      error: null,
      detail: {
        type: "plain_text",
        icon: "square_terminal",
      },
    });

    expect(display).toEqual({
      displayName: "Terminal",
    });
  });

  it("shows terminal interaction command as the summary when available", () => {
    const display = buildToolCallDisplayModel({
      name: "terminal",
      status: "completed",
      error: null,
      detail: {
        type: "plain_text",
        label: "npm run test",
        icon: "square_terminal",
      },
    });

    expect(display).toEqual({
      displayName: "Terminal",
      summary: "npm run test",
    });
  });

  it("localizes canonical tool labels when locale is provided", () => {
    const cases: Array<{
      input: ToolCallDisplayInput;
      en: string;
      zh: string;
    }> = [
      {
        input: {
          name: "shell",
          status: "running",
          error: null,
          detail: { type: "shell", command: "npm test" },
        },
        en: "Shell",
        zh: "终端",
      },
      {
        input: {
          name: "read_file",
          status: "completed",
          error: null,
          detail: { type: "read", filePath: "/tmp/repo/README.md" },
          cwd: "/tmp/repo",
        },
        en: "Read",
        zh: "读取",
      },
      {
        input: {
          name: "apply_patch",
          status: "completed",
          error: null,
          detail: { type: "edit", filePath: "/tmp/repo/src/index.ts" },
          cwd: "/tmp/repo",
        },
        en: "Edit",
        zh: "编辑",
      },
      {
        input: {
          name: "write_file",
          status: "completed",
          error: null,
          detail: { type: "write", filePath: "/tmp/repo/src/index.ts" },
          cwd: "/tmp/repo",
        },
        en: "Write",
        zh: "写入",
      },
      {
        input: {
          name: "grep",
          status: "completed",
          error: null,
          detail: { type: "search", query: "TODO" },
        },
        en: "Search",
        zh: "搜索",
      },
      {
        input: {
          name: "web_fetch",
          status: "completed",
          error: null,
          detail: { type: "fetch", url: "https://example.com" },
        },
        en: "Fetch",
        zh: "获取",
      },
      {
        input: {
          name: "paseo_worktree_setup",
          status: "running",
          error: null,
          detail: {
            type: "worktree_setup",
            worktreePath: "/tmp/repo/.paseo/worktrees/repo/branch",
            branchName: "feature-branch",
            log: "",
            commands: [],
          },
        },
        en: "Worktree Setup",
        zh: "工作区准备",
      },
      {
        input: {
          name: "task",
          status: "running",
          error: null,
          detail: {
            type: "sub_agent",
            description: "Inspect repository structure",
            log: "",
            actions: [],
          },
        },
        en: "Task",
        zh: "任务",
      },
      {
        input: {
          name: "thinking",
          status: "running",
          error: null,
          detail: { type: "unknown", input: "Thinking...", output: null },
        },
        en: "Thinking",
        zh: "思考中",
      },
      {
        input: {
          name: "terminal",
          status: "completed",
          error: null,
          detail: { type: "plain_text", icon: "square_terminal" },
        },
        en: "Terminal",
        zh: "终端",
      },
      {
        input: {
          name: "plan",
          status: "completed",
          error: null,
          detail: { type: "plan", text: "Build it" },
        },
        en: "Plan",
        zh: "计划",
      },
    ];

    for (const { input, en, zh } of cases) {
      expect(buildToolCallDisplayModel(input, { locale: "en" }).displayName).toBe(en);
      expect(buildToolCallDisplayModel(input, { locale: "zh" }).displayName).toBe(zh);
    }
  });

  it("keeps summaries and custom tool names unchanged while localizing labels", () => {
    const readDisplay = buildToolCallDisplayModel(
      {
        name: "read_file",
        status: "completed",
        error: null,
        detail: { type: "read", filePath: "/tmp/repo/src/index.ts" },
        cwd: "/tmp/repo",
      },
      { locale: "zh" },
    );
    expect(readDisplay).toEqual({
      displayName: "读取",
      summary: "src/index.ts",
    });

    const customDisplay = buildToolCallDisplayModel(
      {
        name: "custom_tool_name",
        status: "completed",
        error: null,
        detail: { type: "unknown", input: null, output: null },
      },
      { locale: "zh" },
    );
    expect(customDisplay.displayName).toBe("Custom Tool Name");
  });
});

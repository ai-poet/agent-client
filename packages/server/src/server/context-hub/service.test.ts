import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ContextHubService } from "./service.js";

describe("ContextHubService", () => {
  let paseoHome: string;
  let service: ContextHubService;

  beforeEach(async () => {
    paseoHome = await mkdtemp(path.join(tmpdir(), "paseo-context-hub-"));
    service = new ContextHubService({ paseoHome });
  });

  afterEach(async () => {
    await rm(paseoHome, { recursive: true, force: true });
  });

  it("classifies, redacts, searches, and dedupes project memories", async () => {
    const item = await service.createMemory({
      workspaceId: "workspace-1",
      rawText:
        "Architecture decision: use JSON files for v1 memory. Secret sk-test-secret-token-1234567890 should not persist.",
      tags: ["ADR", "Memory"],
    });

    expect(item.kind).toBe("architecture_decision");
    expect(item.cleanText).toContain("[REDACTED_OPENAI_KEY]");
    expect(item.fingerprint).toHaveLength(64);
    expect(item.tags).toEqual(["adr", "memory"]);

    const duplicate = await service.createMemory({
      workspaceId: "workspace-1",
      rawText:
        "Architecture decision: use JSON files for v1 memory. Secret sk-test-secret-token-1234567890 should not persist.",
    });
    expect(duplicate.id).toBe(item.id);

    const listed = await service.listMemory({
      workspaceId: "workspace-1",
      query: "json files",
    });
    expect(listed.total).toBe(1);
    expect(listed.items[0].id).toBe(item.id);

    const context = await service.renderProjectMemoryContext({
      workspaceId: "workspace-1",
      memoryIds: [item.id],
    });
    expect(context).toContain("<project-memory>");
    expect(context).toContain("architecture_decision");
  });

  it("renders managed prompts and records usage", async () => {
    const prompt = await service.createPrompt({
      name: "Fix file",
      content: "Fix {{file}} with $ARGUMENTS; first=$1",
      tags: ["fix"],
    });

    const rendered = await service.renderPrompt({
      promptId: prompt.id,
      variables: { file: "src/app.ts" },
      argumentsText: "bug tests",
      recordUsage: true,
    });

    expect(rendered.text).toBe("Fix src/app.ts with bug tests; first=bug");

    const prompts = await service.listPrompts();
    const updated = prompts.find((entry) => entry.id === prompt.id);
    expect(updated?.usageCount).toBe(1);
    expect(updated?.lastUsedAt).toEqual(expect.any(String));
  });

  it("imports and exports managed skills", async () => {
    const skill = await service.importSkill({
      name: "Review helper",
      description: "Review code with the local conventions.",
      content: "Use rg before reading broad file trees.",
    });

    expect(skill.readOnly).toBe(false);
    expect(skill.description).toBe("Review code with the local conventions.");

    const exported = await service.exportSkill({ skillId: skill.id });
    expect(exported.content).toContain("Use rg before reading broad file trees.");
  });

  it("validates and resolves MCP server profiles", async () => {
    const valid = await service.testMcpProfile({
      name: "Docs",
      server: {
        type: "http",
        url: "https://example.com/mcp",
      },
    });
    expect(valid.ok).toBe(true);

    const invalid = await service.testMcpProfile({
      name: "Bad",
      server: {
        type: "http",
        url: "ftp://example.com/mcp",
      },
    });
    expect(invalid.ok).toBe(false);

    const profile = await service.upsertMcpProfile({
      name: "local-tools",
      server: {
        type: "stdio",
        command: "node",
        args: ["server.js"],
      },
    });
    const resolved = await service.resolveMcpServers({ ids: [profile.id] });
    expect(resolved["local-tools"]).toEqual({
      type: "stdio",
      command: "node",
      args: ["server.js"],
    });
  });
});

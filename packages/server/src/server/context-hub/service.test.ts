import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import AdmZip from "adm-zip";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ContextHubService } from "./service.js";

function createSkillPackage(files: Record<string, string>): Buffer {
  const zip = new AdmZip();
  for (const [filePath, content] of Object.entries(files)) {
    zip.addFile(filePath, Buffer.from(content, "utf8"));
  }
  return zip.toBuffer();
}

function createUnsafeSkillPackage(): Buffer {
  const zip = new AdmZip();
  zip.addFile("SKILL.md", Buffer.from("# Bad\n", "utf8"));
  zip.addFile("safe.txt", Buffer.from("nope", "utf8"));
  const entry = zip.getEntry("safe.txt");
  if (!entry) {
    throw new Error("Failed to create unsafe zip entry");
  }
  entry.entryName = "../escape.txt";
  return zip.toBuffer();
}

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

  it("lists bundled skills as read-only local skills", async () => {
    const bundledRoot = await mkdtemp(path.join(tmpdir(), "paseo-bundled-skills-"));
    try {
      const bundledService = new ContextHubService({
        paseoHome,
        bundledSkillsRoots: [bundledRoot],
      });
      const skillDir = path.join(bundledRoot, "paseo-chat");
      await mkdir(skillDir, { recursive: true });
      await writeFile(
        path.join(skillDir, "SKILL.md"),
        "---\nname: paseo-chat\ndescription: Use chat rooms through the Paseo CLI.\n---\n",
        "utf8",
      );

      const skills = await bundledService.listSkills();
      expect(skills).toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            name: "paseo-chat",
            description: "Use chat rooms through the Paseo CLI.",
            source: "bundled",
            readOnly: true,
            scope: "global",
          }),
        ]),
      );
    } finally {
      await rm(bundledRoot, { recursive: true, force: true });
    }
  });

  it("installs marketplace skill packages into workspace provider skill directories", async () => {
    const workspaceRoot = await mkdtemp(path.join(tmpdir(), "paseo-skill-workspace-"));
    try {
      const result = await service.installMarketplaceSkillPackage({
        workspaceId: "workspace-1",
        cwd: workspaceRoot,
        skillId: "skill-1",
        name: "Google Search",
        packageBuffer: createSkillPackage({
          "SKILL.md": "---\nname: google-search\ndescription: Search Google.\n---\n",
          "main.py": "print('ok')\n",
        }),
      });

      expect(result.installed).toBe(true);
      expect(result.skill.scope).toBe("workspace");
      expect(result.skill.name).toBe("google-search");
      await expect(
        readFile(
          path.join(workspaceRoot, ".agents", "skills", "google-search", "SKILL.md"),
          "utf8",
        ),
      ).resolves.toContain("Search Google");
      await expect(
        readFile(path.join(workspaceRoot, ".codex", "skills", "google-search", "SKILL.md"), "utf8"),
      ).resolves.toContain("Search Google");
      await expect(
        readFile(
          path.join(workspaceRoot, ".claude", "skills", "google-search", "SKILL.md"),
          "utf8",
        ),
      ).resolves.toContain("Search Google");
    } finally {
      await rm(workspaceRoot, { recursive: true, force: true });
    }
  });

  it("treats repeated marketplace installs with identical content as idempotent", async () => {
    const workspaceRoot = await mkdtemp(path.join(tmpdir(), "paseo-skill-workspace-"));
    const packageBuffer = createSkillPackage({
      "SKILL.md": "---\nname: stable-skill\ndescription: Stable skill.\n---\n",
    });
    try {
      await service.installMarketplaceSkillPackage({
        workspaceId: "workspace-1",
        cwd: workspaceRoot,
        skillId: "skill-1",
        name: "stable-skill",
        packageBuffer,
      });
      const second = await service.installMarketplaceSkillPackage({
        workspaceId: "workspace-1",
        cwd: workspaceRoot,
        skillId: "skill-1",
        name: "stable-skill",
        packageBuffer,
      });

      expect(second.installed).toBe(false);
      expect(second.conflict).toBe(false);
    } finally {
      await rm(workspaceRoot, { recursive: true, force: true });
    }
  });

  it("rejects marketplace skill conflicts unless overwrite is explicit", async () => {
    const workspaceRoot = await mkdtemp(path.join(tmpdir(), "paseo-skill-workspace-"));
    try {
      await service.installMarketplaceSkillPackage({
        workspaceId: "workspace-1",
        cwd: workspaceRoot,
        skillId: "skill-1",
        name: "review-helper",
        packageBuffer: createSkillPackage({
          "SKILL.md": "---\nname: review-helper\ndescription: First.\n---\n",
        }),
      });

      await expect(
        service.installMarketplaceSkillPackage({
          workspaceId: "workspace-1",
          cwd: workspaceRoot,
          skillId: "skill-1",
          name: "review-helper",
          packageBuffer: createSkillPackage({
            "SKILL.md": "---\nname: review-helper\ndescription: Second.\n---\n",
          }),
        }),
      ).rejects.toThrow(/already exists with different content/);

      const overwritten = await service.installMarketplaceSkillPackage({
        workspaceId: "workspace-1",
        cwd: workspaceRoot,
        skillId: "skill-1",
        name: "review-helper",
        overwrite: true,
        packageBuffer: createSkillPackage({
          "SKILL.md": "---\nname: review-helper\ndescription: Second.\n---\n",
        }),
      });
      expect(overwritten.installed).toBe(true);
      await expect(
        readFile(
          path.join(workspaceRoot, ".agents", "skills", "review-helper", "SKILL.md"),
          "utf8",
        ),
      ).resolves.toContain("Second");
    } finally {
      await rm(workspaceRoot, { recursive: true, force: true });
    }
  });

  it("rejects marketplace packages without top-level SKILL.md or with unsafe paths", async () => {
    const workspaceRoot = await mkdtemp(path.join(tmpdir(), "paseo-skill-workspace-"));
    try {
      await expect(
        service.installMarketplaceSkillPackage({
          workspaceId: "workspace-1",
          cwd: workspaceRoot,
          skillId: "skill-1",
          name: "bad-skill",
          packageBuffer: createSkillPackage({ "nested/SKILL.md": "# Bad\n" }),
        }),
      ).rejects.toThrow(/top-level SKILL\.md/);

      await expect(
        service.installMarketplaceSkillPackage({
          workspaceId: "workspace-1",
          cwd: workspaceRoot,
          skillId: "skill-1",
          name: "bad-skill",
          packageBuffer: createUnsafeSkillPackage(),
        }),
      ).rejects.toThrow(/Unsafe skill package path/);
    } finally {
      await rm(workspaceRoot, { recursive: true, force: true });
    }
  });

  it("adds workspace skill directories to git exclude when available", async () => {
    const workspaceRoot = await mkdtemp(path.join(tmpdir(), "paseo-skill-workspace-"));
    try {
      await writeFile(
        path.join(workspaceRoot, ".git"),
        "gitdir: ../git-dir/worktrees/demo\n",
        "utf8",
      );
      await service.installMarketplaceSkillPackage({
        workspaceId: "workspace-1",
        cwd: workspaceRoot,
        skillId: "skill-1",
        name: "git-skill",
        packageBuffer: createSkillPackage({
          "SKILL.md": "---\nname: git-skill\ndescription: Git skill.\n---\n",
        }),
      });

      const exclude = await readFile(
        path.resolve(workspaceRoot, "..", "git-dir", "worktrees", "demo", "info", "exclude"),
        "utf8",
      );
      expect(exclude).toContain(".agents/skills/");
      expect(exclude).toContain(".codex/skills/");
      expect(exclude).toContain(".claude/skills/");
    } finally {
      await rm(workspaceRoot, { recursive: true, force: true });
      await rm(path.resolve(workspaceRoot, "..", "git-dir"), { recursive: true, force: true });
    }
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

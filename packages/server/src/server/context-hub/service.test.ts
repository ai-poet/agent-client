import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import AdmZip from "adm-zip";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
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

function createJsonResponse(payload: unknown, init: ResponseInit = {}): Response {
  return new Response(JSON.stringify(payload), {
    status: 200,
    headers: { "content-type": "application/json" },
    ...init,
  });
}

function createSkillsMpPayload(skills: unknown[]): unknown {
  return {
    success: true,
    data: {
      skills,
    },
  };
}

function createSkillsMpSkill(overrides: Record<string, unknown> = {}): unknown {
  return {
    id: "openai-codex-codex-skills-code-review-skill-md",
    name: "Code Review",
    author: "openai",
    description: "Review code changes with repo context.",
    githubUrl: "https://github.com/openai/codex/tree/main/.codex/skills/code-review",
    skillUrl: "https://skillsmp.com/skills/openai-codex-codex-skills-code-review-skill-md",
    stars: 88313,
    updatedAt: "1776755044",
    ...overrides,
  };
}

describe("ContextHubService", () => {
  let paseoHome: string;
  let service: ContextHubService;
  let originalFetch: typeof globalThis.fetch;

  beforeEach(async () => {
    originalFetch = globalThis.fetch;
    paseoHome = await mkdtemp(path.join(tmpdir(), "paseo-context-hub-"));
    service = new ContextHubService({ paseoHome });
  });

  afterEach(async () => {
    globalThis.fetch = originalFetch;
    vi.restoreAllMocks();
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

  it("maps SkillsMP search results", async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = new URL(String(input));
      expect(url.origin).toBe("https://skillsmp.com");
      expect(url.pathname).toBe("/api/v1/skills/search");
      expect(url.searchParams.get("q")).toBe("code review");
      expect(url.searchParams.get("sortBy")).toBe("stars");
      return createJsonResponse(createSkillsMpPayload([createSkillsMpSkill()]));
    });
    globalThis.fetch = fetchMock as typeof fetch;

    const skills = await service.listMarketplaceSkills({
      query: "code review",
      limit: 10,
    });

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(skills).toHaveLength(1);
    expect(skills[0]).toMatchObject({
      id: "skillsmp:openai-codex-codex-skills-code-review-skill-md",
      name: "Code Review",
      description: "Review code changes with repo context.",
      trustLevel: "verified",
      vettingStatus: "trusted-source",
      platformCompatibility: ["Claude", "Codex"],
      downloadCount: 88313,
      downloads7d: 0,
      installed: false,
    });
  });

  it("returns an empty marketplace list when SkillsMP has no results", async () => {
    const fetchMock = vi.fn(async () => createJsonResponse(createSkillsMpPayload([])));
    globalThis.fetch = fetchMock as typeof fetch;

    const skills = await service.listMarketplaceSkills({ query: "review" });

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(skills).toEqual([]);
  });

  it("surfaces SkillsMP search failures without falling back", async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = new URL(String(input));
      expect(url.origin).toBe("https://skillsmp.com");
      return new Response("nope", { status: 503, statusText: "Service Unavailable" });
    });
    globalThis.fetch = fetchMock as typeof fetch;

    await expect(service.listMarketplaceSkills({ query: "fallback" })).rejects.toThrow(
      /SkillsMP search failed: 503 Service Unavailable/,
    );

    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("rejects non-SkillsMP marketplace installs", async () => {
    const workspaceRoot = await mkdtemp(path.join(tmpdir(), "paseo-skill-workspace-"));
    try {
      await expect(
        service.installMarketplaceSkill({
          workspaceId: "workspace-1",
          cwd: workspaceRoot,
          skillId: "legacy-review",
          name: "Legacy Review",
        }),
      ).rejects.toThrow(/Unsupported marketplace skill id: legacy-review/);
    } finally {
      await rm(workspaceRoot, { recursive: true, force: true });
    }
  });

  it("requires both Claude and Codex provider skill directories for installed state", async () => {
    const workspaceRoot = await mkdtemp(path.join(tmpdir(), "paseo-skill-workspace-"));
    const fetchMock = vi.fn(async () =>
      createJsonResponse(createSkillsMpPayload([createSkillsMpSkill()])),
    );
    globalThis.fetch = fetchMock as typeof fetch;
    try {
      await mkdir(path.join(workspaceRoot, ".agents", "skills", "code-review"), {
        recursive: true,
      });
      await writeFile(
        path.join(workspaceRoot, ".agents", "skills", "code-review", "SKILL.md"),
        "---\nname: code-review\ndescription: Agent cache only.\n---\n",
        "utf8",
      );

      const agentOnly = await service.listMarketplaceSkills({
        workspaceId: "workspace-1",
        cwd: workspaceRoot,
        query: "code review",
      });
      expect(agentOnly[0].installed).toBe(false);

      await mkdir(path.join(workspaceRoot, ".codex", "skills", "code-review"), {
        recursive: true,
      });
      await writeFile(
        path.join(workspaceRoot, ".codex", "skills", "code-review", "SKILL.md"),
        "---\nname: code-review\ndescription: Codex only.\n---\n",
        "utf8",
      );
      const codexOnly = await service.listMarketplaceSkills({
        workspaceId: "workspace-1",
        cwd: workspaceRoot,
        query: "code review",
      });
      expect(codexOnly[0].installed).toBe(false);

      await mkdir(path.join(workspaceRoot, ".claude", "skills", "code-review"), {
        recursive: true,
      });
      await writeFile(
        path.join(workspaceRoot, ".claude", "skills", "code-review", "SKILL.md"),
        "---\nname: code-review\ndescription: Claude and Codex.\n---\n",
        "utf8",
      );
      const providerReady = await service.listMarketplaceSkills({
        workspaceId: "workspace-1",
        cwd: workspaceRoot,
        query: "code review",
      });
      expect(providerReady[0].installed).toBe(true);
    } finally {
      await rm(workspaceRoot, { recursive: true, force: true });
    }
  });

  it("installs SkillsMP raw SKILL.md into agents, Codex, and Claude skill directories", async () => {
    const workspaceRoot = await mkdtemp(path.join(tmpdir(), "paseo-skill-workspace-"));
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = new URL(String(input));
      if (url.origin === "https://skillsmp.com") {
        return createJsonResponse(createSkillsMpPayload([createSkillsMpSkill()]));
      }
      expect(String(input)).toBe(
        "https://raw.githubusercontent.com/openai/codex/main/.codex/skills/code-review/SKILL.md",
      );
      return new Response("# Code Review\n\nUse the repository context.\n", { status: 200 });
    });
    globalThis.fetch = fetchMock as typeof fetch;
    try {
      const result = await service.installMarketplaceSkill({
        workspaceId: "workspace-1",
        cwd: workspaceRoot,
        skillId: "skillsmp:openai-codex-codex-skills-code-review-skill-md",
        name: "Code Review",
      });

      expect(result.installed).toBe(true);
      expect(result.skill.name).toBe("code-review");
      for (const root of [".agents/skills", ".codex/skills", ".claude/skills"] as const) {
        const content = await readFile(
          path.join(workspaceRoot, root, "code-review", "SKILL.md"),
          "utf8",
        );
        expect(content).toContain("name: code-review");
        expect(content).toContain("description: Review code changes with repo context.");
        expect(content).toContain("Use the repository context.");
      }
    } finally {
      await rm(workspaceRoot, { recursive: true, force: true });
    }
  });

  it("rejects SkillsMP installs without GitHub tree URLs or downloadable SKILL.md", async () => {
    const workspaceRoot = await mkdtemp(path.join(tmpdir(), "paseo-skill-workspace-"));
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = new URL(String(input));
      if (url.origin === "https://skillsmp.com") {
        return createJsonResponse(
          createSkillsMpPayload([
            createSkillsMpSkill({
              id: "bad-url",
              name: "Bad URL",
              githubUrl: "https://example.com/not-github",
            }),
            createSkillsMpSkill({
              id: "missing-raw",
              name: "Missing Raw",
              githubUrl: "https://github.com/openai/codex/tree/main/.codex/skills/missing",
            }),
          ]),
        );
      }
      return new Response("missing", { status: 404, statusText: "Not Found" });
    });
    globalThis.fetch = fetchMock as typeof fetch;
    try {
      await expect(
        service.installMarketplaceSkill({
          workspaceId: "workspace-1",
          cwd: workspaceRoot,
          skillId: "skillsmp:bad-url",
          name: "Bad URL",
        }),
      ).rejects.toThrow(/github\.com tree URL/);

      await expect(
        service.installMarketplaceSkill({
          workspaceId: "workspace-1",
          cwd: workspaceRoot,
          skillId: "skillsmp:missing-raw",
          name: "Missing Raw",
        }),
      ).rejects.toThrow(/SKILL\.md download failed: 404 Not Found/);
    } finally {
      await rm(workspaceRoot, { recursive: true, force: true });
    }
  });

  it("fails marketplace installs when provider skill sync cannot complete", async () => {
    const workspaceRoot = await mkdtemp(path.join(tmpdir(), "paseo-skill-workspace-"));
    try {
      await mkdir(path.join(workspaceRoot, ".codex"), { recursive: true });
      await writeFile(path.join(workspaceRoot, ".codex", "skills"), "blocked", "utf8");

      await expect(
        service.installMarketplaceSkillPackage({
          workspaceId: "workspace-1",
          cwd: workspaceRoot,
          skillId: "skill-1",
          name: "blocked-sync",
          packageBuffer: createSkillPackage({
            "SKILL.md": "---\nname: blocked-sync\ndescription: Blocked sync.\n---\n",
          }),
        }),
      ).rejects.toThrow();
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

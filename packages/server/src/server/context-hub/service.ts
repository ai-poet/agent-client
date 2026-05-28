import { createHash, randomUUID } from "node:crypto";
import { access, mkdir, readFile, readdir, rename, stat, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import path from "node:path";
import type pino from "pino";
import { z } from "zod";
import type { AgentProvider } from "../agent/agent-sdk-types.js";
import {
  ContextHubMcpServerConfigSchema,
  ManagedSkillEntrySchema,
  McpServerProfileSchema,
  ProjectMemoryItemSchema,
  PromptTemplateSchema,
  type ContextHubMcpServerConfig,
  type ManagedSkillEntry,
  type McpServerProfile,
  type McpServerProfileUpsertInput,
  type ProjectMemoryCreateInput,
  type ProjectMemoryItem,
  type ProjectMemoryKind,
  type ProjectMemoryUpdateInput,
  type PromptTemplate,
  type PromptTemplateCreateInput,
  type PromptTemplateUpdateInput,
} from "./rpc-schemas.js";

const MEMORY_CONTEXT_LIMIT = 8;
const MEMORY_SNIPPET_LIMIT = 900;
const PROMPT_STORE_SCHEMA = z.object({
  prompts: z.array(PromptTemplateSchema),
});
const MCP_STORE_SCHEMA = z.object({
  profiles: z.array(McpServerProfileSchema),
});

type Logger = Pick<pino.Logger, "debug" | "warn" | "error">;

type ContextHubServiceOptions = {
  paseoHome: string;
  logger?: Logger;
};

type ListMemoryOptions = {
  workspaceId: string;
  query?: string;
  kind?: ProjectMemoryKind;
  importance?: ProjectMemoryItem["importance"];
  tag?: string;
  includeDeleted?: boolean;
  page?: number;
  pageSize?: number;
};

type ListMemoryResult = {
  items: ProjectMemoryItem[];
  total: number;
};

type RenderMemoryContextOptions = {
  workspaceId: string;
  memoryIds?: string[];
  useWorkspaceMemory?: boolean;
};

type ListSkillsOptions = {
  workspaceId?: string;
  cwd?: string | null;
  includeContent?: boolean;
};

type ExportSkillOptions = {
  skillId: string;
  workspaceId?: string;
  cwd?: string | null;
};

type RenderPromptOptions = {
  promptId: string;
  variables?: Record<string, string>;
  argumentsText?: string;
  recordUsage?: boolean;
};

type RenderPromptResult = {
  text: string;
  prompt: PromptTemplate;
};

type ResolveMcpServersOptions = {
  ids?: string[];
  workspaceId?: string | null;
  provider?: AgentProvider | string | null;
};

const REDACTION_RULES: Array<[RegExp, string | ((match: string) => string)]> = [
  [
    /-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----/g,
    "[REDACTED_PRIVATE_KEY]",
  ],
  [/\bgithub_pat_[A-Za-z0-9_]{20,}\b/g, "[REDACTED_GITHUB_TOKEN]"],
  [/\bgh[pousr]_[A-Za-z0-9_]{20,}\b/g, "[REDACTED_GITHUB_TOKEN]"],
  [/\bsk-[A-Za-z0-9_-]{16,}\b/g, "[REDACTED_OPENAI_KEY]"],
  [/\bBearer\s+[A-Za-z0-9._~+/=-]{16,}\b/gi, "Bearer [REDACTED_TOKEN]"],
  [/\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b/g, "[REDACTED_JWT]"],
  [
    /\b([A-Za-z][A-Za-z0-9+.-]*:\/\/)([^@\s/:]+):([^@\s]+)@([^\s/]+)\b/g,
    "$1[REDACTED_USER]:[REDACTED_PASSWORD]@$4",
  ],
  [/\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b/gi, "[REDACTED_EMAIL]"],
];

const MEMORY_KIND_RULES: Array<{ kind: ProjectMemoryKind; patterns: RegExp[] }> = [
  {
    kind: "architecture_decision",
    patterns: [/\badr\b/i, /\barchitecture\b/i, /\bdecision\b/i, /\btrade-?off\b/i],
  },
  {
    kind: "known_issue",
    patterns: [/\bbug\b/i, /\bknown issue\b/i, /\bfail(?:s|ing|ed)?\b/i, /\bregression\b/i],
  },
  {
    kind: "setup_instruction",
    patterns: [/\bsetup\b/i, /\binstall\b/i, /\bbootstrap\b/i, /\b\.env\b/i],
  },
  {
    kind: "coding_convention",
    patterns: [/\bconvention\b/i, /\bstyle\b/i, /\blint\b/i, /\bformat\b/i, /\bnaming\b/i],
  },
  {
    kind: "user_preference",
    patterns: [/\buser prefer/i, /\bprefer(?:s|red)?\b/i, /\bwants\b/i, /\blikes\b/i],
  },
  {
    kind: "api_contract",
    patterns: [/\bapi\b/i, /\bcontract\b/i, /\bschema\b/i, /\brequest\b/i, /\bresponse\b/i],
  },
  {
    kind: "workflow_command",
    patterns: [/\bnpm run\b/i, /\bpnpm\b/i, /\byarn\b/i, /\bmake\b/i, /\bcommand\b/i],
  },
  {
    kind: "task_followup",
    patterns: [/\btodo\b/i, /\bfollow[- ]?up\b/i, /\bnext step\b/i, /\blater\b/i],
  },
  {
    kind: "project_context",
    patterns: [/\brepo\b/i, /\bproject\b/i, /\bworkspace\b/i, /\bpackage\b/i],
  },
];

function nowIso(): string {
  return new Date().toISOString();
}

function hashText(value: string): string {
  return createHash("sha256").update(value).digest("hex");
}

function safeSegment(value: string): string {
  const prefix = value
    .trim()
    .replace(/[^a-zA-Z0-9._-]+/g, "_")
    .replace(/^_+|_+$/g, "")
    .slice(0, 80);
  return `${prefix || "item"}-${hashText(value).slice(0, 10)}`;
}

function normalizeWhitespace(value: string): string {
  return value.trim().replace(/\s+/g, " ");
}

function normalizeTags(tags: readonly string[] | undefined): string[] {
  const seen = new Set<string>();
  const normalized: string[] = [];
  for (const tag of tags ?? []) {
    const value = tag.trim().toLowerCase().replace(/\s+/g, "-");
    if (!value || seen.has(value)) {
      continue;
    }
    seen.add(value);
    normalized.push(value);
  }
  return normalized.slice(0, 20);
}

function titleFromText(text: string): string {
  const firstLine = text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .find(Boolean);
  const candidate = firstLine || normalizeWhitespace(text);
  return candidate.slice(0, 90) || "Untitled memory";
}

function summaryFromText(text: string): string {
  return normalizeWhitespace(text).slice(0, 260) || "No summary";
}

function classifyMemoryKind(text: string): ProjectMemoryKind {
  for (const rule of MEMORY_KIND_RULES) {
    if (rule.patterns.some((pattern) => pattern.test(text))) {
      return rule.kind;
    }
  }
  return "note";
}

function classifyImportance(text: string): ProjectMemoryItem["importance"] {
  if (/\bcritical\b|\bmust\b|\bnever\b|\bbreaking\b|\bsecurity\b|\bsecret\b/i.test(text)) {
    return "high";
  }
  if (/\bshould\b|\bimportant\b|\bprefer\b|\btodo\b|\bfollow[- ]?up\b/i.test(text)) {
    return "medium";
  }
  return "low";
}

function redactSensitiveText(text: string): string {
  let current = text;
  for (const [pattern, replacement] of REDACTION_RULES) {
    current =
      typeof replacement === "function"
        ? current.replace(pattern, replacement)
        : current.replace(pattern, replacement);
  }
  return current;
}

function memoryFingerprint(workspaceId: string, cleanText: string): string {
  return hashText(`${workspaceId}\n${normalizeWhitespace(cleanText).toLowerCase()}`);
}

function extractPromptVariables(content: string): string[] {
  const variables = new Set<string>();
  for (const match of content.matchAll(/\{\{\s*([A-Za-z0-9_]+)\s*\}\}/g)) {
    variables.add(match[1]);
  }
  if (/\$ARGUMENTS\b/.test(content)) {
    variables.add("ARGUMENTS");
  }
  for (const match of content.matchAll(/\$(\d+)/g)) {
    variables.add(match[1]);
  }
  return [...variables].sort();
}

function renderPromptContent(
  content: string,
  variables: Record<string, string> | undefined,
  argumentsText: string | undefined,
): string {
  const args = argumentsText ?? variables?.ARGUMENTS ?? "";
  const argv =
    args.length > 0
      ? (args.match(/(?:[^\s"]+|"[^"]*")+/g)?.map((part) => part.replace(/^"|"$/g, "")) ?? [])
      : [];
  return content
    .replace(/\{\{\s*([A-Za-z0-9_]+)\s*\}\}/g, (_match, key: string) => variables?.[key] ?? "")
    .replace(/\$ARGUMENTS\b/g, args)
    .replace(/\$(\d+)/g, (_match, index: string) => argv[Number(index) - 1] ?? "");
}

function parseSkillDescription(content: string): string | null {
  const frontmatter = content.match(/^---\r?\n([\s\S]*?)\r?\n---/);
  if (frontmatter) {
    const description = frontmatter[1].match(/^description:\s*(.+)$/m)?.[1]?.trim();
    if (description) {
      return description.replace(/^["']|["']$/g, "");
    }
  }
  const heading = content.match(/^#\s+(.+)$/m)?.[1]?.trim();
  return heading || null;
}

function ensureSkillMarkdown(
  name: string,
  description: string | undefined,
  content: string,
): string {
  const body = content.trimEnd();
  if (!description || body.startsWith("---")) {
    return `${body}\n`;
  }
  return `---\ndescription: ${description.replace(/\r?\n/g, " ")}\n---\n\n# ${name}\n\n${body}\n`;
}

async function exists(filePath: string): Promise<boolean> {
  try {
    await access(filePath);
    return true;
  } catch {
    return false;
  }
}

async function readJsonFile<T>(filePath: string, schema: z.ZodType<T>, fallback: T): Promise<T> {
  try {
    const raw = await readFile(filePath, "utf8");
    return schema.parse(JSON.parse(raw));
  } catch (error) {
    if (error && typeof error === "object" && "code" in error && error.code === "ENOENT") {
      return fallback;
    }
    throw error;
  }
}

async function writeJsonFile(filePath: string, payload: unknown): Promise<void> {
  await mkdir(path.dirname(filePath), { recursive: true });
  const tempPath = `${filePath}.${process.pid}.${Date.now()}.${randomUUID()}.tmp`;
  await writeFile(tempPath, JSON.stringify(payload, null, 2), "utf8");
  await rename(tempPath, filePath);
}

export class ContextHubService {
  private readonly paseoHome: string;
  private readonly logger?: Logger;

  constructor(options: ContextHubServiceOptions) {
    this.paseoHome = options.paseoHome;
    this.logger = options.logger;
  }

  async listMemory(options: ListMemoryOptions): Promise<ListMemoryResult> {
    const page = options.page ?? 0;
    const pageSize = options.pageSize ?? 50;
    const query = options.query?.trim().toLowerCase();
    const tag = options.tag?.trim().toLowerCase();
    const allItems = await this.readWorkspaceMemory(options.workspaceId);
    const filtered = allItems
      .filter((item) => options.includeDeleted || !item.deletedAt)
      .filter((item) => !options.kind || item.kind === options.kind)
      .filter((item) => !options.importance || item.importance === options.importance)
      .filter((item) => !tag || item.tags.includes(tag))
      .filter((item) => {
        if (!query) {
          return true;
        }
        const haystack = [
          item.title,
          item.summary,
          item.detail,
          item.cleanText,
          item.kind,
          item.importance,
          item.tags.join(" "),
        ]
          .filter(Boolean)
          .join("\n")
          .toLowerCase();
        return haystack.includes(query);
      })
      .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
    return {
      items: filtered.slice(page * pageSize, page * pageSize + pageSize),
      total: filtered.length,
    };
  }

  async getMemory(workspaceId: string, memoryId: string): Promise<ProjectMemoryItem | null> {
    const location = await this.findMemoryLocation(workspaceId, memoryId);
    return location?.item ?? null;
  }

  async createMemory(input: ProjectMemoryCreateInput): Promise<ProjectMemoryItem> {
    const content = [input.title, input.summary, input.detail, input.rawText]
      .filter((value): value is string => Boolean(value?.trim()))
      .join("\n\n")
      .trim();
    if (!content) {
      throw new Error("Memory content is required");
    }

    const cleanText = redactSensitiveText(content);
    const fingerprint = memoryFingerprint(input.workspaceId, cleanText);
    const existing = (await this.readWorkspaceMemory(input.workspaceId)).find(
      (item) => !item.deletedAt && item.fingerprint === fingerprint,
    );
    if (existing) {
      return existing;
    }

    const createdAt = nowIso();
    const item: ProjectMemoryItem = ProjectMemoryItemSchema.parse({
      id: `mem_${randomUUID()}`,
      workspaceId: input.workspaceId,
      kind: input.kind ?? classifyMemoryKind(cleanText),
      title: input.title?.trim() || titleFromText(cleanText),
      summary: input.summary?.trim() || summaryFromText(cleanText),
      detail: input.detail?.trim() || null,
      rawText: input.rawText?.trim() || null,
      cleanText,
      tags: normalizeTags(input.tags),
      importance: input.importance ?? classifyImportance(cleanText),
      source: input.source?.trim() || "manual",
      threadId: input.threadId?.trim() || null,
      messageId: input.messageId?.trim() || null,
      metadata: input.metadata,
      fingerprint,
      createdAt,
      updatedAt: createdAt,
      deletedAt: null,
    });

    const bucketPath = this.memoryBucketPath(input.workspaceId, createdAt);
    const items = await this.readMemoryBucket(bucketPath);
    items.push(item);
    await writeJsonFile(bucketPath, items);
    return item;
  }

  async updateMemory(
    workspaceId: string,
    memoryId: string,
    patch: ProjectMemoryUpdateInput,
  ): Promise<ProjectMemoryItem | null> {
    const location = await this.findMemoryLocation(workspaceId, memoryId);
    if (!location) {
      return null;
    }

    const next = {
      ...location.item,
      ...patch,
      detail: patch.detail !== undefined ? patch.detail?.trim() || null : location.item.detail,
      rawText: patch.rawText !== undefined ? patch.rawText?.trim() || null : location.item.rawText,
      title:
        patch.title !== undefined
          ? patch.title?.trim() || location.item.title
          : location.item.title,
      summary:
        patch.summary !== undefined
          ? patch.summary?.trim() || location.item.summary
          : location.item.summary,
      tags: patch.tags !== undefined ? normalizeTags(patch.tags) : location.item.tags,
      source:
        patch.source !== undefined
          ? patch.source?.trim() || location.item.source
          : location.item.source,
      updatedAt: nowIso(),
    };

    const contentChanged =
      patch.title !== undefined ||
      patch.summary !== undefined ||
      patch.detail !== undefined ||
      patch.rawText !== undefined;
    if (contentChanged) {
      const content = [next.title, next.summary, next.detail, next.rawText]
        .filter((value): value is string => Boolean(value?.trim()))
        .join("\n\n");
      next.cleanText = redactSensitiveText(content);
      next.fingerprint = memoryFingerprint(workspaceId, next.cleanText);
      if (patch.kind === undefined) {
        next.kind = classifyMemoryKind(next.cleanText);
      }
      if (patch.importance === undefined) {
        next.importance = classifyImportance(next.cleanText);
      }
      const duplicate = (await this.readWorkspaceMemory(workspaceId)).find(
        (item) => item.id !== memoryId && !item.deletedAt && item.fingerprint === next.fingerprint,
      );
      if (duplicate) {
        throw new Error(`Memory duplicates existing item ${duplicate.id}`);
      }
    }

    const parsed = ProjectMemoryItemSchema.parse(next);
    const items = await this.readMemoryBucket(location.filePath);
    const index = items.findIndex((item) => item.id === memoryId);
    if (index === -1) {
      return null;
    }
    items[index] = parsed;
    await writeJsonFile(location.filePath, items);
    return parsed;
  }

  async deleteMemory(workspaceId: string, memoryId: string): Promise<ProjectMemoryItem | null> {
    return this.updateMemory(workspaceId, memoryId, { deletedAt: nowIso() });
  }

  async renderProjectMemoryContext(options: RenderMemoryContextOptions): Promise<string> {
    const selected: ProjectMemoryItem[] = [];
    const seen = new Set<string>();

    for (const memoryId of options.memoryIds ?? []) {
      const item = await this.getMemory(options.workspaceId, memoryId);
      if (item && !item.deletedAt && !seen.has(item.id)) {
        seen.add(item.id);
        selected.push(item);
      }
    }

    if (options.useWorkspaceMemory === true) {
      const { items } = await this.listMemory({
        workspaceId: options.workspaceId,
        pageSize: MEMORY_CONTEXT_LIMIT,
      });
      for (const item of items) {
        if (!seen.has(item.id)) {
          seen.add(item.id);
          selected.push(item);
        }
      }
    }

    if (selected.length === 0) {
      return "";
    }

    const lines = selected.slice(0, MEMORY_CONTEXT_LIMIT).map((item) => {
      const summary = normalizeWhitespace(item.summary || item.cleanText).slice(
        0,
        MEMORY_SNIPPET_LIMIT,
      );
      const tags = item.tags.length > 0 ? ` tags=${item.tags.join(",")}` : "";
      return `- [${item.kind}; ${item.importance}${tags}] ${item.title}: ${summary}`;
    });
    return `<project-memory>\n${lines.join("\n")}\n</project-memory>`;
  }

  async listSkills(options: ListSkillsOptions = {}): Promise<ManagedSkillEntry[]> {
    const roots: Array<{
      root: string;
      source: string;
      scope: "global" | "workspace";
      readOnly: boolean;
      workspaceId: string | null;
    }> = [
      {
        root: this.skillsRoot,
        source: "managed",
        scope: "global",
        readOnly: false,
        workspaceId: null,
      },
      {
        root: path.join(homedir(), ".codex", "skills"),
        source: "global_codex",
        scope: "global",
        readOnly: true,
        workspaceId: null,
      },
      {
        root: path.join(homedir(), ".claude", "skills"),
        source: "global_claude",
        scope: "global",
        readOnly: true,
        workspaceId: null,
      },
      {
        root: path.join(homedir(), ".agents", "skills"),
        source: "global_agents",
        scope: "global",
        readOnly: true,
        workspaceId: null,
      },
    ];

    const cwd = options.cwd?.trim();
    if (cwd) {
      roots.push(
        {
          root: path.join(cwd, ".codex", "skills"),
          source: "project_codex",
          scope: "workspace",
          readOnly: true,
          workspaceId: options.workspaceId ?? null,
        },
        {
          root: path.join(cwd, ".claude", "skills"),
          source: "project_claude",
          scope: "workspace",
          readOnly: true,
          workspaceId: options.workspaceId ?? null,
        },
        {
          root: path.join(cwd, ".agents", "skills"),
          source: "project_agents",
          scope: "workspace",
          readOnly: true,
          workspaceId: options.workspaceId ?? null,
        },
      );
    }

    const skills: ManagedSkillEntry[] = [];
    for (const root of roots) {
      skills.push(...(await this.scanSkillRoot(root, options.includeContent === true)));
    }
    return skills.sort((a, b) => a.name.localeCompare(b.name));
  }

  async importSkill(input: {
    name: string;
    content: string;
    description?: string;
  }): Promise<ManagedSkillEntry> {
    const skillDir = path.join(this.skillsRoot, safeSegment(input.name));
    const skillPath = path.join(skillDir, "SKILL.md");
    await mkdir(skillDir, { recursive: true });
    await writeFile(
      skillPath,
      ensureSkillMarkdown(input.name, input.description, input.content),
      "utf8",
    );
    const imported = (await this.listSkills({ includeContent: true })).find(
      (entry) => entry.path === skillPath,
    );
    if (!imported) {
      throw new Error(`Imported skill was not found at ${skillPath}`);
    }
    return imported;
  }

  async exportSkill(
    options: ExportSkillOptions,
  ): Promise<{ skill: ManagedSkillEntry; content: string }> {
    const skills = await this.listSkills({
      workspaceId: options.workspaceId,
      cwd: options.cwd,
      includeContent: true,
    });
    const skill = skills.find((entry) => entry.id === options.skillId);
    if (!skill) {
      throw new Error(`Skill not found: ${options.skillId}`);
    }
    const content = skill.content ?? (await readFile(skill.path, "utf8"));
    return { skill, content };
  }

  async listPrompts(
    options: { workspaceId?: string; includeDeleted?: boolean } = {},
  ): Promise<PromptTemplate[]> {
    const managed = (await this.readPromptStore()).prompts
      .filter((prompt) => options.includeDeleted || !prompt.deletedAt)
      .filter((prompt) => prompt.scope === "global" || prompt.workspaceId === options.workspaceId);
    const codex = await this.listCodexPrompts();
    return [...managed, ...codex].sort((a, b) => a.name.localeCompare(b.name));
  }

  async createPrompt(input: PromptTemplateCreateInput): Promise<PromptTemplate> {
    if (input.scope === "workspace" && !input.workspaceId?.trim()) {
      throw new Error("workspaceId is required for workspace prompts");
    }
    const now = nowIso();
    const prompt = PromptTemplateSchema.parse({
      id: `prompt_${randomUUID()}`,
      scope: input.scope ?? "global",
      workspaceId: input.scope === "workspace" ? (input.workspaceId?.trim() ?? null) : null,
      name: input.name.trim(),
      description: input.description?.trim() || null,
      content: input.content,
      tags: normalizeTags(input.tags),
      variables: extractPromptVariables(input.content),
      usageCount: 0,
      lastUsedAt: null,
      source: "managed",
      readOnly: false,
      path: null,
      createdAt: now,
      updatedAt: now,
      deletedAt: null,
    });
    const store = await this.readPromptStore();
    store.prompts.push(prompt);
    await this.writePromptStore(store);
    return prompt;
  }

  async updatePrompt(
    promptId: string,
    patch: PromptTemplateUpdateInput,
  ): Promise<PromptTemplate | null> {
    const store = await this.readPromptStore();
    const index = store.prompts.findIndex((prompt) => prompt.id === promptId);
    if (index === -1) {
      return null;
    }
    const current = store.prompts[index];
    if (current.readOnly) {
      throw new Error("Read-only prompts cannot be updated");
    }
    const next = PromptTemplateSchema.parse({
      ...current,
      ...patch,
      workspaceId:
        patch.scope === "global"
          ? null
          : patch.workspaceId !== undefined
            ? patch.workspaceId?.trim() || null
            : current.workspaceId,
      name: patch.name !== undefined ? patch.name.trim() : current.name,
      description:
        patch.description !== undefined ? patch.description?.trim() || null : current.description,
      tags: patch.tags !== undefined ? normalizeTags(patch.tags) : current.tags,
      variables:
        patch.content !== undefined ? extractPromptVariables(patch.content) : current.variables,
      updatedAt: nowIso(),
    });
    store.prompts[index] = next;
    await this.writePromptStore(store);
    return next;
  }

  async deletePrompt(promptId: string): Promise<string> {
    if (promptId.startsWith("codex:")) {
      throw new Error("Read-only Codex prompts cannot be deleted");
    }
    const prompt = await this.updatePrompt(promptId, { deletedAt: nowIso() });
    if (!prompt) {
      throw new Error(`Prompt not found: ${promptId}`);
    }
    return promptId;
  }

  async renderPrompt(options: RenderPromptOptions): Promise<RenderPromptResult> {
    const prompt = await this.getPrompt(options.promptId);
    if (!prompt || prompt.deletedAt) {
      throw new Error(`Prompt not found: ${options.promptId}`);
    }
    const text = renderPromptContent(prompt.content, options.variables, options.argumentsText);
    if (options.recordUsage === true && prompt.source === "managed") {
      await this.updatePromptUsage(prompt.id);
    }
    return { text, prompt };
  }

  async listMcpProfiles(): Promise<McpServerProfile[]> {
    return (await this.readMcpStore()).profiles.sort((a, b) => a.name.localeCompare(b.name));
  }

  async upsertMcpProfile(input: McpServerProfileUpsertInput): Promise<McpServerProfile> {
    this.validateMcpServerConfig(input.server);
    const now = nowIso();
    const store = await this.readMcpStore();
    const index = input.id ? store.profiles.findIndex((profile) => profile.id === input.id) : -1;
    const current = index >= 0 ? store.profiles[index] : null;
    const profile = McpServerProfileSchema.parse({
      id: current?.id ?? input.id ?? `mcp_${randomUUID()}`,
      name: input.name.trim(),
      enabled: input.enabled ?? current?.enabled ?? true,
      providerIds: input.providerIds ?? current?.providerIds ?? [],
      workspaceIds: input.workspaceIds ?? current?.workspaceIds ?? [],
      server: input.server,
      createdAt: current?.createdAt ?? now,
      updatedAt: now,
    });
    if (index >= 0) {
      store.profiles[index] = profile;
    } else {
      store.profiles.push(profile);
    }
    await this.writeMcpStore(store);
    return profile;
  }

  async deleteMcpProfile(profileId: string): Promise<string> {
    const store = await this.readMcpStore();
    const nextProfiles = store.profiles.filter((profile) => profile.id !== profileId);
    if (nextProfiles.length === store.profiles.length) {
      throw new Error(`MCP server not found: ${profileId}`);
    }
    await this.writeMcpStore({ profiles: nextProfiles });
    return profileId;
  }

  async testMcpProfile(
    input: McpServerProfileUpsertInput,
  ): Promise<{ ok: boolean; message: string }> {
    try {
      this.validateMcpServerConfig(input.server);
      return { ok: true, message: "Configuration looks valid." };
    } catch (error) {
      return {
        ok: false,
        message: error instanceof Error ? error.message : String(error),
      };
    }
  }

  async resolveMcpServers(
    options: ResolveMcpServersOptions,
  ): Promise<Record<string, ContextHubMcpServerConfig>> {
    if (!options.ids || options.ids.length === 0) {
      return {};
    }
    const idSet = new Set(options.ids);
    const profiles = (await this.listMcpProfiles()).filter(
      (profile) => profile.enabled && idSet.has(profile.id),
    );
    const result: Record<string, ContextHubMcpServerConfig> = {};
    for (const profile of profiles) {
      let key = profile.name.trim().replace(/[^a-zA-Z0-9._-]+/g, "_") || profile.id;
      if (result[key]) {
        key = `${key}_${profile.id.slice(0, 8)}`;
      }
      result[key] = profile.server;
    }
    return result;
  }

  private get memoryRoot(): string {
    return path.join(this.paseoHome, "memory");
  }

  private get promptsPath(): string {
    return path.join(this.paseoHome, "prompts", "prompts.json");
  }

  private get skillsRoot(): string {
    return path.join(this.paseoHome, "skills");
  }

  private get mcpPath(): string {
    return path.join(this.paseoHome, "mcp", "servers.json");
  }

  private memoryWorkspaceDir(workspaceId: string): string {
    return path.join(this.memoryRoot, safeSegment(workspaceId));
  }

  private memoryBucketPath(workspaceId: string, isoDate: string): string {
    return path.join(this.memoryWorkspaceDir(workspaceId), `${isoDate.slice(0, 10)}.json`);
  }

  private async readWorkspaceMemory(workspaceId: string): Promise<ProjectMemoryItem[]> {
    const dir = this.memoryWorkspaceDir(workspaceId);
    let entries: string[];
    try {
      entries = await readdir(dir);
    } catch (error) {
      if (error && typeof error === "object" && "code" in error && error.code === "ENOENT") {
        return [];
      }
      throw error;
    }
    const items: ProjectMemoryItem[] = [];
    for (const entry of entries) {
      if (!entry.endsWith(".json")) {
        continue;
      }
      items.push(...(await this.readMemoryBucket(path.join(dir, entry))));
    }
    return items;
  }

  private async readMemoryBucket(filePath: string): Promise<ProjectMemoryItem[]> {
    return readJsonFile(filePath, z.array(ProjectMemoryItemSchema), []);
  }

  private async findMemoryLocation(
    workspaceId: string,
    memoryId: string,
  ): Promise<{ filePath: string; item: ProjectMemoryItem } | null> {
    const dir = this.memoryWorkspaceDir(workspaceId);
    let entries: string[];
    try {
      entries = await readdir(dir);
    } catch (error) {
      if (error && typeof error === "object" && "code" in error && error.code === "ENOENT") {
        return null;
      }
      throw error;
    }
    for (const entry of entries) {
      if (!entry.endsWith(".json")) {
        continue;
      }
      const filePath = path.join(dir, entry);
      const item = (await this.readMemoryBucket(filePath)).find(
        (candidate) => candidate.id === memoryId,
      );
      if (item) {
        return { filePath, item };
      }
    }
    return null;
  }

  private async scanSkillRoot(
    rootInfo: {
      root: string;
      source: string;
      scope: "global" | "workspace";
      readOnly: boolean;
      workspaceId: string | null;
    },
    includeContent: boolean,
  ): Promise<ManagedSkillEntry[]> {
    if (!(await exists(rootInfo.root))) {
      return [];
    }
    const entries = await readdir(rootInfo.root, { withFileTypes: true });
    const skills: ManagedSkillEntry[] = [];
    for (const entry of entries) {
      if (!entry.isDirectory()) {
        continue;
      }
      const skillPath = path.join(rootInfo.root, entry.name, "SKILL.md");
      if (!(await exists(skillPath))) {
        continue;
      }
      try {
        const content = await readFile(skillPath, "utf8");
        const stats = await stat(skillPath);
        skills.push(
          ManagedSkillEntrySchema.parse({
            id:
              rootInfo.source === "managed"
                ? `managed:${entry.name}`
                : `${rootInfo.source}:${hashText(skillPath).slice(0, 16)}`,
            name: entry.name,
            description: parseSkillDescription(content),
            source: rootInfo.source,
            scope: rootInfo.scope,
            workspaceId: rootInfo.workspaceId,
            path: skillPath,
            readOnly: rootInfo.readOnly,
            content: includeContent ? content : null,
            createdAt: stats.birthtime.toISOString(),
            updatedAt: stats.mtime.toISOString(),
          }),
        );
      } catch (error) {
        this.logger?.warn({ err: error, skillPath }, "Failed to scan skill");
      }
    }
    return skills;
  }

  private async readPromptStore(): Promise<{ prompts: PromptTemplate[] }> {
    return readJsonFile(this.promptsPath, PROMPT_STORE_SCHEMA, { prompts: [] });
  }

  private async writePromptStore(store: { prompts: PromptTemplate[] }): Promise<void> {
    await writeJsonFile(this.promptsPath, store);
  }

  private async listCodexPrompts(): Promise<PromptTemplate[]> {
    const promptsDir = path.join(homedir(), ".codex", "prompts");
    let entries: string[];
    try {
      entries = await readdir(promptsDir);
    } catch (error) {
      if (error && typeof error === "object" && "code" in error && error.code === "ENOENT") {
        return [];
      }
      throw error;
    }
    const prompts: PromptTemplate[] = [];
    for (const entry of entries) {
      if (!entry.endsWith(".md")) {
        continue;
      }
      const filePath = path.join(promptsDir, entry);
      const content = await readFile(filePath, "utf8");
      const stats = await stat(filePath);
      const name = entry.replace(/\.md$/, "");
      prompts.push(
        PromptTemplateSchema.parse({
          id: `codex:${name}`,
          scope: "global",
          workspaceId: null,
          name,
          description: null,
          content,
          tags: ["codex"],
          variables: extractPromptVariables(content),
          usageCount: 0,
          lastUsedAt: null,
          source: "codex",
          readOnly: true,
          path: filePath,
          createdAt: stats.birthtime.toISOString(),
          updatedAt: stats.mtime.toISOString(),
          deletedAt: null,
        }),
      );
    }
    return prompts;
  }

  private async getPrompt(promptId: string): Promise<PromptTemplate | null> {
    if (promptId.startsWith("codex:")) {
      return (await this.listCodexPrompts()).find((prompt) => prompt.id === promptId) ?? null;
    }
    return (await this.readPromptStore()).prompts.find((prompt) => prompt.id === promptId) ?? null;
  }

  private async updatePromptUsage(promptId: string): Promise<void> {
    const store = await this.readPromptStore();
    const index = store.prompts.findIndex((prompt) => prompt.id === promptId);
    if (index === -1) {
      return;
    }
    const current = store.prompts[index];
    store.prompts[index] = PromptTemplateSchema.parse({
      ...current,
      usageCount: current.usageCount + 1,
      lastUsedAt: nowIso(),
      updatedAt: nowIso(),
    });
    await this.writePromptStore(store);
  }

  private async readMcpStore(): Promise<{ profiles: McpServerProfile[] }> {
    return readJsonFile(this.mcpPath, MCP_STORE_SCHEMA, { profiles: [] });
  }

  private async writeMcpStore(store: { profiles: McpServerProfile[] }): Promise<void> {
    await writeJsonFile(this.mcpPath, store);
  }

  private validateMcpServerConfig(config: ContextHubMcpServerConfig): void {
    const parsed = ContextHubMcpServerConfigSchema.parse(config);
    if (parsed.type === "stdio") {
      if (!parsed.command.trim()) {
        throw new Error("stdio MCP servers require a command");
      }
      return;
    }
    try {
      const url = new URL(parsed.url);
      if (url.protocol !== "http:" && url.protocol !== "https:") {
        throw new Error("MCP URL must use http or https");
      }
    } catch {
      throw new Error("MCP URL is invalid");
    }
  }
}

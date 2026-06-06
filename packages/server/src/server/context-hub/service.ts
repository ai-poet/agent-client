import { createHash, randomUUID } from "node:crypto";
import {
  access,
  cp,
  lstat,
  mkdir,
  readlink,
  readFile,
  readdir,
  rename,
  rm,
  stat,
  symlink as fsSymlink,
  writeFile,
} from "node:fs/promises";
import { homedir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import AdmZip from "adm-zip";
import type pino from "pino";
import { z } from "zod";
import type { AgentProvider } from "../agent/agent-sdk-types.js";
import {
  ContextHubMcpServerConfigSchema,
  ManagedSkillEntrySchema,
  MarketplaceSkillEntrySchema,
  McpServerProfileSchema,
  ProjectMemoryItemSchema,
  PromptTemplateSchema,
  type ContextHubMcpServerConfig,
  type ManagedSkillEntry,
  type MarketplaceSkillEntry,
  type McpServerProfile,
  type McpServerProfileUpsertInput,
  type ProjectMemoryCreateInput,
  type ProjectMemoryItem,
  type ProjectMemoryKind,
  type ProjectMemoryUpdateInput,
  type PromptTemplate,
  type PromptTemplateCreateInput,
  type PromptTemplateUpdateInput,
  type SkillScope,
  type SkillWritableTarget,
} from "./rpc-schemas.js";

const MEMORY_CONTEXT_LIMIT = 8;
const MEMORY_SNIPPET_LIMIT = 900;
const PROMPT_STORE_SCHEMA = z.object({
  prompts: z.array(PromptTemplateSchema),
});
const MCP_STORE_SCHEMA = z.object({
  profiles: z.array(McpServerProfileSchema),
});
const SKILLSMP_BASE_URL = "https://skillsmp.com";
const SKILLSMP_SKILL_ID_PREFIX = "skillsmp:";
const SKILLSMP_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZ".split("");
const SKILLSMP_ALPHABET_FETCH_LIMIT = 50;
const SKILLSMP_ALPHABET_PER_LETTER_LIMIT = 10;
const SKILLSMP_SEARCH_MAX_LIMIT = 50;
const BUNDLED_SKILLS_ENV = "PASEO_BUNDLED_SKILLS_DIR";
const SKILL_PACKAGE_MAX_FILES = 100;
const SKILL_PACKAGE_MAX_FILE_BYTES = 5 * 1024 * 1024;
const SKILL_PACKAGE_MAX_TOTAL_BYTES = 20 * 1024 * 1024;
const SKILL_MARKDOWN_MAX_BYTES = 512 * 1024;
const WORKSPACE_SKILL_EXCLUDE_ENTRIES = [
  "# Paseo workspace skills",
  ".agents/skills/",
  ".codex/skills/",
  ".claude/skills/",
] as const;
const TRUSTED_SKILLSMP_AUTHORS = new Set([
  "anthropics",
  "github",
  "google-gemini",
  "microsoft",
  "openai",
  "supabase",
]);

type Logger = Pick<pino.Logger, "debug" | "warn" | "error">;

type ContextHubServiceOptions = {
  paseoHome: string;
  logger?: Logger;
  bundledSkillsRoots?: string[];
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

type SaveSkillOptions = {
  target: SkillWritableTarget;
  scope: SkillScope;
  skillId?: string;
  name: string;
  content: string;
  workspaceId?: string;
  cwd?: string | null;
  overwrite?: boolean;
};

type ImportSkillPackageOptions = {
  target: SkillWritableTarget;
  scope: SkillScope;
  name: string;
  packageBuffer: Buffer;
  workspaceId?: string;
  cwd?: string | null;
  overwrite?: boolean;
};

type DeleteSkillOptions = {
  skillId: string;
  workspaceId?: string;
  cwd?: string | null;
};

type ListMarketplaceSkillsOptions = {
  query?: string;
  capability?: string;
  limit?: number;
  minTrust?: "verified" | "community" | "sandbox";
  workspaceId?: string;
  cwd?: string | null;
};

type InstallMarketplaceSkillOptions = {
  workspaceId: string;
  cwd: string;
  skillId: string;
  name: string;
  version?: string;
  overwrite?: boolean;
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

type ValidSkillZipEntry = {
  entryName: string;
  relativePath: string;
  isDirectory: boolean;
  size: number;
};

type NormalizedSkillZipEntry = ValidSkillZipEntry & {
  normalizedRelativePath: string;
};

type WritableSkillRoot = {
  root: string;
  source: SkillWritableTarget;
  scope: SkillScope;
  readOnly: false;
  workspaceId: string | null;
};

type SkillsMpSkill = {
  id: string;
  name: string;
  author: string;
  description: string;
  githubUrl: string;
  skillUrl: string;
  stars: number;
  updatedAt: number;
};

class SkillMarketplaceConflictError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "SkillMarketplaceConflictError";
  }
}

class SkillConflictError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "SkillConflictError";
  }
}

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

function uniquePaths(paths: string[]): string[] {
  const seen = new Set<string>();
  const unique: string[] = [];
  for (const rawPath of paths) {
    const trimmed = rawPath.trim();
    if (!trimmed) {
      continue;
    }
    const resolved = path.resolve(trimmed);
    if (seen.has(resolved)) {
      continue;
    }
    seen.add(resolved);
    unique.push(resolved);
  }
  return unique;
}

function splitPathList(value: string | undefined): string[] {
  if (!value) {
    return [];
  }
  return value
    .split(path.delimiter)
    .map((entry) => entry.trim())
    .filter(Boolean);
}

function resolveBundledSkillsRoots(extraRoots: string[] = []): string[] {
  const moduleDir = path.dirname(fileURLToPath(import.meta.url));
  return uniquePaths([
    ...extraRoots,
    ...splitPathList(process.env[BUNDLED_SKILLS_ENV]),
    path.join(process.cwd(), "skills"),
    path.resolve(moduleDir, "../../../../../skills"),
    path.resolve(moduleDir, "../../../../../../skills"),
  ]);
}

function safeSegment(value: string): string {
  const prefix = value
    .trim()
    .replace(/[^a-zA-Z0-9._-]+/g, "_")
    .replace(/^_+|_+$/g, "")
    .slice(0, 80);
  return `${prefix || "item"}-${hashText(value).slice(0, 10)}`;
}

function safeSkillName(value: string): string {
  return (
    value
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9._-]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 80) || "skill"
  );
}

function ensureSkillMarkdownContent(content: string): string {
  const trimmed = content.trimEnd();
  if (!trimmed) {
    throw new Error("Skill content is required");
  }
  return `${trimmed}\n`;
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

function parseSimpleFrontMatter(content: string): { lines: string[]; body: string } | null {
  const lines = content.split(/\r?\n/);
  if (lines[0]?.trim() !== "---") {
    return null;
  }
  const end = lines.findIndex((line, index) => index > 0 && line.trim() === "---");
  if (end === -1) {
    return null;
  }
  return { lines: lines.slice(1, end), body: lines.slice(end + 1).join("\n") };
}

function hasFrontMatterKey(lines: string[], key: string): boolean {
  const pattern = new RegExp(`^\\s*${key}\\s*:`);
  return lines.some((line) => pattern.test(line));
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

function ensureInvocableSkillMarkdown(input: {
  name: string;
  description: string;
  content: string;
}): string {
  const safeName = safeSkillName(input.name);
  const description = normalizeWhitespace(input.description || "Skill");
  const content = input.content.trimEnd();
  const parsed = parseSimpleFrontMatter(content);
  const frontMatterLines = parsed?.lines ?? [];
  const body = (parsed?.body ?? content).trimStart();
  if (!hasFrontMatterKey(frontMatterLines, "name")) {
    frontMatterLines.push(`name: ${safeName}`);
  }
  if (!hasFrontMatterKey(frontMatterLines, "description")) {
    frontMatterLines.push(`description: ${description}`);
  }
  const frontMatterText = frontMatterLines.join("\n");
  return `---\n${frontMatterText}\n---\n\n${body || `# ${safeName}`}\n`;
}

function parseUnixTimestampSeconds(value: number | string): number | null {
  const timestamp = typeof value === "number" ? value : Number.parseInt(String(value).trim(), 10);
  return Number.isFinite(timestamp) && timestamp > 0 ? timestamp : null;
}

function daysSinceUnixTimestampSeconds(value: number | string): number | null {
  const timestamp = parseUnixTimestampSeconds(value);
  if (!timestamp) {
    return null;
  }
  return Math.max(0, Math.floor((Date.now() - timestamp * 1000) / 86_400_000));
}

function inferSkillCapabilities(description: string): string[] {
  const normalized = description.toLowerCase();
  const capabilities: string[] = [];
  const rules: Array<[RegExp, string]> = [
    [/\breview|pull request|pr\b/, "review"],
    [/\btest|testing|qa\b/, "testing"],
    [/\bsecurity|vulnerab|audit\b/, "security"],
    [/\bfrontend|ui|design\b/, "frontend"],
    [/\bdeploy|devops|ci\/cd|workflow\b/, "devops"],
    [/\bdoc|documentation|write\b/, "documentation"],
  ];
  for (const [pattern, capability] of rules) {
    if (pattern.test(normalized)) {
      capabilities.push(capability);
    }
  }
  return capabilities;
}

function compareMarketplaceSkillPriority(
  left: MarketplaceSkillEntry,
  right: MarketplaceSkillEntry,
): number {
  const leftTrusted = left.vettingStatus === "trusted-source" || left.trustLevel === "verified";
  const rightTrusted = right.vettingStatus === "trusted-source" || right.trustLevel === "verified";
  if (leftTrusted !== rightTrusted) {
    return leftTrusted ? -1 : 1;
  }
  if (left.downloadCount !== right.downloadCount) {
    return right.downloadCount - left.downloadCount;
  }
  return left.name.localeCompare(right.name);
}

function parseGithubTreeRawSkillUrl(githubUrl: string): string {
  let url: URL;
  try {
    url = new URL(githubUrl);
  } catch {
    throw new Error(`SkillsMP skill has an invalid GitHub URL: ${githubUrl}`);
  }
  if (url.hostname !== "github.com") {
    throw new Error(`SkillsMP installs require a github.com tree URL: ${githubUrl}`);
  }
  const parts = url.pathname.split("/").filter(Boolean);
  const [owner, repo, marker, branch, ...dirParts] = parts;
  if (!owner || !repo || marker !== "tree" || !branch || dirParts.length === 0) {
    throw new Error(`SkillsMP installs require a GitHub tree URL: ${githubUrl}`);
  }
  return `https://raw.githubusercontent.com/${owner}/${repo}/${branch}/${dirParts.join("/")}/SKILL.md`;
}

function marketplaceSkillInitial(name: string): string | null {
  const initial = name
    .trim()
    .match(/[A-Za-z]/)?.[0]
    ?.toUpperCase();
  return initial && /^[A-Z]$/.test(initial) ? initial : null;
}

async function exists(filePath: string): Promise<boolean> {
  try {
    await access(filePath);
    return true;
  } catch {
    return false;
  }
}

async function directoryDigest(dir: string): Promise<string | null> {
  if (!(await exists(dir))) {
    return null;
  }
  const hash = createHash("sha256");
  async function visit(currentDir: string, prefix: string): Promise<void> {
    const entries = (await readdir(currentDir, { withFileTypes: true })).sort((a, b) =>
      a.name.localeCompare(b.name),
    );
    for (const entry of entries) {
      const relativePath = prefix ? `${prefix}/${entry.name}` : entry.name;
      const fullPath = path.join(currentDir, entry.name);
      const stats = await lstat(fullPath);
      if (stats.isSymbolicLink()) {
        const target = await readlink(fullPath).catch(() => "");
        hash.update(`link:${relativePath}:${target}\n`);
        continue;
      }
      if (stats.isDirectory()) {
        hash.update(`dir:${relativePath}\n`);
        await visit(fullPath, relativePath);
        continue;
      }
      if (stats.isFile()) {
        hash.update(`file:${relativePath}:${stats.size}\n`);
        hash.update(await readFile(fullPath));
      }
    }
  }
  await visit(dir, "");
  return hash.digest("hex");
}

function assertPathInside(parent: string, candidate: string): void {
  const relative = path.relative(path.resolve(parent), path.resolve(candidate));
  if (relative.startsWith("..") || path.isAbsolute(relative)) {
    throw new Error(`Refusing to write outside workspace: ${candidate}`);
  }
}

function isWritableSkillSource(source: string): source is SkillWritableTarget {
  return (
    source === "managed" ||
    source === "global_codex" ||
    source === "global_claude" ||
    source === "global_agents" ||
    source === "project_codex" ||
    source === "project_claude" ||
    source === "project_agents"
  );
}

async function copyOrSymlinkSkillDirectory(sourceDir: string, destDir: string): Promise<void> {
  await rm(destDir, { recursive: true, force: true });
  await mkdir(path.dirname(destDir), { recursive: true });
  try {
    await fsSymlink(sourceDir, destDir, process.platform === "win32" ? "junction" : "dir");
  } catch {
    await cp(sourceDir, destDir, { recursive: true });
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
  private readonly bundledSkillsRoots: string[];
  private readonly skillsMpSkillCache = new Map<string, SkillsMpSkill>();

  constructor(options: ContextHubServiceOptions) {
    this.paseoHome = options.paseoHome;
    this.logger = options.logger;
    this.bundledSkillsRoots = resolveBundledSkillsRoots(options.bundledSkillsRoots);
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
      ...this.bundledSkillsRoots.map((root) => ({
        root,
        source: "bundled",
        scope: "global" as const,
        readOnly: true,
        workspaceId: null,
      })),
      {
        root: path.join(homedir(), ".codex", "skills"),
        source: "global_codex",
        scope: "global",
        readOnly: false,
        workspaceId: null,
      },
      {
        root: path.join(homedir(), ".claude", "skills"),
        source: "global_claude",
        scope: "global",
        readOnly: false,
        workspaceId: null,
      },
      {
        root: path.join(homedir(), ".agents", "skills"),
        source: "global_agents",
        scope: "global",
        readOnly: false,
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
          readOnly: false,
          workspaceId: options.workspaceId ?? null,
        },
        {
          root: path.join(cwd, ".claude", "skills"),
          source: "project_claude",
          scope: "workspace",
          readOnly: false,
          workspaceId: options.workspaceId ?? null,
        },
        {
          root: path.join(cwd, ".agents", "skills"),
          source: "project_agents",
          scope: "workspace",
          readOnly: false,
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

  async saveSkill(
    options: SaveSkillOptions,
  ): Promise<{ skill: ManagedSkillEntry; conflict: boolean }> {
    const rootInfo = this.resolveWritableSkillRoot({
      target: options.target,
      scope: options.scope,
      workspaceId: options.workspaceId,
      cwd: options.cwd,
    });
    const content = ensureSkillMarkdownContent(options.content);
    const existingSkill = options.skillId
      ? await this.getWritableSkillForMutation({
          skillId: options.skillId,
          rootInfo,
          workspaceId: options.workspaceId,
          cwd: options.cwd,
        })
      : null;

    if (existingSkill) {
      await writeFile(existingSkill.path, content, "utf8");
      return {
        skill: await this.readSkillFromPath(rootInfo, existingSkill.path, true),
        conflict: false,
      };
    }

    const safeName = safeSkillName(options.name);
    const skillDir = path.join(rootInfo.root, safeName);
    const skillPath = path.join(skillDir, "SKILL.md");
    assertPathInside(rootInfo.root, skillDir);
    if ((await exists(skillDir)) && options.overwrite !== true) {
      throw new SkillConflictError(`Skill already exists: ${safeName}`);
    }
    if (options.overwrite === true) {
      await rm(skillDir, { recursive: true, force: true });
    }
    await mkdir(skillDir, { recursive: true });
    await writeFile(skillPath, content, "utf8");
    return {
      skill: await this.readSkillFromPath(rootInfo, skillPath, true),
      conflict: false,
    };
  }

  async importSkillPackage(
    options: ImportSkillPackageOptions,
  ): Promise<{ skill: ManagedSkillEntry; conflict: boolean }> {
    if (options.packageBuffer.byteLength > SKILL_PACKAGE_MAX_TOTAL_BYTES) {
      throw new Error("Skill package is too large");
    }
    const rootInfo = this.resolveWritableSkillRoot({
      target: options.target,
      scope: options.scope,
      workspaceId: options.workspaceId,
      cwd: options.cwd,
    });
    const safeName = safeSkillName(options.name);
    const tempRoot = path.join(rootInfo.root, `.paseo-import-${safeName}-${randomUUID()}`);
    const sourceDir = path.join(tempRoot, safeName);
    const targetDir = path.join(rootInfo.root, safeName);
    assertPathInside(rootInfo.root, tempRoot);
    assertPathInside(rootInfo.root, targetDir);

    await rm(tempRoot, { recursive: true, force: true });
    try {
      await this.extractSkillPackage(options.packageBuffer, sourceDir);
      const existingDigest = await directoryDigest(targetDir);
      const nextDigest = await directoryDigest(sourceDir);
      if (!nextDigest) {
        throw new Error("Skill package did not produce an installable directory");
      }
      if (existingDigest && existingDigest !== nextDigest && options.overwrite !== true) {
        throw new SkillConflictError(`Skill already exists with different content: ${safeName}`);
      }
      if (existingDigest === nextDigest) {
        const skill = await this.findSkillByPath({
          rootInfo,
          skillPath: path.join(targetDir, "SKILL.md"),
          includeContent: true,
        });
        if (!skill) {
          throw new Error(`Imported skill was not found: ${safeName}`);
        }
        return { skill, conflict: false };
      }

      await rm(targetDir, { recursive: true, force: true });
      await mkdir(path.dirname(targetDir), { recursive: true });
      await rename(sourceDir, targetDir);
      const skill = await this.findSkillByPath({
        rootInfo,
        skillPath: path.join(targetDir, "SKILL.md"),
        includeContent: true,
      });
      if (!skill) {
        throw new Error(`Imported skill was not found: ${safeName}`);
      }
      return { skill, conflict: false };
    } finally {
      await rm(tempRoot, { recursive: true, force: true });
    }
  }

  async deleteSkill(options: DeleteSkillOptions): Promise<string> {
    const skill = (
      await this.listSkills({
        workspaceId: options.workspaceId,
        cwd: options.cwd,
      })
    ).find((entry) => entry.id === options.skillId);
    if (!skill) {
      throw new Error(`Skill not found: ${options.skillId}`);
    }
    if (skill.readOnly || !isWritableSkillSource(skill.source)) {
      throw new Error(`Skill is read-only: ${skill.name}`);
    }
    const rootInfo = this.resolveWritableSkillRoot({
      target: skill.source,
      scope: skill.scope,
      workspaceId: options.workspaceId,
      cwd: options.cwd,
    });
    const skillDir = path.dirname(skill.path);
    assertPathInside(rootInfo.root, skillDir);
    if (path.resolve(skillDir) === path.resolve(rootInfo.root)) {
      throw new Error(`Refusing to delete skills root: ${skillDir}`);
    }
    await rm(skillDir, { recursive: true, force: true });
    return skill.id;
  }

  async listMarketplaceSkills(
    options: ListMarketplaceSkillsOptions = {},
  ): Promise<MarketplaceSkillEntry[]> {
    const installedNames = await this.getInstalledWorkspaceSkillNames({
      workspaceId: options.workspaceId,
      cwd: options.cwd,
    });
    return this.listSkillsMpSkills(options, installedNames);
  }

  async installMarketplaceSkill(
    options: InstallMarketplaceSkillOptions,
  ): Promise<{ skill: ManagedSkillEntry; installed: boolean; conflict: boolean }> {
    const workspaceRoot = options.cwd.trim();
    if (!workspaceRoot) {
      throw new Error("cwd is required to install a marketplace skill");
    }
    if (options.skillId.startsWith(SKILLSMP_SKILL_ID_PREFIX)) {
      const packageBuffer = await this.downloadSkillsMpSkillPackage(options);
      return this.installMarketplaceSkillPackage({
        ...options,
        packageBuffer,
      });
    }
    throw new Error(`Unsupported marketplace skill id: ${options.skillId}`);
  }

  async installMarketplaceSkillPackage(
    options: InstallMarketplaceSkillOptions & { packageBuffer: Buffer },
  ): Promise<{ skill: ManagedSkillEntry; installed: boolean; conflict: boolean }> {
    const workspaceRoot = options.cwd.trim();
    if (!workspaceRoot) {
      throw new Error("cwd is required to install a marketplace skill");
    }
    const safeName = safeSkillName(options.name);
    const tempRoot = path.join(
      workspaceRoot,
      ".agents",
      "skills",
      `.paseo-install-${safeName}-${randomUUID()}`,
    );
    const sourceDir = path.join(tempRoot, safeName);
    const targetDir = path.join(workspaceRoot, ".agents", "skills", safeName);
    assertPathInside(workspaceRoot, tempRoot);
    assertPathInside(workspaceRoot, targetDir);

    await rm(tempRoot, { recursive: true, force: true });
    try {
      await this.extractMarketplaceSkillPackage(options.packageBuffer, sourceDir);
      const existingDigest = await directoryDigest(targetDir);
      const nextDigest = await directoryDigest(sourceDir);
      if (!nextDigest) {
        throw new Error("Downloaded skill package did not produce an installable directory");
      }
      if (existingDigest && existingDigest !== nextDigest && options.overwrite !== true) {
        throw new SkillMarketplaceConflictError(
          `Skill already exists with different content: ${safeName}`,
        );
      }
      if (existingDigest === nextDigest) {
        await this.syncWorkspaceSkillProviderDirs(workspaceRoot, safeName);
        await this.updateWorkspaceSkillExclude(workspaceRoot);
        const skill = await this.findWorkspaceSkill({
          workspaceId: options.workspaceId,
          cwd: workspaceRoot,
          name: safeName,
        });
        if (!skill) {
          throw new Error(`Installed skill was not found after sync: ${safeName}`);
        }
        return { skill, installed: false, conflict: false };
      }

      await rm(targetDir, { recursive: true, force: true });
      await mkdir(path.dirname(targetDir), { recursive: true });
      await rename(sourceDir, targetDir);
      await this.syncWorkspaceSkillProviderDirs(workspaceRoot, safeName);
      await this.updateWorkspaceSkillExclude(workspaceRoot);
      const skill = await this.findWorkspaceSkill({
        workspaceId: options.workspaceId,
        cwd: workspaceRoot,
        name: safeName,
      });
      if (!skill) {
        throw new Error(`Installed skill was not found: ${safeName}`);
      }
      return { skill, installed: true, conflict: false };
    } finally {
      await rm(tempRoot, { recursive: true, force: true });
    }
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

  private resolveWritableSkillRoot(options: {
    target: SkillWritableTarget;
    scope: SkillScope;
    workspaceId?: string;
    cwd?: string | null;
  }): WritableSkillRoot {
    const targetScope: SkillScope = options.target.startsWith("project_") ? "workspace" : "global";
    if (targetScope !== options.scope) {
      throw new Error(`Skill target ${options.target} does not match ${options.scope} scope`);
    }
    if (options.scope === "workspace") {
      const workspaceRoot = options.cwd?.trim();
      if (!workspaceRoot) {
        throw new Error("cwd is required for workspace skills");
      }
      const root =
        options.target === "project_codex"
          ? path.join(workspaceRoot, ".codex", "skills")
          : options.target === "project_claude"
            ? path.join(workspaceRoot, ".claude", "skills")
            : path.join(workspaceRoot, ".agents", "skills");
      assertPathInside(workspaceRoot, root);
      return {
        root,
        source: options.target,
        scope: "workspace",
        readOnly: false,
        workspaceId: options.workspaceId ?? null,
      };
    }
    const root =
      options.target === "managed"
        ? this.skillsRoot
        : options.target === "global_codex"
          ? path.join(homedir(), ".codex", "skills")
          : options.target === "global_claude"
            ? path.join(homedir(), ".claude", "skills")
            : path.join(homedir(), ".agents", "skills");
    return {
      root,
      source: options.target,
      scope: "global",
      readOnly: false,
      workspaceId: null,
    };
  }

  private async readSkillFromPath(
    rootInfo: WritableSkillRoot,
    skillPath: string,
    includeContent: boolean,
  ): Promise<ManagedSkillEntry> {
    const skill = await this.findSkillByPath({ rootInfo, skillPath, includeContent });
    if (!skill) {
      throw new Error(`Skill was not found at ${skillPath}`);
    }
    return skill;
  }

  private async findSkillByPath(options: {
    rootInfo: WritableSkillRoot;
    skillPath: string;
    includeContent: boolean;
  }): Promise<ManagedSkillEntry | null> {
    const normalizedPath = path.resolve(options.skillPath);
    const entries = await this.scanSkillRoot(options.rootInfo, options.includeContent);
    return entries.find((entry) => path.resolve(entry.path) === normalizedPath) ?? null;
  }

  private async getWritableSkillForMutation(options: {
    skillId: string;
    rootInfo: WritableSkillRoot;
    workspaceId?: string;
    cwd?: string | null;
  }): Promise<ManagedSkillEntry> {
    const skill = (
      await this.listSkills({
        workspaceId: options.workspaceId,
        cwd: options.cwd,
        includeContent: true,
      })
    ).find((entry) => entry.id === options.skillId);
    if (!skill) {
      throw new Error(`Skill not found: ${options.skillId}`);
    }
    if (skill.readOnly || !isWritableSkillSource(skill.source)) {
      throw new Error(`Skill is read-only: ${skill.name}`);
    }
    if (skill.source !== options.rootInfo.source || skill.scope !== options.rootInfo.scope) {
      throw new Error(`Skill target mismatch for ${skill.name}`);
    }
    assertPathInside(options.rootInfo.root, skill.path);
    return skill;
  }

  private async listSkillsMpSkills(
    options: ListMarketplaceSkillsOptions,
    installedNames: Set<string>,
  ): Promise<MarketplaceSkillEntry[]> {
    const query = options.query?.trim() || options.capability?.trim();
    if (!query) {
      return this.listSkillsMpAlphabetSkills(installedNames);
    }
    const skills = await this.fetchSkillsMpSearch({
      query,
      limit: Math.min(Math.max(options.limit ?? SKILLSMP_SEARCH_MAX_LIMIT, 1), 50),
      installedNames,
    });
    return this.dedupeMarketplaceSkills(skills);
  }

  private async listSkillsMpAlphabetSkills(
    installedNames: Set<string>,
  ): Promise<MarketplaceSkillEntry[]> {
    const batches = await Promise.all(
      SKILLSMP_ALPHABET.map((letter) =>
        this.fetchSkillsMpSearch({
          query: letter,
          limit: SKILLSMP_ALPHABET_FETCH_LIMIT,
          installedNames,
        }),
      ),
    );
    const byLetter = new Map<string, MarketplaceSkillEntry[]>();
    for (const letter of SKILLSMP_ALPHABET) {
      byLetter.set(letter, []);
    }
    for (const skill of this.dedupeMarketplaceSkills(batches.flat())) {
      const initial = marketplaceSkillInitial(skill.name);
      if (!initial) {
        continue;
      }
      const bucket = byLetter.get(initial);
      if (!bucket || bucket.length >= SKILLSMP_ALPHABET_PER_LETTER_LIMIT) {
        continue;
      }
      bucket.push(skill);
    }
    return Array.from(byLetter.values()).flat();
  }

  private async fetchSkillsMpSearch(options: {
    query: string;
    limit: number;
    installedNames: Set<string>;
  }): Promise<MarketplaceSkillEntry[]> {
    const url = new URL("/api/v1/skills/search", SKILLSMP_BASE_URL);
    url.searchParams.set("q", options.query);
    url.searchParams.set("limit", String(Math.min(Math.max(options.limit, 1), 50)));
    url.searchParams.set("sortBy", "stars");

    const response = await fetch(url);
    if (!response.ok) {
      throw new Error(`SkillsMP search failed: ${response.status} ${response.statusText}`);
    }
    const payload = z
      .object({
        success: z.boolean(),
        data: z
          .object({
            skills: z.array(z.unknown()).default([]),
          })
          .optional(),
      })
      .parse(await response.json());
    if (!payload.success) {
      throw new Error("SkillsMP search failed");
    }
    const mapped =
      payload.data?.skills
        .map((raw) => this.mapSkillsMpSkill(raw, options.installedNames))
        .filter((skill): skill is MarketplaceSkillEntry => skill !== null) ?? [];
    return mapped;
  }

  private mapSkillsMpSkill(
    raw: unknown,
    installedNames: Set<string>,
  ): MarketplaceSkillEntry | null {
    const parsed = z
      .object({
        id: z.string(),
        name: z.string(),
        author: z.string(),
        description: z.string(),
        githubUrl: z.string(),
        skillUrl: z.string().optional(),
        stars: z.number().int().nonnegative().default(0),
        updatedAt: z.union([z.number(), z.string()]),
      })
      .safeParse(raw);
    if (!parsed.success) {
      return null;
    }
    const cacheEntry: SkillsMpSkill = {
      id: parsed.data.id,
      name: parsed.data.name,
      author: parsed.data.author,
      description: parsed.data.description,
      githubUrl: parsed.data.githubUrl,
      skillUrl: parsed.data.skillUrl ?? "",
      stars: parsed.data.stars,
      updatedAt: parseUnixTimestampSeconds(parsed.data.updatedAt) ?? 0,
    };
    this.skillsMpSkillCache.set(`${SKILLSMP_SKILL_ID_PREFIX}${parsed.data.id}`, cacheEntry);
    const skillName = safeSkillName(parsed.data.name);
    const trusted = TRUSTED_SKILLSMP_AUTHORS.has(parsed.data.author.toLowerCase());
    return MarketplaceSkillEntrySchema.parse({
      id: `${SKILLSMP_SKILL_ID_PREFIX}${parsed.data.id}`,
      name: parsed.data.name,
      description: parsed.data.description || "SkillsMP skill",
      version: null,
      trustLevel: trusted ? "verified" : "community",
      vettingStatus: trusted ? "trusted-source" : null,
      capabilities: inferSkillCapabilities(parsed.data.description),
      permissions: {
        network: false,
        filesystem: false,
        subprocess: false,
        envVars: [],
      },
      platformCompatibility: ["Claude", "Codex"],
      downloadCount: parsed.data.stars,
      downloads7d: 0,
      daysSinceUpdate: daysSinceUnixTimestampSeconds(parsed.data.updatedAt),
      installed: installedNames.has(skillName),
    });
  }

  private dedupeMarketplaceSkills(skills: MarketplaceSkillEntry[]): MarketplaceSkillEntry[] {
    const byName = new Map<string, MarketplaceSkillEntry>();
    for (const skill of skills) {
      const key = safeSkillName(skill.name);
      const existing = byName.get(key);
      if (!existing) {
        byName.set(key, skill);
        continue;
      }
      if (compareMarketplaceSkillPriority(skill, existing) < 0) {
        byName.set(key, skill);
      }
    }
    return Array.from(byName.values()).sort(compareMarketplaceSkillPriority);
  }

  private async getInstalledWorkspaceSkillNames(options: {
    workspaceId?: string;
    cwd?: string | null;
  }): Promise<Set<string>> {
    const cwd = options.cwd?.trim();
    if (!cwd) {
      return new Set();
    }
    const roots = [path.join(cwd, ".codex", "skills"), path.join(cwd, ".claude", "skills")];
    const providerNames: Array<Set<string>> = [];
    for (const root of roots) {
      const entries = await this.scanSkillRoot(
        {
          root,
          source: "provider_ready",
          scope: "workspace",
          readOnly: true,
          workspaceId: options.workspaceId ?? null,
        },
        false,
      );
      providerNames.push(new Set(entries.map((skill) => safeSkillName(skill.name))));
    }
    const codexNames = providerNames[0] ?? new Set<string>();
    const claudeNames = providerNames[1] ?? new Set<string>();
    return new Set([...codexNames].filter((name) => claudeNames.has(name)));
  }

  private async downloadSkillsMpSkillPackage(
    options: InstallMarketplaceSkillOptions,
  ): Promise<Buffer> {
    const skill = await this.resolveSkillsMpSkill(options.skillId, options.name);
    const rawUrl = parseGithubTreeRawSkillUrl(skill.githubUrl);
    const response = await fetch(rawUrl);
    if (!response.ok) {
      throw new Error(
        `SkillsMP SKILL.md download failed: ${response.status} ${response.statusText}`,
      );
    }
    const arrayBuffer = await response.arrayBuffer();
    if (arrayBuffer.byteLength > SKILL_MARKDOWN_MAX_BYTES) {
      throw new Error("SkillsMP SKILL.md is too large");
    }
    const rawContent = Buffer.from(arrayBuffer).toString("utf8");
    if (!rawContent.trim()) {
      throw new Error("SkillsMP SKILL.md is empty");
    }
    const zip = new AdmZip();
    zip.addFile(
      "SKILL.md",
      Buffer.from(
        ensureInvocableSkillMarkdown({
          name: options.name || skill.name,
          description: skill.description,
          content: rawContent,
        }),
        "utf8",
      ),
    );
    return zip.toBuffer();
  }

  private async resolveSkillsMpSkill(skillId: string, name: string): Promise<SkillsMpSkill> {
    const cached = this.skillsMpSkillCache.get(skillId);
    if (cached) {
      return cached;
    }
    const skillsMpId = skillId.slice(SKILLSMP_SKILL_ID_PREFIX.length);
    const url = new URL("/api/v1/skills/search", SKILLSMP_BASE_URL);
    url.searchParams.set("q", name.trim() || skillsMpId);
    url.searchParams.set("limit", "50");
    url.searchParams.set("sortBy", "stars");
    const response = await fetch(url);
    if (!response.ok) {
      throw new Error(`SkillsMP lookup failed: ${response.status} ${response.statusText}`);
    }
    const payload = z
      .object({
        success: z.boolean(),
        data: z
          .object({
            skills: z.array(z.unknown()).default([]),
          })
          .optional(),
      })
      .parse(await response.json());
    if (!payload.success) {
      throw new Error("SkillsMP lookup failed");
    }
    for (const raw of payload.data?.skills ?? []) {
      const parsed = z
        .object({
          id: z.string(),
          name: z.string(),
          author: z.string(),
          description: z.string(),
          githubUrl: z.string(),
          skillUrl: z.string().optional(),
          stars: z.number().int().nonnegative().default(0),
          updatedAt: z.union([z.number(), z.string()]),
        })
        .safeParse(raw);
      if (!parsed.success || parsed.data.id !== skillsMpId) {
        continue;
      }
      const skill: SkillsMpSkill = {
        id: parsed.data.id,
        name: parsed.data.name,
        author: parsed.data.author,
        description: parsed.data.description,
        githubUrl: parsed.data.githubUrl,
        skillUrl: parsed.data.skillUrl ?? "",
        stars: parsed.data.stars,
        updatedAt: parseUnixTimestampSeconds(parsed.data.updatedAt) ?? 0,
      };
      this.skillsMpSkillCache.set(skillId, skill);
      return skill;
    }
    throw new Error(`SkillsMP skill not found: ${skillId}`);
  }

  private validateSkillZipEntries(zip: AdmZip): NormalizedSkillZipEntry[] {
    const entries = zip.getEntries();
    if (entries.length === 0) {
      throw new Error("Skill package is empty");
    }
    if (entries.length > SKILL_PACKAGE_MAX_FILES) {
      throw new Error(`Skill package has too many files: ${entries.length}`);
    }

    let totalBytes = 0;
    const validated: ValidSkillZipEntry[] = [];
    for (const entry of entries) {
      const entryName = entry.entryName.replace(/\\/g, "/");
      if (
        !entryName ||
        entryName.startsWith("/") ||
        entryName.includes("../") ||
        entryName.startsWith("../") ||
        path.posix.isAbsolute(entryName)
      ) {
        throw new Error(`Unsafe skill package path: ${entry.entryName}`);
      }
      const parts = entryName.split("/").filter(Boolean);
      if (parts.some((part) => part === "." || part === "..")) {
        throw new Error(`Unsafe skill package path: ${entry.entryName}`);
      }
      const isDirectory = entry.isDirectory;
      const size = entry.header.size;
      if (!isDirectory && size > SKILL_PACKAGE_MAX_FILE_BYTES) {
        throw new Error(`Skill package file is too large: ${entryName}`);
      }
      totalBytes += isDirectory ? 0 : size;
      if (totalBytes > SKILL_PACKAGE_MAX_TOTAL_BYTES) {
        throw new Error("Skill package is too large");
      }
      const relativePath = parts.join("/");
      validated.push({
        entryName: entry.entryName,
        relativePath,
        isDirectory,
        size,
      });
    }

    const fileEntries = validated.filter((entry) => !entry.isDirectory);
    const hasRootSkillFile = fileEntries.some((entry) => entry.relativePath === "SKILL.md");
    if (hasRootSkillFile) {
      return validated.map((entry) => ({
        ...entry,
        normalizedRelativePath: entry.relativePath,
      }));
    }

    const topLevelNames = new Set(
      validated
        .map((entry) => entry.relativePath.split("/").filter(Boolean)[0])
        .filter((part): part is string => Boolean(part)),
    );
    if (topLevelNames.size === 1) {
      const [topLevelName] = topLevelNames;
      const prefix = `${topLevelName}/`;
      const normalized = validated
        .map((entry): NormalizedSkillZipEntry | null => {
          if (entry.relativePath === topLevelName) {
            return null;
          }
          if (!entry.relativePath.startsWith(prefix)) {
            throw new Error(`Unsafe skill package path: ${entry.entryName}`);
          }
          return {
            ...entry,
            normalizedRelativePath: entry.relativePath.slice(prefix.length),
          };
        })
        .filter((entry): entry is NormalizedSkillZipEntry =>
          Boolean(entry?.normalizedRelativePath),
        );
      if (normalized.some((entry) => entry.normalizedRelativePath === "SKILL.md")) {
        return normalized;
      }
    }

    throw new Error(
      "Skill package must contain SKILL.md at the root or inside one top-level directory",
    );
  }

  private async extractSkillPackage(buffer: Buffer, targetDir: string): Promise<void> {
    const zip = new AdmZip(buffer);
    const entries = this.validateSkillZipEntries(zip);
    await mkdir(targetDir, { recursive: true });
    for (const entryInfo of entries) {
      if (!entryInfo.normalizedRelativePath) {
        continue;
      }
      const destPath = path.join(targetDir, entryInfo.normalizedRelativePath);
      assertPathInside(targetDir, destPath);
      if (entryInfo.isDirectory) {
        await mkdir(destPath, { recursive: true });
        continue;
      }
      const entry = zip.getEntry(entryInfo.entryName);
      if (!entry) {
        throw new Error(`Missing zip entry after validation: ${entryInfo.entryName}`);
      }
      await mkdir(path.dirname(destPath), { recursive: true });
      await writeFile(destPath, entry.getData());
    }
  }

  private async extractMarketplaceSkillPackage(buffer: Buffer, targetDir: string): Promise<void> {
    await this.extractSkillPackage(buffer, targetDir);
  }

  private async syncWorkspaceSkillProviderDirs(
    workspaceRoot: string,
    skillName: string,
  ): Promise<void> {
    const sourceDir = path.join(workspaceRoot, ".agents", "skills", skillName);
    for (const root of [".codex/skills", ".claude/skills"] as const) {
      const destDir = path.join(workspaceRoot, root, skillName);
      assertPathInside(workspaceRoot, destDir);
      await copyOrSymlinkSkillDirectory(sourceDir, destDir);
      const skillPath = path.join(destDir, "SKILL.md");
      if (!(await exists(skillPath))) {
        throw new Error(`Failed to sync skill into provider directory: ${skillPath}`);
      }
    }
  }

  private async updateWorkspaceSkillExclude(workspaceRoot: string): Promise<void> {
    const gitDir = await this.findGitDir(workspaceRoot);
    if (!gitDir) {
      return;
    }
    const excludePath = path.join(gitDir, "info", "exclude");
    await mkdir(path.dirname(excludePath), { recursive: true });
    const existing = (await readFile(excludePath, "utf8").catch(() => "")).trimEnd();
    const missingEntries = WORKSPACE_SKILL_EXCLUDE_ENTRIES.filter(
      (entry) => !existing.includes(entry),
    );
    if (missingEntries.length === 0) {
      return;
    }
    const prefix = existing ? `${existing}\n` : "";
    await writeFile(excludePath, `${prefix}${missingEntries.join("\n")}\n`, "utf8");
  }

  private async findGitDir(workspaceRoot: string): Promise<string | null> {
    const dotGit = path.join(workspaceRoot, ".git");
    const stats = await lstat(dotGit).catch(() => null);
    if (!stats) {
      return null;
    }
    if (stats.isDirectory()) {
      return dotGit;
    }
    if (stats.isFile()) {
      const content = await readFile(dotGit, "utf8").catch(() => "");
      const match = content.match(/^gitdir:\s*(.+)$/m);
      if (!match) {
        return null;
      }
      const gitDir = path.isAbsolute(match[1]) ? match[1] : path.resolve(workspaceRoot, match[1]);
      return gitDir;
    }
    return null;
  }

  private async findWorkspaceSkill(options: {
    workspaceId: string;
    cwd: string;
    name: string;
  }): Promise<ManagedSkillEntry | null> {
    const skills = await this.listSkills({
      workspaceId: options.workspaceId,
      cwd: options.cwd,
    });
    return (
      skills.find(
        (skill) => skill.scope === "workspace" && safeSkillName(skill.name) === options.name,
      ) ?? null
    );
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
      if (!entry.isDirectory() && !entry.isSymbolicLink()) {
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

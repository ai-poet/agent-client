/**
 * Provider switching for the local coding CLIs.
 *
 * Reads/writes ~/.claude/settings.json, ~/.codex/, ~/.grok/config.toml and ~/.pi/agent/.
 * Custom entries target exactly one CLI; an untargeted legacy row means Claude + Codex.
 */
import { existsSync } from "node:fs";
import { mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import log from "electron-log/main";
import { getDesktopMessage } from "../i18n/desktop-i18n.js";

export const DEFAULT_PROVIDER_ID = "default";
const LEGACY_DEFAULT_PROVIDER_ID = "sub2api-default";
export const DEFAULT_PROVIDER_NAME = "Default";
/** Paseo Cloud–managed rows: one per CLI so keys/endpoints can differ. */
export const PASEO_MANAGED_CLAUDE_PROVIDER_ID = "paseo-managed-claude";
export const PASEO_MANAGED_CODEX_PROVIDER_ID = "paseo-managed-codex";
export const PASEO_MANAGED_GROK_PROVIDER_ID = "paseo-managed-grok";
export const PASEO_MANAGED_PI_PROVIDER_ID = "paseo-managed-pi";

/** `"both"` is the legacy Claude+Codex pairing; every other value is a single CLI. */
export type SetupManagedCloudScope = ManagedProviderTarget | "both";

/** Every CLI whose config this module can write, in the order the UI offers them. */
export const MANAGED_PROVIDER_TARGETS = ["claude", "codex", "grok", "pi"] as const;

export type ManagedProviderTarget = (typeof MANAGED_PROVIDER_TARGETS)[number];

/** Claude Code is written as native Anthropic Messages only; other wire shapes belong in a future gateway layer. */
export type ClaudeApiFormat = "anthropic";

/** Codex config uses OpenAI Responses wire only until chat support is added. */
export type CodexWireApi = "responses";

const CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC_KEY = "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC";
const CLAUDE_CODE_ATTRIBUTION_HEADER_KEY = "CLAUDE_CODE_ATTRIBUTION_HEADER";
const CLAUDE_CODE_GIT_BASH_PATH_KEY = "CLAUDE_CODE_GIT_BASH_PATH";
const DEFAULT_CODEX_MODEL = "gpt-5.4";

export interface StoredProvider {
  id: string;
  name: string;
  type: "default" | "custom";
  endpoint: string;
  apiKey: string;
  isDefault: boolean;
  /** When set, switching applies only to that CLI. Omitted = write both (managed default / legacy). */
  target?: ManagedProviderTarget;
  claudeApiFormat?: ClaudeApiFormat;
  codexWireApi?: CodexWireApi;
  claudeConfig?: Record<string, unknown>;
  codexAuth?: Record<string, unknown>;
  codexConfig?: string;
}

/** @deprecated Prefer `StoredProvider`; kept for daemon IPC typings. */
export type Provider = StoredProvider;

export interface ProviderStore {
  providers: StoredProvider[];
  /** Legacy field: equals both ids when they match; otherwise Claude id for older readers. */
  activeProviderId: string | null;
  activeClaudeProviderId: string | null;
  activeCodexProviderId: string | null;
  /** Optional so stores written before Grok/Pi routing existed still load. */
  activeGrokProviderId?: string | null;
  activePiProviderId?: string | null;
}

const PROVIDERS_FILE = join(homedir(), ".agent-client", "providers.json");

function claudeSettingsPath(): string {
  return join(homedir(), ".claude", "settings.json");
}

function codexAuthPath(): string {
  return join(homedir(), ".codex", "auth.json");
}

function codexConfigPath(): string {
  return join(homedir(), ".codex", "config.toml");
}

/** Grok CLI's own config, created on demand — the CLI ships no default file. */
export function grokConfigPath(): string {
  return join(homedir(), ".grok", "config.toml");
}

/** Context window advertised for gateway-routed Grok models. */
const GROK_CONTEXT_WINDOW = 500_000;
const GROK_REASONING_EFFORTS = ["low", "medium", "high"] as const;

/**
 * Preferences the CLI works better with but which are the user's to own. They are seeded on a
 * first write and never touched again, so re-running a login cannot clobber hand edits.
 */
const GROK_PREFERENCE_SECTIONS: Record<string, Record<string, TomlValue>> = {
  session: { auto_compact_threshold_percent: 85, load_envrc: true },
  memory: { enabled: true },
  "memory.session": { save_on_end: true },
  ui: {
    fork_secondary_model: "grok",
    max_thoughts_width: 120,
    yolo: false,
    compact_mode: false,
    permission_mode: "always-approve",
  },
  subagents: { enabled: true },
};

/**
 * Builds Grok's `~/.grok/config.toml`.
 *
 * Every gateway model gets its own `[model."<id>"]` block keyed by the real model id, so the
 * app's model picker can switch between them; `[models].default` points at the first one.
 * Routing sections are always rewritten, preference sections only seeded when the file is new.
 */
export function buildGrokConfigToml(options: {
  endpoint: string;
  apiKey: string;
  models: string[];
  existingToml?: string | null;
}): string {
  const existing = options.existingToml?.trim() ? options.existingToml : null;
  let toml = existing ?? "";

  toml = upsertTomlSection(toml, "cli", { installer: "internal" });

  for (const model of options.models) {
    toml = upsertTomlSection(toml, `model.${quoteTomlString(model)}`, {
      model,
      name: model,
      base_url: options.endpoint,
      api_key: options.apiKey,
      context_window: GROK_CONTEXT_WINDOW,
      // The gateway serves these models over the OpenAI Responses API.
      api_backend: "responses",
      supports_reasoning_effort: true,
      reasoning_efforts: GROK_REASONING_EFFORTS,
    });
  }

  const defaultModel = options.models[0];
  if (defaultModel) {
    toml = upsertTomlSection(toml, "models", {
      default: defaultModel,
      default_reasoning_effort: "high",
    });
  }

  if (!existing) {
    for (const [section, entries] of Object.entries(GROK_PREFERENCE_SECTIONS)) {
      toml = upsertTomlSection(toml, section, entries);
    }
  }

  return toml.endsWith("\n") ? toml : `${toml}\n`;
}

async function writeGrokSettings(provider: StoredProvider, models: string[]): Promise<void> {
  const configPath = grokConfigPath();
  await mkdir(dirname(configPath), { recursive: true });
  await atomicWriteText(
    configPath,
    buildGrokConfigToml({
      // Saved rows store the bare origin; the gateway speaks OpenAI at /v1, same as Codex.
      endpoint: providerEndpointBaseUrl(provider.endpoint),
      apiKey: provider.apiKey,
      models,
      existingToml: await readFileOrNull(configPath),
    }),
  );
  log.info("[provider-switch] wrote grok config for provider:", provider.name);
}

function piAgentDirPath(): string {
  return join(homedir(), ".pi", "agent");
}

function piModelsPath(): string {
  return join(piAgentDirPath(), "models.json");
}

function piAuthPath(): string {
  return join(piAgentDirPath(), "auth.json");
}

/** The one provider key we own inside Pi's config; everything else in those files is left alone. */
export const PI_MANAGED_PROVIDER_NAME = "paseo-gateway";

/**
 * Merges our gateway into Pi's `models.json`.
 *
 * Unlike Grok we cannot redirect Pi at a managed directory: Pi Direct runs in-process inside
 * the daemon, and `applyProviderEnv` only decorates spawned processes, so `PI_CODING_AGENT_DIR`
 * on the provider env channel would never reach it. We therefore write Pi's real config dir
 * and merge rather than overwrite.
 *
 * Pi is not tied to one model family, so every gateway model is listed — no prefix filtering.
 */
export function buildPiModelsJson(options: {
  endpoint: string;
  models: string[];
  existing?: Record<string, unknown>;
}): Record<string, unknown> {
  const existing = options.existing ?? {};
  const providers = isRecord(existing.providers) ? { ...existing.providers } : {};

  providers[PI_MANAGED_PROVIDER_NAME] = {
    baseUrl: options.endpoint,
    api: "openai-completions",
    models: options.models.map((id) => ({ id, name: id })),
  };

  return { ...existing, providers };
}

/** Pi keeps credentials in `auth.json`, the same file `pi` writes when you log in. */
export function buildPiAuthJson(options: {
  apiKey: string;
  existing?: Record<string, unknown>;
}): Record<string, unknown> {
  return {
    ...(options.existing ?? {}),
    [PI_MANAGED_PROVIDER_NAME]: { type: "api_key", key: options.apiKey },
  };
}

async function writePiSettings(provider: StoredProvider, models: string[]): Promise<void> {
  await mkdir(piAgentDirPath(), { recursive: true });

  const modelsPath = piModelsPath();
  const authPath = piAuthPath();
  const oldModels = await readFileOrNull(modelsPath);
  const nextModels = buildPiModelsJson({
    endpoint: providerEndpointBaseUrl(provider.endpoint),
    models,
    existing: parseJsonObjectForMerge(oldModels, "Pi models.json"),
  });
  const nextAuth = buildPiAuthJson({
    apiKey: provider.apiKey,
    existing: parseJsonObjectForMerge(await readFileOrNull(authPath), "Pi auth.json"),
  });

  await atomicWriteText(modelsPath, JSON.stringify(nextModels, null, 2));
  try {
    await atomicWriteText(authPath, JSON.stringify(nextAuth, null, 2));
  } catch (error) {
    await restoreFile(modelsPath, oldModels);
    throw error;
  }
  log.info("[provider-switch] wrote managed pi config for provider:", provider.name);
}

export function getProviderConfigPaths(): {
  claudeSettingsPath: string;
  codexAuthPath: string;
  codexConfigPath: string;
} {
  return {
    claudeSettingsPath: claudeSettingsPath(),
    codexAuthPath: codexAuthPath(),
    codexConfigPath: codexConfigPath(),
  };
}

export function normalizeProviderEndpoint(endpoint: string): string {
  const trimmed = endpoint.trim().replace(/\/+$/, "");
  if (trimmed.toLowerCase().endsWith("/v1")) {
    return trimmed.slice(0, -3);
  }
  return trimmed;
}

function normalizeProviderType(type: string): "default" | "custom" {
  return type === "custom" ? "custom" : "default";
}

function normalizeClaudeApiFormat(raw: unknown): ClaudeApiFormat | undefined {
  if (raw === "anthropic") {
    return "anthropic";
  }
  // Legacy openai_* values are dropped on load until conversion is implemented app-side.
  return undefined;
}

function normalizeCodexWireApi(raw: unknown): CodexWireApi | undefined {
  if (raw === "responses") {
    return "responses";
  }
  // Legacy "chat" is dropped on load until we implement chat-completions wiring.
  return undefined;
}

function normalizeTarget(raw: unknown): ManagedProviderTarget | undefined {
  if (raw === "claude" || raw === "codex") {
    return raw;
  }
  return undefined;
}

export function normalizeProvider(input: StoredProvider): StoredProvider {
  return {
    ...input,
    type: normalizeProviderType(input.type),
    endpoint: normalizeProviderEndpoint(input.endpoint),
    isDefault: input.id === DEFAULT_PROVIDER_ID ? true : input.isDefault,
    target: normalizeTarget(input.target),
    claudeApiFormat: normalizeClaudeApiFormat(input.claudeApiFormat),
    codexWireApi: normalizeCodexWireApi(input.codexWireApi),
  };
}

type LegacyDualTargetIdMapping = {
  claudeId: string;
  codexId: string;
};

type LegacyDualTargetMigrationResult = {
  providers: StoredProvider[];
  idMappings: Map<string, LegacyDualTargetIdMapping>;
  changed: boolean;
};

type ManagedScopedDedupResult = {
  providers: StoredProvider[];
  idMappings: Map<string, string>;
  changed: boolean;
};

function nextUniqueProviderId(baseId: string, usedIds: Set<string>): string {
  if (!usedIds.has(baseId)) {
    usedIds.add(baseId);
    return baseId;
  }
  let n = 2;
  while (usedIds.has(`${baseId}-${n}`)) {
    n += 1;
  }
  const id = `${baseId}-${n}`;
  usedIds.add(id);
  return id;
}

function scopedProviderName(name: string, scope: ManagedProviderTarget): string {
  const base = name.trim().replace(/\s+\((Claude Code|Codex)\)\s*$/u, "");
  return scope === "claude" ? `${base} (Claude Code)` : `${base} (Codex)`;
}

/**
 * Legacy store rows without `target` behaved as dual-CLI rows.
 * We no longer support one row writing both CLIs, so we split them into scoped rows.
 */
function migrateLegacyDualTargetProviders(
  providers: StoredProvider[],
): LegacyDualTargetMigrationResult {
  const usedIds = new Set<string>();
  const migrated: StoredProvider[] = [];
  const idMappings = new Map<string, LegacyDualTargetIdMapping>();
  let changed = false;

  for (const provider of providers) {
    if (provider.target === "claude" || provider.target === "codex") {
      const id = nextUniqueProviderId(provider.id, usedIds);
      if (id !== provider.id) {
        changed = true;
      }
      migrated.push(id === provider.id ? provider : { ...provider, id });
      continue;
    }

    changed = true;
    const legacyId = provider.id;
    const claudeBaseId =
      legacyId === DEFAULT_PROVIDER_ID ? PASEO_MANAGED_CLAUDE_PROVIDER_ID : `${legacyId}-claude`;
    const codexBaseId =
      legacyId === DEFAULT_PROVIDER_ID ? PASEO_MANAGED_CODEX_PROVIDER_ID : `${legacyId}-codex`;

    const claudeId = nextUniqueProviderId(claudeBaseId, usedIds);
    const codexId = nextUniqueProviderId(codexBaseId, usedIds);
    idMappings.set(legacyId, { claudeId, codexId });

    migrated.push(
      normalizeProvider({
        ...provider,
        id: claudeId,
        name: scopedProviderName(provider.name, "claude"),
        target: "claude",
        claudeApiFormat: provider.claudeApiFormat ?? "anthropic",
      }),
      normalizeProvider({
        ...provider,
        id: codexId,
        name: scopedProviderName(provider.name, "codex"),
        target: "codex",
        codexWireApi: provider.codexWireApi ?? "responses",
      }),
    );
  }

  return {
    providers: migrated,
    idMappings,
    changed,
  };
}

function dedupeManagedScopedProviders(providers: StoredProvider[]): ManagedScopedDedupResult {
  const result: StoredProvider[] = [];
  const idMappings = new Map<string, string>();
  let changed = false;

  const nonManagedProviders = providers.filter(
    (provider) =>
      !(provider.isDefault && (provider.target === "claude" || provider.target === "codex")),
  );
  const nonManagedIds = new Set(nonManagedProviders.map((provider) => provider.id));

  for (const provider of nonManagedProviders) {
    result.push(provider);
  }

  const dedupeScope = (scope: ManagedProviderTarget, canonicalId: string): void => {
    const scoped = providers.filter((provider) => provider.isDefault && provider.target === scope);
    if (scoped.length === 0) {
      return;
    }

    const canonicalExisting = scoped.find((provider) => provider.id === canonicalId) ?? null;
    const winner = canonicalExisting ?? scoped[0]!;
    let winnerId = winner.id;
    let winnerOut = winner;

    if (winner.id !== canonicalId && !nonManagedIds.has(canonicalId)) {
      winnerId = canonicalId;
      winnerOut = normalizeProvider({
        ...winner,
        id: canonicalId,
      });
      idMappings.set(winner.id, canonicalId);
      changed = true;
    }

    result.push(winnerOut);

    for (const provider of scoped) {
      if (provider.id === winner.id) {
        continue;
      }
      idMappings.set(provider.id, winnerId);
      changed = true;
    }
  };

  dedupeScope("claude", PASEO_MANAGED_CLAUDE_PROVIDER_ID);
  dedupeScope("codex", PASEO_MANAGED_CODEX_PROVIDER_ID);

  return {
    providers: result,
    idMappings,
    changed,
  };
}

function providerEndpointBaseUrl(endpoint: string): string {
  const normalized = normalizeProviderEndpoint(endpoint);
  if (!normalized) {
    return `${endpoint.replace(/\/+$/, "")}/v1`;
  }
  return `${normalized}/v1`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

async function ensureParentDir(filePath: string): Promise<void> {
  await mkdir(dirname(filePath), { recursive: true });
}

async function atomicWriteText(filePath: string, content: string): Promise<void> {
  await ensureParentDir(filePath);
  const tempPath = `${filePath}.tmp-${process.pid}-${Date.now()}`;
  await writeFile(tempPath, content, "utf-8");
  if (process.platform === "win32" && existsSync(filePath)) {
    await rm(filePath, { force: true });
  }
  await rename(tempPath, filePath);
}

async function readFileOrNull(filePath: string): Promise<string | null> {
  try {
    return await readFile(filePath, "utf-8");
  } catch {
    return null;
  }
}

function parseJsonObjectForMerge(raw: string | null, label: string): Record<string, unknown> {
  if (raw === null || raw.trim().length === 0) {
    return {};
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    throw new Error(`${label} contains invalid JSON and was not overwritten.`);
  }
  if (!isRecord(parsed)) {
    throw new Error(`${label} must contain a JSON object and was not overwritten.`);
  }
  return parsed;
}

function deepMergeRecords(
  base: Record<string, unknown>,
  overrides: Record<string, unknown>,
): Record<string, unknown> {
  const merged: Record<string, unknown> = { ...base };
  for (const [key, value] of Object.entries(overrides)) {
    const current = merged[key];
    merged[key] = isRecord(current) && isRecord(value) ? deepMergeRecords(current, value) : value;
  }
  return merged;
}

async function restoreFile(filePath: string, contents: string | null): Promise<void> {
  if (contents === null) {
    await rm(filePath, { force: true });
    return;
  }
  await atomicWriteText(filePath, contents);
}

function providerNeedsReNormalize(original: unknown, normalized: StoredProvider): boolean {
  if (!isRecord(original)) {
    return true;
  }
  return (
    normalizeProviderEndpoint(String(original.endpoint ?? "")) !== normalized.endpoint ||
    normalizeProviderType(String(original.type ?? "")) !== normalized.type ||
    Boolean(original.isDefault) !== normalized.isDefault ||
    normalizeTarget(original.target) !== normalized.target ||
    normalizeClaudeApiFormat(original.claudeApiFormat) !== normalized.claudeApiFormat ||
    normalizeCodexWireApi(original.codexWireApi) !== normalized.codexWireApi
  );
}

function normalizeUrlForProviderMatch(url: string): string {
  return normalizeProviderEndpoint(url).replace(/\/+$/, "").toLowerCase();
}

/** Codex may store base_url with or without a trailing /v1; we accept either vs our saved row. */
function codexDiskBaseUrlMatchKeys(diskRaw: string): Set<string> {
  const trimmed = diskRaw.trim().replace(/\/+$/, "");
  const keys = new Set<string>();
  keys.add(normalizeUrlForProviderMatch(trimmed));
  if (!trimmed.toLowerCase().endsWith("/v1")) {
    keys.add(normalizeUrlForProviderMatch(`${trimmed}/v1`));
  }
  return keys;
}

function listCodexModelProviderSectionNames(toml: string): string[] {
  const names: string[] = [];
  const re = /^\[model_providers\.([^\]]+)\]\s*$/gm;
  let m: RegExpExecArray | null;
  while ((m = re.exec(toml)) !== null) {
    names.push(m[1]);
  }
  return names;
}

function extractCodexModelProviderKey(toml: string): string | null {
  const line =
    /^\s*model_provider\s*=\s*"([^"]+)"/m.exec(toml) ??
    /^\s*model_provider\s*=\s*'([^']+)'/m.exec(toml) ??
    /^\s*model_provider\s*=\s*([A-Za-z0-9_-]+)/m.exec(toml);
  return line ? line[1] : null;
}

function extractCodexBaseUrlForSection(toml: string, sectionName: string): string | null {
  const header = `[model_providers.${sectionName}]`;
  let inSection = false;
  for (const line of toml.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (trimmed.startsWith("[") && trimmed.endsWith("]")) {
      if (inSection) {
        break;
      }
      if (trimmed === header) {
        inSection = true;
        continue;
      }
      inSection = false;
      continue;
    }
    if (!inSection) {
      continue;
    }
    const quoted =
      /^\s*base_url\s*=\s*"([^"]*)"/.exec(line) ?? /^\s*base_url\s*=\s*'([^']*)'/.exec(line);
    if (quoted) {
      return quoted[1].trim();
    }
    const bare = /^\s*base_url\s*=\s*(\S+)/.exec(line);
    if (bare) {
      return bare[1].trim();
    }
  }
  return null;
}

function parseCodexAuthOpenAiKey(authJsonText: string): string | null {
  try {
    const parsed: unknown = JSON.parse(authJsonText);
    if (!isRecord(parsed)) {
      return null;
    }
    return readString(parsed.OPENAI_API_KEY);
  } catch {
    return null;
  }
}

function resolveCodexDiskBaseUrl(configToml: string): string | null {
  const named = extractCodexModelProviderKey(configToml);
  if (named) {
    const fromNamed = extractCodexBaseUrlForSection(configToml, named);
    if (fromNamed) {
      return fromNamed;
    }
  }
  const sections = listCodexModelProviderSectionNames(configToml);
  if (sections.length === 1) {
    return extractCodexBaseUrlForSection(configToml, sections[0]!);
  }
  return null;
}

/**
 * Match ~/.codex files to a saved provider row. Returns undefined when files are missing or
 * unreadable so callers keep JSON state; null when disk is readable but matches nothing.
 */
export function resolveActiveCodexIdFromDiskState(
  authJsonText: string,
  configTomlText: string,
  providers: StoredProvider[],
  preferId: string | null,
): string | null | undefined {
  const apiKey = parseCodexAuthOpenAiKey(authJsonText);
  const baseUrl = resolveCodexDiskBaseUrl(configTomlText);
  if (!apiKey || !baseUrl) {
    return undefined;
  }
  const diskKeys = codexDiskBaseUrlMatchKeys(baseUrl);

  const candidates = providers.filter(shouldWriteCodex);
  const matches = candidates.filter((p) => {
    const expected = normalizeUrlForProviderMatch(providerEndpointBaseUrl(p.endpoint));
    return diskKeys.has(expected) && p.apiKey.trim() === apiKey;
  });
  if (matches.length === 0) {
    return null;
  }
  if (matches.length === 1) {
    return matches[0]!.id;
  }
  if (preferId && matches.some((m) => m.id === preferId)) {
    return preferId;
  }
  return matches[0]!.id;
}

async function inferActiveClaudeProviderIdFromDisk(
  providers: StoredProvider[],
  preferId: string | null,
): Promise<string | null | undefined> {
  const config = await readClaudeDiskConfig();
  if (!config) {
    return undefined;
  }
  const diskEp = normalizeProviderEndpoint(config.baseUrl);

  const candidates = providers.filter(shouldWriteClaude);
  const matches = candidates.filter(
    (p) =>
      normalizeProviderEndpoint(p.endpoint) === diskEp && p.apiKey.trim() === config.apiKey.trim(),
  );
  if (matches.length === 0) {
    return null;
  }
  if (matches.length === 1) {
    return matches[0]!.id;
  }
  if (preferId && matches.some((m) => m.id === preferId)) {
    return preferId;
  }
  return matches[0]!.id;
}

/** Read Claude Code CLI config from disk. Returns null if missing or invalid. */
async function readClaudeDiskConfig(): Promise<{ baseUrl: string; apiKey: string } | null> {
  const raw = await readFileOrNull(claudeSettingsPath());
  if (!raw?.trim()) {
    return null;
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!isRecord(parsed)) {
    return null;
  }
  const env = isRecord(parsed.env) ? parsed.env : {};
  const baseUrl = readString(env.ANTHROPIC_BASE_URL);
  const apiKey = readString(env.ANTHROPIC_AUTH_TOKEN);
  if (!baseUrl || !apiKey) {
    return null;
  }
  return { baseUrl, apiKey };
}

async function inferActiveCodexProviderIdFromDisk(
  providers: StoredProvider[],
  preferId: string | null,
): Promise<string | null | undefined> {
  const config = await readCodexDiskConfig();
  if (!config) {
    return undefined;
  }
  return resolveActiveCodexIdFromDiskState(
    JSON.stringify({ OPENAI_API_KEY: config.apiKey }),
    config.configToml,
    providers,
    preferId,
  );
}

/** Read Codex CLI config from disk. Returns null if missing or invalid. */
async function readCodexDiskConfig(): Promise<{ apiKey: string; configToml: string } | null> {
  const authRaw = await readFileOrNull(codexAuthPath());
  const configRaw = await readFileOrNull(codexConfigPath());
  if (!authRaw?.trim() || !configRaw?.trim()) {
    return null;
  }
  const apiKey = parseCodexAuthOpenAiKey(authRaw);
  if (!apiKey) {
    return null;
  }
  return { apiKey, configToml: configRaw };
}

/** Build a BYOK provider from local Claude Code CLI config. */
export function buildByokClaudeProviderFromDisk(config: {
  baseUrl: string;
  apiKey: string;
}): StoredProvider {
  return normalizeProvider({
    id: "byok-local-claude",
    name: "Local Claude Code",
    type: "custom",
    endpoint: config.baseUrl,
    apiKey: config.apiKey,
    isDefault: false,
    target: "claude",
    claudeApiFormat: "anthropic",
  });
}

/** Build a BYOK provider from local Codex CLI config. */
export function buildByokCodexProviderFromDisk(config: {
  apiKey: string;
  configToml: string;
}): StoredProvider {
  const baseUrl = resolveCodexDiskBaseUrl(config.configToml);
  return normalizeProvider({
    id: "byok-local-codex",
    name: "Local Codex",
    type: "custom",
    endpoint: baseUrl ?? "",
    apiKey: config.apiKey,
    isDefault: false,
    target: "codex",
    codexWireApi: "responses",
  });
}

/** Import providers from local CLI configs when store is empty. */
async function importProvidersFromDisk(): Promise<{
  providers: StoredProvider[];
  activeClaudeProviderId: string | null;
  activeCodexProviderId: string | null;
}> {
  const providers: StoredProvider[] = [];
  let activeClaudeProviderId: string | null = null;
  let activeCodexProviderId: string | null = null;

  const claudeConfig = await readClaudeDiskConfig();
  if (claudeConfig) {
    const provider = buildByokClaudeProviderFromDisk(claudeConfig);
    providers.push(provider);
    activeClaudeProviderId = provider.id;
  }

  const codexConfig = await readCodexDiskConfig();
  if (codexConfig) {
    const provider = buildByokCodexProviderFromDisk(codexConfig);
    providers.push(provider);
    activeCodexProviderId = provider.id;
  }

  return { providers, activeClaudeProviderId, activeCodexProviderId };
}

async function loadStore(): Promise<ProviderStore> {
  try {
    if (!existsSync(PROVIDERS_FILE)) {
      const imported = await importProvidersFromDisk();
      if (imported.providers.length > 0) {
        const store: ProviderStore = {
          providers: imported.providers,
          activeProviderId:
            imported.activeClaudeProviderId !== null &&
            imported.activeClaudeProviderId === imported.activeCodexProviderId
              ? imported.activeClaudeProviderId
              : (imported.activeClaudeProviderId ?? imported.activeCodexProviderId ?? null),
          activeClaudeProviderId: imported.activeClaudeProviderId,
          activeCodexProviderId: imported.activeCodexProviderId,
        };
        await saveStore(store);
        log.info("[provider-switch] auto-imported providers from local CLI configs");
        return store;
      }
      return {
        providers: [],
        activeProviderId: null,
        activeClaudeProviderId: null,
        activeCodexProviderId: null,
      };
    }
    const raw = await readFile(PROVIDERS_FILE, "utf-8");
    const parsed: unknown = JSON.parse(raw);
    const parsedRecord = isRecord(parsed) ? parsed : {};
    const providersInput = Array.isArray(parsedRecord.providers) ? parsedRecord.providers : [];

    // If providers.json exists but has no providers, try importing from local CLI configs
    if (providersInput.length === 0) {
      const imported = await importProvidersFromDisk();
      if (imported.providers.length > 0) {
        const store: ProviderStore = {
          providers: imported.providers,
          activeProviderId:
            imported.activeClaudeProviderId !== null &&
            imported.activeClaudeProviderId === imported.activeCodexProviderId
              ? imported.activeClaudeProviderId
              : (imported.activeClaudeProviderId ?? imported.activeCodexProviderId ?? null),
          activeClaudeProviderId: imported.activeClaudeProviderId,
          activeCodexProviderId: imported.activeCodexProviderId,
        };
        await saveStore(store);
        log.info("[provider-switch] auto-imported providers from local CLI configs (empty store)");
        return store;
      }
    }

    const normalizedProviders = providersInput.map((p) => normalizeProvider(p as StoredProvider));
    const migration = migrateLegacyDualTargetProviders(normalizedProviders);
    const deduped = dedupeManagedScopedProviders(migration.providers);
    const providers = deduped.providers;

    const mapManagedScopedId = (id: string): string => deduped.idMappings.get(id) ?? id;

    const mapLegacyIdForScope = (id: string, scope: ManagedProviderTarget): string => {
      const mapping = migration.idMappings.get(id);
      const migratedId = mapping ? (scope === "claude" ? mapping.claudeId : mapping.codexId) : id;
      return mapManagedScopedId(migratedId);
    };

    const legacyActiveRaw =
      typeof parsedRecord.activeProviderId === "string" ? parsedRecord.activeProviderId : null;
    const legacyActiveMapping = legacyActiveRaw
      ? (migration.idMappings.get(legacyActiveRaw) ?? null)
      : null;
    const legacyActive =
      legacyActiveRaw !== null
        ? (() => {
            const mapped = mapLegacyIdForScope(legacyActiveRaw, "claude");
            return providers.some((provider) => provider.id === mapped) ? mapped : null;
          })()
        : null;

    const readScopedActive = (key: string, scope: ManagedProviderTarget): string | null => {
      const rawId = parsedRecord[key];
      if (typeof rawId !== "string") {
        return null;
      }
      const mappedId = mapLegacyIdForScope(rawId, scope);
      if (!providers.some((p) => p.id === mappedId)) {
        return null;
      }
      return mappedId;
    };

    const scopedClaudeActive = readScopedActive("activeClaudeProviderId", "claude");
    const scopedCodexActive = readScopedActive("activeCodexProviderId", "codex");
    const hasScopedActive = scopedClaudeActive !== null || scopedCodexActive !== null;
    let activeClaudeProviderId: string | null;
    let activeCodexProviderId: string | null;
    if (hasScopedActive) {
      activeClaudeProviderId = scopedClaudeActive;
      activeCodexProviderId = scopedCodexActive;
    } else if (legacyActiveMapping !== null) {
      activeClaudeProviderId = mapManagedScopedId(legacyActiveMapping.claudeId);
      activeCodexProviderId = mapManagedScopedId(legacyActiveMapping.codexId);
    } else {
      activeClaudeProviderId = legacyActive;
      activeCodexProviderId = legacyActive;
    }

    const diskClaudeId = await inferActiveClaudeProviderIdFromDisk(
      providers,
      activeClaudeProviderId,
    );
    if (diskClaudeId !== undefined) {
      activeClaudeProviderId = diskClaudeId;
    }
    const diskCodexId = await inferActiveCodexProviderIdFromDisk(providers, activeCodexProviderId);
    if (diskCodexId !== undefined) {
      activeCodexProviderId = diskCodexId;
    }

    const activeProviderId =
      activeClaudeProviderId !== null &&
      activeCodexProviderId !== null &&
      activeClaudeProviderId === activeCodexProviderId
        ? activeClaudeProviderId
        : (activeClaudeProviderId ?? activeCodexProviderId ?? null);

    const normalizedStore: ProviderStore = {
      providers,
      activeProviderId,
      activeClaudeProviderId,
      activeCodexProviderId,
    };
    const shouldPersistNormalizedStore =
      migration.changed ||
      deduped.changed ||
      !Array.isArray(parsedRecord.providers) ||
      parsedRecord.activeProviderId !== activeProviderId ||
      parsedRecord.activeClaudeProviderId !== activeClaudeProviderId ||
      parsedRecord.activeCodexProviderId !== activeCodexProviderId ||
      providers.length !== providersInput.length ||
      providers.some((provider, index) => {
        const original = providersInput[index];
        return providerNeedsReNormalize(original, provider);
      });
    if (shouldPersistNormalizedStore) {
      await saveStore(normalizedStore);
    }
    return normalizedStore;
  } catch {
    return {
      providers: [],
      activeProviderId: null,
      activeClaudeProviderId: null,
      activeCodexProviderId: null,
    };
  }
}

async function saveStore(store: ProviderStore): Promise<void> {
  await ensureParentDir(PROVIDERS_FILE);
  await atomicWriteText(PROVIDERS_FILE, JSON.stringify(store, null, 2));
}

function readString(value: unknown): string | null {
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

export function buildClaudeSettings(
  provider: StoredProvider,
  existing: Record<string, unknown>,
): Record<string, unknown> {
  const shouldUseMinimalClaudeEnv =
    provider.id === PASEO_MANAGED_CLAUDE_PROVIDER_ID ||
    (provider.isDefault && (provider.target === undefined || provider.target === "claude"));
  const existingEnv = isRecord(existing.env) ? existing.env : {};

  if (shouldUseMinimalClaudeEnv) {
    return {
      ...existing,
      env: {
        ...existingEnv,
        ANTHROPIC_BASE_URL: normalizeProviderEndpoint(provider.endpoint),
        ANTHROPIC_AUTH_TOKEN: provider.apiKey,
        [CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC_KEY]: "1",
        [CLAUDE_CODE_ATTRIBUTION_HEADER_KEY]: "0",
      },
    };
  }

  const providerConfig = isRecord(provider.claudeConfig) ? provider.claudeConfig : {};
  const mergedConfig = deepMergeRecords(existing, providerConfig);
  const mergedEnv = isRecord(mergedConfig.env) ? mergedConfig.env : {};

  const env: Record<string, unknown> = {
    ...mergedEnv,
    ANTHROPIC_BASE_URL: normalizeProviderEndpoint(provider.endpoint),
    ANTHROPIC_AUTH_TOKEN: provider.apiKey,
    [CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC_KEY]: "1",
    [CLAUDE_CODE_ATTRIBUTION_HEADER_KEY]: "0",
  };

  return {
    ...mergedConfig,
    env,
  };
}

export function buildCodexAuth(
  provider: StoredProvider,
  existing: Record<string, unknown> = {},
): Record<string, unknown> {
  const desiredAuth = isRecord(provider.codexAuth)
    ? provider.codexAuth
    : { OPENAI_API_KEY: provider.apiKey };
  return deepMergeRecords(existing, desiredAuth);
}

function buildDefaultCodexConfig(provider: StoredProvider): string {
  if (provider.codexConfig && provider.id !== PASEO_MANAGED_CODEX_PROVIDER_ID) {
    return provider.codexConfig;
  }
  return `model_provider = "OpenAI"
model = "${DEFAULT_CODEX_MODEL}"
review_model = "${DEFAULT_CODEX_MODEL}"
model_reasoning_effort = "xhigh"
disable_response_storage = true
network_access = "enabled"
windows_wsl_setup_acknowledged = true
model_context_window = 1000000
model_auto_compact_token_limit = 900000

[model_providers.OpenAI]
name = "OpenAI"
base_url = "${providerEndpointBaseUrl(provider.endpoint)}"
wire_api = "responses"
requires_openai_auth = true
`;
}

function quoteTomlString(value: string): string {
  return JSON.stringify(value);
}

/** Values we know how to emit. Arrays stay single-line, which is all our configs need. */
type TomlValue = string | boolean | number | readonly string[];

function formatTomlValue(value: TomlValue): string {
  if (typeof value === "boolean" || typeof value === "number") {
    return String(value);
  }
  if (Array.isArray(value)) {
    return `[${value.map(quoteTomlString).join(", ")}]`;
  }
  return quoteTomlString(value as string);
}

/**
 * A bare TOML key may only contain letters, digits, underscores and dashes. Anything else —
 * a dot inside a model id like `grok-4.6`, for instance — has to be quoted, or the dot would
 * silently turn one table into two nested ones.
 */
function renderTomlKeyPart(part: string): string {
  return /^[A-Za-z0-9_-]+$/u.test(part) ? part : quoteTomlString(part);
}

function splitTomlDottedName(raw: string): string[] | null {
  const parts: string[] = [];
  let i = 0;
  while (i < raw.length) {
    while (/\s/u.test(raw[i] ?? "")) {
      i += 1;
    }
    if (i >= raw.length) {
      break;
    }

    let part = "";
    const quote = raw[i];
    if (quote === '"' || quote === "'") {
      i += 1;
      while (i < raw.length) {
        const ch = raw[i]!;
        if (quote === '"' && ch === "\\") {
          part += ch;
          i += 1;
          if (i < raw.length) {
            part += raw[i]!;
            i += 1;
          }
          continue;
        }
        if (ch === quote) {
          i += 1;
          break;
        }
        part += ch;
        i += 1;
      }
    } else {
      while (i < raw.length && raw[i] !== ".") {
        part += raw[i]!;
        i += 1;
      }
      part = part.trim();
    }
    if (!part) {
      return null;
    }
    parts.push(part);

    while (/\s/u.test(raw[i] ?? "")) {
      i += 1;
    }
    if (i >= raw.length) {
      break;
    }
    if (raw[i] !== ".") {
      return null;
    }
    i += 1;
  }
  return parts.length > 0 ? parts : null;
}

function normalizeTomlDottedName(raw: string): string | null {
  const parts = splitTomlDottedName(raw);
  return parts ? parts.join(".") : null;
}

function parseTomlHeader(line: string): { name: string; isArray: boolean } | null {
  const arrayMatch = /^\s*\[\[\s*(.+?)\s*\]\]\s*(?:#.*)?$/u.exec(line);
  if (arrayMatch) {
    const name = normalizeTomlDottedName(arrayMatch[1]!);
    return name ? { name, isArray: true } : null;
  }
  const match = /^\s*\[\s*(.+?)\s*\]\s*(?:#.*)?$/u.exec(line);
  if (!match) {
    return null;
  }
  const name = normalizeTomlDottedName(match[1]!);
  return name ? { name, isArray: false } : null;
}

function findTomlAssignmentKey(line: string): string | null {
  const trimmed = line.trim();
  if (!trimmed || trimmed.startsWith("#") || trimmed.startsWith("[")) {
    return null;
  }

  let quote: string | null = null;
  for (let i = 0; i < line.length; i += 1) {
    const ch = line[i]!;
    if (quote !== null) {
      if (quote === '"' && ch === "\\") {
        i += 1;
        continue;
      }
      if (ch === quote) {
        quote = null;
      }
      continue;
    }
    if (ch === '"' || ch === "'") {
      quote = ch;
      continue;
    }
    if (ch === "=") {
      return normalizeTomlDottedName(line.slice(0, i).trim());
    }
  }
  return null;
}

function formatTomlAssignment(key: string, value: TomlValue): string {
  return `${key} = ${formatTomlValue(value)}`;
}

function upsertTopLevelTomlKey(toml: string, key: string, value: TomlValue): string {
  const lines = toml.split(/\r?\n/u);
  const matches: number[] = [];
  let firstSectionIndex = lines.length;

  for (let i = 0; i < lines.length; i += 1) {
    if (parseTomlHeader(lines[i]!)) {
      firstSectionIndex = i;
      break;
    }
    if (findTomlAssignmentKey(lines[i]!) === key) {
      matches.push(i);
    }
  }

  if (matches.length > 1) {
    throw new Error(`Codex config has duplicate top-level ${key} entries.`);
  }

  const assignment = formatTomlAssignment(key, value);
  if (matches.length === 1) {
    lines[matches[0]!] = assignment;
    return lines.join("\n");
  }

  let insertIndex = firstSectionIndex;
  while (insertIndex > 0 && lines[insertIndex - 1]!.trim() === "") {
    insertIndex -= 1;
  }
  lines.splice(insertIndex, 0, assignment);
  return lines.join("\n");
}

/**
 * @param sectionName Dotted name, quoted where a part is not a bare key (`model."grok-4.6"`).
 *   Matching is done on the normalized form, so an existing section is found regardless of how
 *   it was quoted on disk; the header we write back is re-quoted as needed.
 */
function upsertTomlSection(
  toml: string,
  sectionName: string,
  entries: Record<string, TomlValue>,
): string {
  const sectionParts = splitTomlDottedName(sectionName) ?? [sectionName];
  const canonicalName = sectionParts.join(".");
  const renderedName = sectionParts.map(renderTomlKeyPart).join(".");
  const lines = toml.split(/\r?\n/u);
  const sectionIndexes: number[] = [];

  for (let i = 0; i < lines.length; i += 1) {
    const header = parseTomlHeader(lines[i]!);
    if (header?.name === canonicalName) {
      if (header.isArray) {
        throw new Error(`Config section [[${renderedName}]] cannot be safely patched.`);
      }
      sectionIndexes.push(i);
    }
  }

  if (sectionIndexes.length > 1) {
    throw new Error(`Config has duplicate [${renderedName}] sections.`);
  }

  const assignmentLines = Object.entries(entries).map(([key, value]) =>
    formatTomlAssignment(key, value),
  );

  if (sectionIndexes.length === 0) {
    while (lines.length > 0 && lines[lines.length - 1]!.trim() === "") {
      lines.pop();
    }
    if (lines.length > 0) {
      lines.push("");
    }
    lines.push(`[${renderedName}]`, ...assignmentLines);
    return `${lines.join("\n")}\n`;
  }

  const sectionStart = sectionIndexes[0]!;
  let sectionEnd = lines.length;
  for (let i = sectionStart + 1; i < lines.length; i += 1) {
    if (parseTomlHeader(lines[i]!)) {
      sectionEnd = i;
      break;
    }
  }

  const existingEntryIndexes = new Map<string, number>();
  for (let i = sectionStart + 1; i < sectionEnd; i += 1) {
    const key = findTomlAssignmentKey(lines[i]!);
    if (!key || !(key in entries)) {
      continue;
    }
    if (existingEntryIndexes.has(key)) {
      throw new Error(`Codex config has duplicate ${key} entries in [${sectionName}].`);
    }
    existingEntryIndexes.set(key, i);
  }

  const missingLines: string[] = [];
  for (const [key, value] of Object.entries(entries)) {
    const assignment = formatTomlAssignment(key, value);
    const existingIndex = existingEntryIndexes.get(key);
    if (existingIndex === undefined) {
      missingLines.push(assignment);
    } else {
      lines[existingIndex] = assignment;
    }
  }

  if (missingLines.length > 0) {
    let insertIndex = sectionEnd;
    while (insertIndex > sectionStart + 1 && lines[insertIndex - 1]!.trim() === "") {
      insertIndex -= 1;
    }
    lines.splice(insertIndex, 0, ...missingLines);
  }

  return lines.join("\n");
}

function patchCodexConfigToml(provider: StoredProvider, existingToml: string): string {
  const withProvider = upsertTopLevelTomlKey(existingToml, "model_provider", "OpenAI");
  const patched = upsertTomlSection(withProvider, "model_providers.OpenAI", {
    name: "OpenAI",
    base_url: providerEndpointBaseUrl(provider.endpoint),
    wire_api: "responses",
    requires_openai_auth: true,
  });
  return patched.endsWith("\n") ? patched : `${patched}\n`;
}

export function buildCodexConfig(provider: StoredProvider, existingToml?: string | null): string {
  if (existingToml?.trim()) {
    return patchCodexConfigToml(provider, existingToml);
  }
  return buildDefaultCodexConfig(provider);
}

async function writeClaudeSettings(provider: StoredProvider): Promise<void> {
  const existing = parseJsonObjectForMerge(
    await readFileOrNull(claudeSettingsPath()),
    "Claude Code settings.json",
  );
  const merged = buildClaudeSettings(provider, existing);
  await atomicWriteText(claudeSettingsPath(), JSON.stringify(merged, null, 2));
  log.info("[provider-switch] wrote claude settings for provider:", provider.name);
}

export async function patchClaudeCodeGitBashPathForWindows(
  gitBashPath: string | null,
  options: { platform?: NodeJS.Platform } = {},
): Promise<void> {
  const platform = options.platform ?? process.platform;
  if (platform !== "win32") {
    return;
  }
  const trimmed = readString(gitBashPath);
  if (!trimmed) {
    return;
  }
  const existing = parseJsonObjectForMerge(
    await readFileOrNull(claudeSettingsPath()),
    "Claude Code settings.json",
  );
  const existingEnv = isRecord(existing.env) ? existing.env : {};
  const next = {
    ...existing,
    env: {
      ...existingEnv,
      [CLAUDE_CODE_GIT_BASH_PATH_KEY]: trimmed,
    },
  };
  await atomicWriteText(claudeSettingsPath(), JSON.stringify(next, null, 2));
  log.info("[provider-switch] patched Claude Code Git Bash path");
}

async function writeCodexSettings(provider: StoredProvider): Promise<void> {
  const authPath = codexAuthPath();
  const configPath = codexConfigPath();
  const oldAuth = await readFileOrNull(authPath);
  const existingAuth = parseJsonObjectForMerge(oldAuth, "Codex auth.json");
  const nextAuth = JSON.stringify(buildCodexAuth(provider, existingAuth), null, 2);
  const nextConfig = buildCodexConfig(provider, await readFileOrNull(configPath));
  await atomicWriteText(authPath, nextAuth);
  try {
    await atomicWriteText(configPath, nextConfig);
  } catch (error) {
    await restoreFile(authPath, oldAuth);
    throw error;
  }
  log.info("[provider-switch] wrote codex settings for provider:", provider.name);
}

export interface ConfigBackup {
  timestamp: number;
  claudeSettings: string | null;
  codexAuth: string | null;
  codexConfig: string | null;
  /** Optional so a backup taken by an older build still restores what it did capture. */
  grokConfig?: string | null;
  piModels?: string | null;
  piAuth?: string | null;
}

export async function backupCurrentConfig(): Promise<ConfigBackup> {
  return {
    timestamp: Date.now(),
    claudeSettings: await readFileOrNull(claudeSettingsPath()),
    codexAuth: await readFileOrNull(codexAuthPath()),
    codexConfig: await readFileOrNull(codexConfigPath()),
    grokConfig: await readFileOrNull(grokConfigPath()),
    piModels: await readFileOrNull(piModelsPath()),
    piAuth: await readFileOrNull(piAuthPath()),
  };
}

export async function restoreConfig(backup: ConfigBackup): Promise<void> {
  await restoreFile(claudeSettingsPath(), backup.claudeSettings);
  await restoreFile(codexAuthPath(), backup.codexAuth);
  await restoreFile(codexConfigPath(), backup.codexConfig);
  // Only restore what the backup actually captured; `undefined` means "not recorded".
  if (backup.grokConfig !== undefined) {
    await restoreFile(grokConfigPath(), backup.grokConfig);
  }
  if (backup.piModels !== undefined) {
    await restoreFile(piModelsPath(), backup.piModels);
  }
  if (backup.piAuth !== undefined) {
    await restoreFile(piAuthPath(), backup.piAuth);
  }
  log.info("[provider-switch] restored config from backup at", backup.timestamp);
}

function shouldWriteClaude(provider: StoredProvider): boolean {
  return provider.target === undefined || provider.target === "claude";
}

function shouldWriteCodex(provider: StoredProvider): boolean {
  return provider.target === undefined || provider.target === "codex";
}

/** Grok is opt-in only: an untargeted endpoint never rewrites its config. */
function shouldWriteGrok(provider: StoredProvider): boolean {
  return provider.target === "grok";
}

/** Pi is opt-in only, for the same reason as Grok. */
function shouldWritePi(provider: StoredProvider): boolean {
  return provider.target === "pi";
}

export async function getProviders(): Promise<ProviderStore> {
  return loadStore();
}

export async function addProvider(provider: StoredProvider): Promise<void> {
  const store = await loadStore();
  const normalizedProvider = normalizeProvider(provider);
  const idx = store.providers.findIndex((entry) => entry.id === normalizedProvider.id);
  if (idx >= 0) {
    store.providers[idx] = normalizedProvider;
  } else {
    store.providers.push(normalizedProvider);
  }
  await saveStore(store);
}

function syncLegacyActiveProviderId(store: ProviderStore): void {
  const c = store.activeClaudeProviderId;
  const x = store.activeCodexProviderId;
  store.activeProviderId = c !== null && c === x ? c : (c ?? x ?? null);
}

export async function removeProvider(id: string): Promise<void> {
  const store = await loadStore();
  store.providers = store.providers.filter((provider) => provider.id !== id);
  if (store.activeProviderId === id) {
    store.activeProviderId = null;
  }
  if (store.activeClaudeProviderId === id) {
    store.activeClaudeProviderId = null;
  }
  if (store.activeCodexProviderId === id) {
    store.activeCodexProviderId = null;
  }
  syncLegacyActiveProviderId(store);
  await saveStore(store);
}

const TARGET_WRITE_RULES: Record<
  ManagedProviderTarget,
  { supports: (provider: StoredProvider) => boolean; unsupportedMessage: () => string }
> = {
  claude: {
    supports: shouldWriteClaude,
    unsupportedMessage: () => getDesktopMessage("provider.notForClaude"),
  },
  codex: {
    supports: shouldWriteCodex,
    unsupportedMessage: () => getDesktopMessage("provider.notForCodex"),
  },
  grok: {
    supports: shouldWriteGrok,
    unsupportedMessage: () => getDesktopMessage("provider.notForGrok"),
  },
  pi: {
    supports: shouldWritePi,
    unsupportedMessage: () => getDesktopMessage("provider.notForPi"),
  },
};

/**
 * Which CLIs a switch writes. An explicit scope writes exactly that one and must be declared
 * by the row; without a scope, every target the row declares is written — so an untargeted
 * legacy row still means Claude + Codex.
 */
function resolveWriteTargets(
  provider: StoredProvider,
  explicitScope?: ManagedProviderTarget,
): ReadonlySet<ManagedProviderTarget> {
  if (explicitScope) {
    const rule = TARGET_WRITE_RULES[explicitScope];
    if (!rule.supports(provider)) {
      throw new Error(rule.unsupportedMessage());
    }
    return new Set([explicitScope]);
  }

  const targets = new Set<ManagedProviderTarget>();
  for (const target of MANAGED_PROVIDER_TARGETS) {
    if (TARGET_WRITE_RULES[target].supports(provider)) {
      targets.add(target);
    }
  }
  return targets;
}

/**
 * Apply a saved endpoint to the CLIs it targets.
 * @param explicitScope When set, only that CLI is updated (must be supported by the provider row).
 */
export async function switchProvider(
  id: string,
  explicitScope?: ManagedProviderTarget,
  options?: { grokModels?: string[]; piModels?: string[] },
): Promise<ConfigBackup> {
  const store = await loadStore();
  const provider = store.providers.find((entry) => entry.id === id);
  if (!provider) {
    throw new Error(getDesktopMessage("provider.notFound", { id }));
  }

  const targets = resolveWriteTargets(provider, explicitScope);

  const backup = await backupCurrentConfig();
  try {
    if (targets.has("claude")) {
      await writeClaudeSettings(provider);
      store.activeClaudeProviderId = id;
    }
    if (targets.has("codex")) {
      await writeCodexSettings(provider);
      store.activeCodexProviderId = id;
    }
    if (targets.has("grok")) {
      await writeGrokSettings(provider, options?.grokModels ?? []);
      store.activeGrokProviderId = id;
    }
    if (targets.has("pi")) {
      await writePiSettings(provider, options?.piModels ?? []);
      store.activePiProviderId = id;
    }
    syncLegacyActiveProviderId(store);
    await saveStore(store);
  } catch (error) {
    await restoreConfig(backup);
    throw error;
  }
  log.info("[provider-switch] switched to provider:", provider.name, explicitScope ?? "auto");
  return backup;
}

export async function getCurrentProvider(): Promise<StoredProvider | null> {
  const store = await loadStore();
  const id = store.activeClaudeProviderId ?? store.activeProviderId;
  if (!id) {
    return null;
  }
  return store.providers.find((p) => p.id === id) ?? null;
}

function upsertProviderRow(store: ProviderStore, provider: StoredProvider): void {
  const normalized = normalizeProvider(provider);
  const idx = store.providers.findIndex((entry) => entry.id === normalized.id);
  if (idx >= 0) {
    store.providers[idx] = normalized;
  } else {
    store.providers.push(normalized);
  }
}

const MANAGED_PROVIDER_ID_BY_TARGET: Record<ManagedProviderTarget, string> = {
  claude: PASEO_MANAGED_CLAUDE_PROVIDER_ID,
  codex: PASEO_MANAGED_CODEX_PROVIDER_ID,
  grok: PASEO_MANAGED_GROK_PROVIDER_ID,
  pi: PASEO_MANAGED_PI_PROVIDER_ID,
};

export function buildPaseoManagedProvider(
  target: ManagedProviderTarget,
  params: { endpoint: string; apiKey: string; name: string },
): StoredProvider {
  return normalizeProvider({
    id: MANAGED_PROVIDER_ID_BY_TARGET[target],
    name: params.name,
    type: "default",
    endpoint: normalizeProviderEndpoint(params.endpoint),
    apiKey: params.apiKey,
    isDefault: true,
    target,
    // Wire settings that only mean something for their own CLI.
    ...(target === "claude" ? { claudeApiFormat: "anthropic" as const } : {}),
    ...(target === "codex" ? { codexWireApi: "responses" as const } : {}),
  });
}

export function buildPaseoManagedClaudeProvider(params: {
  endpoint: string;
  apiKey: string;
  name: string;
}): StoredProvider {
  return buildPaseoManagedProvider("claude", params);
}

export function buildPaseoManagedCodexProvider(params: {
  endpoint: string;
  apiKey: string;
  name: string;
}): StoredProvider {
  return buildPaseoManagedProvider("codex", params);
}

/**
 * Model families a target can actually run. Mirrors `MANAGED_MODEL_PREFIXES` in the server's
 * managed-provider-model-catalog — this is the last check before a config hits disk, so a
 * wrong model id here would produce a valid-looking but broken setup.
 */
const TARGET_MODEL_PREFIXES: Partial<Record<ManagedProviderTarget, string>> = {
  grok: "grok-",
};

function modelsForTarget(target: ManagedProviderTarget, models: string[]): string[] {
  const prefix = TARGET_MODEL_PREFIXES[target];
  return prefix ? models.filter((id) => id.toLowerCase().startsWith(prefix)) : models;
}

/** Writes one target's config and marks it active in the store. */
async function applyManagedTarget(
  store: ProviderStore,
  target: ManagedProviderTarget,
  provider: StoredProvider,
  models: string[],
): Promise<void> {
  upsertProviderRow(store, provider);
  switch (target) {
    case "claude":
      await writeClaudeSettings(provider);
      store.activeClaudeProviderId = provider.id;
      break;
    case "codex":
      await writeCodexSettings(provider);
      store.activeCodexProviderId = provider.id;
      break;
    case "grok":
      await writeGrokSettings(provider, modelsForTarget("grok", models));
      store.activeGrokProviderId = provider.id;
      break;
    case "pi":
      await writePiSettings(provider, modelsForTarget("pi", models));
      store.activePiProviderId = provider.id;
      break;
  }
}

/**
 * Apply Paseo Cloud session key / group routing to one or both CLIs.
 * @param scope `both` (default): same key to Claude + Codex via two managed rows. `claude` / `codex`: that CLI only.
 */
export async function setupDefaultProvider(params: {
  endpoint: string;
  apiKey: string;
  name?: string;
  scope?: SetupManagedCloudScope;
  /** Gateway model ids. Required for Grok and Pi, whose configs embed an explicit list. */
  models?: string[];
  platform?: NodeJS.Platform;
  gitBashPath?: string | null;
}): Promise<StoredProvider> {
  const scope: SetupManagedCloudScope = params.scope ?? "both";
  const baseName = params.name ?? DEFAULT_PROVIDER_NAME;
  const store = await loadStore();
  store.providers = store.providers.filter((entry) => entry.id !== LEGACY_DEFAULT_PROVIDER_ID);

  // "both" writes two rows, so each needs a distinguishing suffix.
  const targets: ManagedProviderTarget[] = scope === "both" ? ["claude", "codex"] : [scope];
  const displayNameFor = (target: ManagedProviderTarget): string =>
    scope === "both" ? `${baseName} (${target === "claude" ? "Claude Code" : "Codex"})` : baseName;

  const built = targets.map((target) => ({
    target,
    provider: buildPaseoManagedProvider(target, {
      endpoint: params.endpoint,
      apiKey: params.apiKey,
      name: displayNameFor(target),
    }),
  }));

  const backup = await backupCurrentConfig();
  try {
    if (scope === "both") {
      store.providers = store.providers.filter((entry) => entry.id !== DEFAULT_PROVIDER_ID);
    }
    for (const { target, provider } of built) {
      await applyManagedTarget(store, target, provider, params.models ?? []);
    }
    syncLegacyActiveProviderId(store);
    await saveStore(store);
  } catch (error) {
    await restoreConfig(backup);
    throw error;
  }
  log.info("[provider-switch] set up managed cloud provider scope:", scope);
  // "both" reports the Claude row, matching the previous behaviour; a single scope reports its own.
  return built[0]!.provider;
}

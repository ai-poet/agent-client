/**
 * Provider switching for Claude Code and Codex configurations.
 *
 * Reads/writes ~/.claude/settings.json and ~/.codex/ config files.
 * Custom entries may target only Claude, only Codex, or both (managed default).
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

export type SetupManagedCloudScope = "claude" | "codex" | "both";

export type ManagedProviderTarget = "claude" | "codex" | "grok";

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
  /** Optional so stores written before Grok routing existed still load. */
  activeGrokProviderId?: string | null;
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

/**
 * Grok reads its whole configuration from GROK_HOME. We point it at a directory we own
 * instead of editing the user's `~/.grok/config.toml` in place, so their own setup is
 * never rewritten and switching back is just dropping the env var.
 */
export function grokManagedHomePath(): string {
  return join(homedir(), ".agent-client", "agent-runtimes", "grok");
}

function grokManagedConfigPath(): string {
  return join(grokManagedHomePath(), "config.toml");
}

/** Env var the generated TOML points at, so the key itself never lands on disk. */
export const GROK_GATEWAY_KEY_ENV = "PASEO_GROK_GATEWAY_KEY";

function escapeTomlString(value: string): string {
  return value.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

/**
 * Builds the managed Grok config. Every gateway model gets a `[model.<id>]` block whose
 * credentials are an env-var reference rather than the literal key.
 */
export function buildGrokConfigToml(options: {
  endpoint: string;
  models: string[];
  defaultModel?: string;
}): string {
  const lines: string[] = [
    "# Managed by Agent Client. Edits here are overwritten when the route changes.",
    "",
  ];

  if (options.defaultModel) {
    lines.push("[models]", `default = "${escapeTomlString(options.defaultModel)}"`, "");
  }

  for (const model of options.models) {
    lines.push(
      `[model."${escapeTomlString(model)}"]`,
      `model = "${escapeTomlString(model)}"`,
      `base_url = "${escapeTomlString(options.endpoint)}"`,
      // The gateway speaks the OpenAI Responses API for these models.
      'api_backend = "responses"',
      `env_key = "${GROK_GATEWAY_KEY_ENV}"`,
      "",
    );
  }

  return lines.join("\n");
}

function daemonConfigPath(): string {
  return join(homedir(), ".agent-client", "config.json");
}

/**
 * Merges per-provider env into the daemon's config so it reaches the spawned CLI via
 * `applyProviderEnv`. Only the keys we own are touched; the rest of the file is preserved.
 */
async function patchDaemonProviderEnv(
  providerId: string,
  env: Record<string, string> | null,
): Promise<void> {
  const configPath = daemonConfigPath();
  const config = parseJsonObjectForMerge(await readFileOrNull(configPath), "daemon config.json");

  const agents = isRecord(config.agents) ? { ...config.agents } : {};
  const providers = isRecord(agents.providers) ? { ...agents.providers } : {};
  const existing = isRecord(providers[providerId]) ? { ...providers[providerId] } : {};

  if (env === null) {
    delete existing.env;
  } else {
    existing.env = { ...(isRecord(existing.env) ? existing.env : {}), ...env };
  }

  if (Object.keys(existing).length === 0) {
    delete providers[providerId];
  } else {
    providers[providerId] = existing;
  }
  agents.providers = providers;

  await atomicWriteText(configPath, JSON.stringify({ ...config, agents }, null, 2));
}

async function writeGrokSettings(provider: StoredProvider, models: string[]): Promise<void> {
  await mkdir(grokManagedHomePath(), { recursive: true });
  await atomicWriteText(
    grokManagedConfigPath(),
    buildGrokConfigToml({
      endpoint: provider.endpoint,
      models,
      defaultModel: models[0],
    }),
  );
  // GROK_HOME redirects the CLI at the config we just generated; the key is passed
  // separately so the TOML can reference it by name instead of embedding it.
  await patchDaemonProviderEnv("grok", {
    GROK_HOME: grokManagedHomePath(),
    [GROK_GATEWAY_KEY_ENV]: provider.apiKey,
  });
  log.info("[provider-switch] wrote managed grok config for provider:", provider.name);
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

function formatTomlValue(value: string | boolean): string {
  return typeof value === "boolean" ? String(value) : quoteTomlString(value);
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

function formatTomlAssignment(key: string, value: string | boolean): string {
  return `${key} = ${formatTomlValue(value)}`;
}

function upsertTopLevelTomlKey(toml: string, key: string, value: string | boolean): string {
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

function upsertTomlSection(
  toml: string,
  sectionName: string,
  entries: Record<string, string | boolean>,
): string {
  const lines = toml.split(/\r?\n/u);
  const sectionIndexes: number[] = [];

  for (let i = 0; i < lines.length; i += 1) {
    const header = parseTomlHeader(lines[i]!);
    if (header?.name === sectionName) {
      if (header.isArray) {
        throw new Error(`Codex config section [[${sectionName}]] cannot be safely patched.`);
      }
      sectionIndexes.push(i);
    }
  }

  if (sectionIndexes.length > 1) {
    throw new Error(`Codex config has duplicate [${sectionName}] sections.`);
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
    lines.push(`[${sectionName}]`, ...assignmentLines);
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
}

export async function backupCurrentConfig(): Promise<ConfigBackup> {
  return {
    timestamp: Date.now(),
    claudeSettings: await readFileOrNull(claudeSettingsPath()),
    codexAuth: await readFileOrNull(codexAuthPath()),
    codexConfig: await readFileOrNull(codexConfigPath()),
  };
}

export async function restoreConfig(backup: ConfigBackup): Promise<void> {
  await restoreFile(claudeSettingsPath(), backup.claudeSettings);
  await restoreFile(codexAuthPath(), backup.codexAuth);
  await restoreFile(codexConfigPath(), backup.codexConfig);
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

/**
 * Apply a saved endpoint to Claude and/or Codex on disk.
 * @param explicitScope When set, only that CLI is updated (must be supported by the provider row).
 */
export async function switchProvider(
  id: string,
  explicitScope?: ManagedProviderTarget,
  options?: { grokModels?: string[] },
): Promise<ConfigBackup> {
  const store = await loadStore();
  const provider = store.providers.find((entry) => entry.id === id);
  if (!provider) {
    throw new Error(getDesktopMessage("provider.notFound", { id }));
  }

  let writeClaude: boolean;
  let writeCodex: boolean;
  let writeGrok = false;
  if (explicitScope === "grok") {
    if (!shouldWriteGrok(provider)) {
      throw new Error(getDesktopMessage("provider.notForGrok"));
    }
    writeClaude = false;
    writeCodex = false;
    writeGrok = true;
  } else if (explicitScope === "claude") {
    if (!shouldWriteClaude(provider)) {
      throw new Error(getDesktopMessage("provider.notForClaude"));
    }
    writeClaude = true;
    writeCodex = false;
  } else if (explicitScope === "codex") {
    if (!shouldWriteCodex(provider)) {
      throw new Error(getDesktopMessage("provider.notForCodex"));
    }
    writeClaude = false;
    writeCodex = true;
  } else {
    writeClaude = shouldWriteClaude(provider);
    writeCodex = shouldWriteCodex(provider);
    writeGrok = shouldWriteGrok(provider);
  }

  const backup = await backupCurrentConfig();
  try {
    if (writeClaude) {
      await writeClaudeSettings(provider);
      store.activeClaudeProviderId = id;
    }
    if (writeCodex) {
      await writeCodexSettings(provider);
      store.activeCodexProviderId = id;
    }
    if (writeGrok) {
      await writeGrokSettings(provider, options?.grokModels ?? []);
      store.activeGrokProviderId = id;
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

export function buildPaseoManagedClaudeProvider(params: {
  endpoint: string;
  apiKey: string;
  name: string;
}): StoredProvider {
  return normalizeProvider({
    id: PASEO_MANAGED_CLAUDE_PROVIDER_ID,
    name: params.name,
    type: "default",
    endpoint: normalizeProviderEndpoint(params.endpoint),
    apiKey: params.apiKey,
    isDefault: true,
    target: "claude",
    claudeApiFormat: "anthropic",
  });
}

export function buildPaseoManagedCodexProvider(params: {
  endpoint: string;
  apiKey: string;
  name: string;
}): StoredProvider {
  return normalizeProvider({
    id: PASEO_MANAGED_CODEX_PROVIDER_ID,
    name: params.name,
    type: "default",
    endpoint: normalizeProviderEndpoint(params.endpoint),
    apiKey: params.apiKey,
    isDefault: true,
    target: "codex",
    codexWireApi: "responses",
  });
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
  platform?: NodeJS.Platform;
  gitBashPath?: string | null;
}): Promise<StoredProvider> {
  const scope: SetupManagedCloudScope = params.scope ?? "both";
  const baseName = params.name ?? DEFAULT_PROVIDER_NAME;
  const store = await loadStore();
  store.providers = store.providers.filter((entry) => entry.id !== LEGACY_DEFAULT_PROVIDER_ID);

  const claudeDisplayName = scope === "both" ? `${baseName} (Claude Code)` : baseName;
  const codexDisplayName = scope === "both" ? `${baseName} (Codex)` : baseName;

  const backup = await backupCurrentConfig();
  try {
    if (scope === "both") {
      store.providers = store.providers.filter((entry) => entry.id !== DEFAULT_PROVIDER_ID);
      const claudeP = buildPaseoManagedClaudeProvider({
        endpoint: params.endpoint,
        apiKey: params.apiKey,
        name: claudeDisplayName,
      });
      const codexP = buildPaseoManagedCodexProvider({
        endpoint: params.endpoint,
        apiKey: params.apiKey,
        name: codexDisplayName,
      });
      upsertProviderRow(store, claudeP);
      upsertProviderRow(store, codexP);
      await writeClaudeSettings(claudeP);
      await writeCodexSettings(codexP);
      store.activeClaudeProviderId = PASEO_MANAGED_CLAUDE_PROVIDER_ID;
      store.activeCodexProviderId = PASEO_MANAGED_CODEX_PROVIDER_ID;
    } else if (scope === "claude") {
      const claudeP = buildPaseoManagedClaudeProvider({
        endpoint: params.endpoint,
        apiKey: params.apiKey,
        name: claudeDisplayName,
      });
      upsertProviderRow(store, claudeP);
      await writeClaudeSettings(claudeP);
      store.activeClaudeProviderId = PASEO_MANAGED_CLAUDE_PROVIDER_ID;
    } else {
      const codexP = buildPaseoManagedCodexProvider({
        endpoint: params.endpoint,
        apiKey: params.apiKey,
        name: codexDisplayName,
      });
      upsertProviderRow(store, codexP);
      await writeCodexSettings(codexP);
      store.activeCodexProviderId = PASEO_MANAGED_CODEX_PROVIDER_ID;
    }
    syncLegacyActiveProviderId(store);
    await saveStore(store);
  } catch (error) {
    await restoreConfig(backup);
    throw error;
  }
  log.info("[provider-switch] set up managed cloud provider scope:", scope);
  if (scope === "codex") {
    return buildPaseoManagedCodexProvider({
      endpoint: params.endpoint,
      apiKey: params.apiKey,
      name: codexDisplayName,
    });
  }
  return buildPaseoManagedClaudeProvider({
    endpoint: params.endpoint,
    apiKey: params.apiKey,
    name: claudeDisplayName,
  });
}

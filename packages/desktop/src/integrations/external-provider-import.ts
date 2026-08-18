import { existsSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";
import log from "electron-log/main";

/**
 * Read-only import of agent endpoints configured in other tools. Nothing here writes to
 * the source application — we only read its config so the user does not have to retype
 * base URLs and keys.
 */

export type ExternalImportSource = "ccswitch" | "cherry-studio";

export interface ExternalProviderCandidate {
  /** Stable id combining the source and the row it came from. */
  id: string;
  source: ExternalImportSource;
  /** Which of our providers this endpoint targets. */
  target: "claude" | "codex";
  name: string;
  baseUrl: string;
  /** Present only when the source stored one; never logged. */
  hasApiKey: boolean;
  apiKey: string;
  models: string[];
}

export interface ExternalImportScan {
  source: ExternalImportSource;
  detected: boolean;
  /** The file we actually read, shown in the UI so the result is verifiable. */
  dataPath: string | null;
  items: ExternalProviderCandidate[];
  error: string | null;
}

function firstExistingFile(candidates: string[]): string | null {
  return candidates.find((candidate) => existsSync(candidate)) ?? null;
}

function ccSwitchDatabaseCandidates(): string[] {
  const home = homedir();
  const appData = process.env.APPDATA;
  const localAppData = process.env.LOCALAPPDATA;
  return [
    ...(appData ? [path.join(appData, "cc-switch", "cc-switch.db")] : []),
    ...(localAppData ? [path.join(localAppData, "cc-switch", "cc-switch.db")] : []),
    path.join(home, ".cc-switch", "cc-switch.db"),
    path.join(home, "Library", "Application Support", "cc-switch", "cc-switch.db"),
    path.join(home, ".config", "cc-switch", "cc-switch.db"),
  ];
}

function cherryStudioDatabaseCandidates(): string[] {
  const home = homedir();
  const appData = process.env.APPDATA;
  return [
    ...(appData ? [path.join(appData, "CherryStudio", "cherrystudio.sqlite")] : []),
    path.join(home, "Library", "Application Support", "CherryStudio", "cherrystudio.sqlite"),
    path.join(home, ".config", "CherryStudio", "cherrystudio.sqlite"),
  ];
}

function readString(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function pickPath(value: unknown, keys: string[]): string {
  let cursor: unknown = value;
  for (const key of keys) {
    if (typeof cursor !== "object" || cursor === null) {
      return "";
    }
    cursor = (cursor as Record<string, unknown>)[key];
  }
  return readString(cursor);
}

const CLAUDE_MODEL_KEYS = [
  "ANTHROPIC_MODEL",
  "ANTHROPIC_REASONING_MODEL",
  "ANTHROPIC_DEFAULT_SONNET_MODEL",
  "ANTHROPIC_DEFAULT_OPUS_MODEL",
  "ANTHROPIC_DEFAULT_HAIKU_MODEL",
];

function parseCcSwitchClaude(
  sourceId: string,
  name: string,
  settings: unknown,
): ExternalProviderCandidate | null {
  const baseUrl =
    pickPath(settings, ["env", "ANTHROPIC_BASE_URL"]) ||
    pickPath(settings, ["config", "ANTHROPIC_BASE_URL"]);
  const apiKey =
    pickPath(settings, ["env", "ANTHROPIC_AUTH_TOKEN"]) ||
    pickPath(settings, ["env", "ANTHROPIC_API_KEY"]);
  if (!baseUrl) {
    return null;
  }

  const models = new Set<string>();
  for (const key of CLAUDE_MODEL_KEYS) {
    const model = pickPath(settings, ["env", key]);
    if (model) {
      models.add(model);
    }
  }

  return {
    id: `ccswitch:${sourceId}`,
    source: "ccswitch",
    target: "claude",
    name: name || baseUrl,
    baseUrl,
    hasApiKey: apiKey.length > 0,
    apiKey,
    models: [...models],
  };
}

function parseCcSwitchCodex(
  sourceId: string,
  name: string,
  settings: unknown,
): ExternalProviderCandidate | null {
  const baseUrl =
    pickPath(settings, ["env", "OPENAI_BASE_URL"]) ||
    pickPath(settings, ["config", "OPENAI_BASE_URL"]) ||
    pickPath(settings, ["auth", "base_url"]);
  const apiKey =
    pickPath(settings, ["env", "OPENAI_API_KEY"]) || pickPath(settings, ["auth", "OPENAI_API_KEY"]);
  if (!baseUrl) {
    return null;
  }

  const model = pickPath(settings, ["config", "model"]) || pickPath(settings, ["model"]);
  return {
    id: `ccswitch:${sourceId}`,
    source: "ccswitch",
    target: "codex",
    name: name || baseUrl,
    baseUrl,
    hasApiKey: apiKey.length > 0,
    apiKey,
    models: model ? [model] : [],
  };
}

/**
 * `node:sqlite` ships with the Node bundled in Electron 41, so reading these databases
 * needs no extra dependency. It is loaded lazily so an environment without it degrades to
 * "nothing detected" rather than failing at import time.
 */
async function openReadOnly(file: string): Promise<{
  all: (sql: string) => Record<string, unknown>[];
  close: () => void;
} | null> {
  try {
    const { DatabaseSync } = await import("node:sqlite");
    const db = new DatabaseSync(file, { readOnly: true });
    return {
      all: (sql: string) => db.prepare(sql).all() as Record<string, unknown>[],
      close: () => db.close(),
    };
  } catch (error) {
    log.warn("[external-import] Unable to open database", { file, error: String(error) });
    return null;
  }
}

export async function scanCcSwitch(): Promise<ExternalImportScan> {
  const file = firstExistingFile(ccSwitchDatabaseCandidates());
  if (!file) {
    return { source: "ccswitch", detected: false, dataPath: null, items: [], error: null };
  }

  const db = await openReadOnly(file);
  if (!db) {
    return {
      source: "ccswitch",
      detected: true,
      dataPath: file,
      items: [],
      error: "Could not read the cc-switch database",
    };
  }

  try {
    const rows = db.all(
      `SELECT id, app_type, name, settings_config FROM providers
       WHERE app_type IN ('claude','claude-code','claude_code','codex')`,
    );
    const items: ExternalProviderCandidate[] = [];
    for (const row of rows) {
      const sourceId = readString(row.id);
      const appType = readString(row.app_type).toLowerCase();
      const name = readString(row.name);
      let settings: unknown;
      try {
        settings = JSON.parse(readString(row.settings_config));
      } catch {
        continue;
      }
      const candidate =
        appType === "codex"
          ? parseCcSwitchCodex(sourceId, name, settings)
          : parseCcSwitchClaude(sourceId, name, settings);
      if (candidate) {
        items.push(candidate);
      }
    }
    return { source: "ccswitch", detected: true, dataPath: file, items, error: null };
  } catch (error) {
    return {
      source: "ccswitch",
      detected: true,
      dataPath: file,
      items: [],
      error: error instanceof Error ? error.message : String(error),
    };
  } finally {
    db.close();
  }
}

export async function scanCherryStudio(): Promise<ExternalImportScan> {
  const file = firstExistingFile(cherryStudioDatabaseCandidates());
  if (!file) {
    // The pre-v2 LevelDB layout is intentionally not supported — it would need a
    // LevelDB reader dependency for a legacy format.
    return { source: "cherry-studio", detected: false, dataPath: null, items: [], error: null };
  }

  const db = await openReadOnly(file);
  if (!db) {
    return {
      source: "cherry-studio",
      detected: true,
      dataPath: file,
      items: [],
      error: "Could not read the Cherry Studio database",
    };
  }

  try {
    const rows = db.all(`SELECT id, name, type, api_host, api_key FROM providers`);
    const items: ExternalProviderCandidate[] = [];
    for (const row of rows) {
      const baseUrl = readString(row.api_host);
      if (!baseUrl) {
        continue;
      }
      const type = readString(row.type).toLowerCase();
      const apiKey = readString(row.api_key);
      items.push({
        id: `cherry:${readString(row.id)}`,
        source: "cherry-studio",
        target: type.includes("anthropic") || type.includes("claude") ? "claude" : "codex",
        name: readString(row.name) || baseUrl,
        baseUrl,
        hasApiKey: apiKey.length > 0,
        apiKey,
        models: [],
      });
    }
    return { source: "cherry-studio", detected: true, dataPath: file, items, error: null };
  } catch (error) {
    return {
      source: "cherry-studio",
      detected: true,
      dataPath: file,
      items: [],
      // Schema drift between Cherry Studio versions is expected; surface it rather than
      // pretending nothing was found.
      error: error instanceof Error ? error.message : String(error),
    };
  } finally {
    db.close();
  }
}

export async function scanExternalProviders(): Promise<ExternalImportScan[]> {
  return Promise.all([scanCcSwitch(), scanCherryStudio()]);
}

/**
 * Writes the selected endpoints into our own saved-endpoints store. Existing entries with
 * the same id are overwritten, which is what the UI warns about before committing; their
 * id, default flag and history are preserved by the store's own upsert.
 */
export async function importExternalProviders(ids: string[]): Promise<{ imported: number }> {
  if (ids.length === 0) {
    return { imported: 0 };
  }

  const scans = await scanExternalProviders();
  const wanted = new Set(ids);
  const candidates = scans.flatMap((scan) => scan.items).filter((item) => wanted.has(item.id));

  const { addProvider } = await import("../features/provider-switch.js");
  let imported = 0;
  for (const candidate of candidates) {
    await addProvider({
      id: candidate.id,
      name: candidate.name,
      type: "custom",
      endpoint: candidate.baseUrl,
      apiKey: candidate.apiKey,
      isDefault: false,
      target: candidate.target,
      ...(candidate.target === "claude"
        ? { claudeApiFormat: "anthropic" as const }
        : { codexWireApi: "responses" as const }),
    });
    imported += 1;
  }

  return { imported };
}

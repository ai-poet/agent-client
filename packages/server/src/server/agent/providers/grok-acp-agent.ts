import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import type { Logger } from "pino";

import type {
  AgentCapabilityFlags,
  AgentMode,
  AgentModelDefinition,
  ProviderInfo,
} from "../agent-sdk-types.js";
import type { ListModelsOptions, ListModesOptions } from "../agent-sdk-types.js";
import type { ProviderRuntimeSettings } from "../provider-launch-config.js";
import { findExecutable } from "../../../utils/executable.js";
import { ACPAgentClient, PROBE_ENV, deriveModelDefinitionsFromACP } from "./acp-agent.js";
import {
  formatDiagnosticStatus,
  formatProviderDiagnostic,
  formatProviderDiagnosticError,
  resolveBinaryVersion,
  resolveCliVersion,
  toDiagnosticErrorMessage,
} from "./diagnostic-utils.js";

const GROK_BINARY_COMMAND = "grok";

const GROK_CAPABILITIES: AgentCapabilityFlags = {
  supportsStreaming: true,
  supportsSessionPersistence: true,
  supportsDynamicModes: true,
  supportsMcpServers: true,
  supportsReasoningStream: true,
  supportsToolInvocations: true,
};

type GrokACPAgentClientOptions = {
  logger: Logger;
  runtimeSettings?: ProviderRuntimeSettings;
};

/**
 * Offered when the CLI will not enumerate models yet. Keeping Grok selectable while it is
 * merely unconfigured matches how Claude and Codex behave.
 */
const FALLBACK_GROK_MODELS = ["grok-4.6", "grok-4.5"];

/** Grok relocates its whole config directory (auth + config.toml) when GROK_HOME is set. */
function resolveGrokHome(): string {
  return process.env.GROK_HOME ?? join(homedir(), ".grok");
}

/**
 * The installer leaves behind a stub `config.toml` holding nothing but `[cli] installer`, so
 * the file existing proves nothing. Only a key inside it counts as a configured route.
 */
function grokConfigCarriesKey(configPath: string): boolean {
  try {
    return /^\s*(?:api_key|env_key)\s*=/mu.test(readFileSync(configPath, "utf8"));
  } catch {
    return false;
  }
}

/**
 * Credentials can arrive three ways: the env var, an OAuth login (`auth.json`), or a gateway
 * route written into `config.toml` — which is how a managed CheapRouter login sets Grok up.
 */
function hasGrokCredentials(): boolean {
  if (process.env.XAI_API_KEY) {
    return true;
  }
  const home = resolveGrokHome();
  return (
    existsSync(join(home, "auth.json")) || grokConfigCarriesKey(join(home, "config.toml"))
  );
}

type InitializeModelState = {
  availableModels: { modelId: string; name: string; description?: string | null }[];
  currentModelId: string | null;
};

/**
 * Grok reports its catalog on the `initialize` response rather than on `session/new`,
 * which is where {@link deriveModelDefinitionsFromACP} looks. The shape is narrowed at
 * runtime because it is outside the typed ACP surface.
 */
function extractInitializeModels(initialize: unknown): InitializeModelState | null {
  if (typeof initialize !== "object" || initialize === null) {
    return null;
  }

  const models = (initialize as { models?: unknown }).models;
  if (typeof models !== "object" || models === null) {
    return null;
  }

  const availableModels = (models as { availableModels?: unknown }).availableModels;
  if (!Array.isArray(availableModels) || availableModels.length === 0) {
    return null;
  }

  const parsed: InitializeModelState["availableModels"] = [];
  for (const entry of availableModels) {
    if (typeof entry !== "object" || entry === null) {
      continue;
    }
    const { modelId, name, description } = entry as {
      modelId?: unknown;
      name?: unknown;
      description?: unknown;
    };
    if (typeof modelId !== "string" || modelId.length === 0) {
      continue;
    }
    parsed.push({
      modelId,
      name: typeof name === "string" && name.length > 0 ? name : modelId,
      description: typeof description === "string" ? description : null,
    });
  }

  if (parsed.length === 0) {
    return null;
  }

  const currentModelId = (models as { currentModelId?: unknown }).currentModelId;
  return {
    availableModels: parsed,
    currentModelId: typeof currentModelId === "string" ? currentModelId : null,
  };
}

export class GrokACPAgentClient extends ACPAgentClient {
  /** Held locally rather than reaching into the base class's own logger field. */
  private readonly log: Logger;

  constructor(options: GrokACPAgentClientOptions) {
    super({
      provider: "grok",
      logger: options.logger,
      runtimeSettings: options.runtimeSettings,
      defaultCommand: [GROK_BINARY_COMMAND, "agent", "stdio"],
      // Modes stay empty so they are derived from whatever Grok reports over ACP.
      // Declaring static modes here would make the session layer call `session/set_mode`
      // with ids the agent may not know.
      defaultModes: [],
      capabilities: GROK_CAPABILITIES,
    });
    this.log = options.logger;
  }

  override async listModels(options: ListModelsOptions): Promise<AgentModelDefinition[]> {
    try {
      return await this.probeModels(options);
    } catch (error) {
      // Grok refuses to enumerate models before it is authenticated ("no auth method id
      // provided"). Failing here would drop the provider out of the picker entirely, so we
      // fall back to the known catalog — the real one replaces it once a route is written.
      this.log.debug({ err: error }, "Grok model probe failed; using fallback catalog");
      return FALLBACK_GROK_MODELS.map((id, index) => ({
        provider: this.provider,
        id,
        label: id,
        isDefault: index === 0,
      }));
    }
  }

  /**
   * The base implementation opens a session to read modes, which hits the same pre-auth
   * refusal as the model probe. Modes are fully dynamic for Grok (`defaultModes` is empty),
   * so an empty list is the honest answer — and it keeps the snapshot out of `error`.
   */
  override async listModes(options: ListModesOptions): Promise<AgentMode[]> {
    try {
      return await super.listModes(options);
    } catch (error) {
      this.log.debug({ err: error }, "Grok mode probe failed; reporting no modes");
      return [];
    }
  }

  private async probeModels(options: ListModelsOptions): Promise<AgentModelDefinition[]> {
    const probe = await this.spawnProcess(PROBE_ENV);
    try {
      const initializeModels = extractInitializeModels(probe.initialize);
      if (initializeModels) {
        return initializeModels.availableModels.map((model) => ({
          provider: this.provider,
          id: model.modelId,
          label: model.name,
          description: model.description ?? undefined,
          isDefault: model.modelId === initializeModels.currentModelId,
        }));
      }

      const response = await probe.connection.newSession({ cwd: options.cwd, mcpServers: [] });
      const transformed = this.transformSessionResponse(response);
      return deriveModelDefinitionsFromACP(
        this.provider,
        transformed.models,
        transformed.configOptions,
      );
    } finally {
      await this.closeProbe(probe);
    }
  }

  /**
   * Installed means selectable, matching Claude and Codex — neither of which checks
   * credentials here. A missing key is a configuration gap the user fixes by editing the
   * route (or `config.toml`) directly, not a reason to hide the provider; `getDiagnostic`
   * is where that gap is reported.
   */
  override async isAvailable(): Promise<boolean> {
    return super.isAvailable();
  }

  async getProviderInfo(): Promise<ProviderInfo> {
    return {
      version: await resolveCliVersion(GROK_BINARY_COMMAND, this.runtimeSettings),
      configDir: resolveGrokHome(),
    };
  }

  async getDiagnostic(): Promise<{ diagnostic: string }> {
    try {
      const available = await this.isAvailable();
      const resolvedBinary = await findExecutable(GROK_BINARY_COMMAND);
      const authConfigPath = join(resolveGrokHome(), "auth.json");
      const grokConfigPath = join(resolveGrokHome(), "config.toml");
      let modelsValue = "Not checked";
      let status = formatDiagnosticStatus(available);

      if (available) {
        try {
          const models = await this.listModels({ cwd: homedir(), force: false });
          modelsValue = String(models.length);
        } catch (error) {
          modelsValue = `Error - ${toDiagnosticErrorMessage(error)}`;
          status = formatDiagnosticStatus(available, {
            source: "model fetch",
            cause: error,
          });
        }
      }

      return {
        diagnostic: formatProviderDiagnostic("Grok", [
          { label: "Binary", value: resolvedBinary ?? "not found" },
          {
            label: "Version",
            value: resolvedBinary ? await resolveBinaryVersion(resolvedBinary) : "unknown",
          },
          { label: "XAI_API_KEY", value: process.env.XAI_API_KEY ? "set" : "not set" },
          {
            label: "Auth config",
            value: existsSync(authConfigPath) ? authConfigPath : `not found (${authConfigPath})`,
          },
          {
            label: "Config",
            value: existsSync(grokConfigPath) ? grokConfigPath : `not found (${grokConfigPath})`,
          },
          // Reported, not enforced: Grok stays selectable so the route can be configured.
          {
            label: "Credentials",
            value: hasGrokCredentials() ? "configured" : "not configured",
          },
          { label: "Models", value: modelsValue },
          { label: "Status", value: status },
        ]),
      };
    } catch (error) {
      return {
        diagnostic: formatProviderDiagnosticError("Grok", error),
      };
    }
  }
}

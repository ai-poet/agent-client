import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import type { Logger } from "pino";

import type { AgentCapabilityFlags, AgentModelDefinition } from "../agent-sdk-types.js";
import type { ListModelsOptions } from "../agent-sdk-types.js";
import type { ProviderRuntimeSettings } from "../provider-launch-config.js";
import { findExecutable } from "../../../utils/executable.js";
import { ACPAgentClient, PROBE_ENV, deriveModelDefinitionsFromACP } from "./acp-agent.js";
import {
  formatDiagnosticStatus,
  formatProviderDiagnostic,
  formatProviderDiagnosticError,
  resolveBinaryVersion,
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

/** Grok relocates its whole config directory (auth + config.toml) when GROK_HOME is set. */
function resolveGrokHome(): string {
  return process.env.GROK_HOME ?? join(homedir(), ".grok");
}

function hasGrokCredentials(): boolean {
  return Boolean(process.env.XAI_API_KEY) || existsSync(join(resolveGrokHome(), "auth.json"));
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
  }

  override async listModels(options: ListModelsOptions): Promise<AgentModelDefinition[]> {
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

  override async isAvailable(): Promise<boolean> {
    if (!(await super.isAvailable())) {
      return false;
    }
    return hasGrokCredentials();
  }

  async getDiagnostic(): Promise<{ diagnostic: string }> {
    try {
      const available = await this.isAvailable();
      const resolvedBinary = await findExecutable(GROK_BINARY_COMMAND);
      const authConfigPath = join(resolveGrokHome(), "auth.json");
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

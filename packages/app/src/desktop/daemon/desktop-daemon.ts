import { getDesktopHost, isElectronRuntime } from "@/desktop/host";
import { invokeDesktopCommand } from "@/desktop/electron/invoke";

export type DesktopDaemonState = "starting" | "running" | "stopped" | "errored";

export type DesktopDaemonStatus = {
  serverId: string;
  status: DesktopDaemonState;
  listen: string | null;
  hostname: string | null;
  pid: number | null;
  home: string;
  version: string | null;
  desktopManaged: boolean;
  error: string | null;
};

export type DesktopDaemonLogs = {
  logPath: string;
  contents: string;
};

export type DesktopPairingOffer = {
  relayEnabled: boolean;
  url: string | null;
  qr: string | null;
};

export type LocalTransportTarget = {
  transportType: "socket" | "pipe";
  transportPath: string;
};

type LocalTransportEventPayload = {
  sessionId: string;
  kind: "open" | "message" | "close" | "error";
  text?: string | null;
  binaryBase64?: string | null;
  code?: number | null;
  reason?: string | null;
  error?: string | null;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function toStringOrNull(value: unknown): string | null {
  return typeof value === "string" && value.trim().length > 0 ? value : null;
}

function toNumberOrNull(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function parseDesktopDaemonState(value: unknown): DesktopDaemonState {
  const normalized = toStringOrNull(value)?.toLowerCase();
  switch (normalized) {
    case "starting":
      return "starting";
    case "running":
      return "running";
    case "errored":
    case "error":
      return "errored";
    case "stopped":
    case "stopping":
    case "unknown":
    default:
      return "stopped";
  }
}

function parseDesktopDaemonStatus(raw: unknown): DesktopDaemonStatus {
  if (!isRecord(raw)) {
    throw new Error("Unexpected desktop daemon status response.");
  }
  return {
    serverId: toStringOrNull(raw.serverId) ?? "",
    status: parseDesktopDaemonState(raw.status),
    listen: toStringOrNull(raw.listen),
    hostname: toStringOrNull(raw.hostname),
    pid: toNumberOrNull(raw.pid),
    home: toStringOrNull(raw.home) ?? "",
    version: toStringOrNull(raw.version),
    desktopManaged: raw.desktopManaged === true,
    error: toStringOrNull(raw.error),
  };
}

function parseDesktopDaemonLogs(raw: unknown): DesktopDaemonLogs {
  if (!isRecord(raw)) {
    throw new Error("Unexpected desktop daemon logs response.");
  }
  return {
    logPath: toStringOrNull(raw.logPath) ?? "",
    contents: typeof raw.contents === "string" ? raw.contents : "",
  };
}

function parseDesktopPairingOffer(raw: unknown): DesktopPairingOffer {
  if (!isRecord(raw)) {
    throw new Error("Unexpected desktop daemon pairing response.");
  }
  return {
    relayEnabled: raw.relayEnabled === true,
    url: toStringOrNull(raw.url),
    qr: toStringOrNull(raw.qr),
  };
}

export function shouldUseDesktopDaemon(): boolean {
  return isElectronRuntime();
}

export async function getDesktopDaemonStatus(): Promise<DesktopDaemonStatus> {
  return parseDesktopDaemonStatus(await invokeDesktopCommand("desktop_daemon_status"));
}

export async function startDesktopDaemon(): Promise<DesktopDaemonStatus> {
  return parseDesktopDaemonStatus(await invokeDesktopCommand("start_desktop_daemon"));
}

export async function stopDesktopDaemon(): Promise<DesktopDaemonStatus> {
  return parseDesktopDaemonStatus(await invokeDesktopCommand("stop_desktop_daemon"));
}

export async function restartDesktopDaemon(): Promise<DesktopDaemonStatus> {
  return parseDesktopDaemonStatus(await invokeDesktopCommand("restart_desktop_daemon"));
}

export async function getDesktopDaemonLogs(): Promise<DesktopDaemonLogs> {
  return parseDesktopDaemonLogs(await invokeDesktopCommand("desktop_daemon_logs"));
}

export async function getDesktopDaemonPairing(): Promise<DesktopPairingOffer> {
  return parseDesktopPairingOffer(await invokeDesktopCommand("desktop_daemon_pairing"));
}

export async function getCliDaemonStatus(): Promise<string> {
  const raw = await invokeDesktopCommand<unknown>("cli_daemon_status");
  if (typeof raw !== "string") {
    throw new Error("Unexpected CLI daemon status response.");
  }
  return raw;
}

export type LocalTransportEventUnlisten = () => void;
export type LocalTransportEventHandler = (payload: LocalTransportEventPayload) => void;

export async function listenToLocalTransportEvents(
  handler: LocalTransportEventHandler,
): Promise<LocalTransportEventUnlisten> {
  const listen = getDesktopHost()?.events?.on;
  if (typeof listen !== "function") {
    throw new Error("Desktop events API is unavailable.");
  }
  const unlisten = await listen("local-daemon-transport-event", (payload: unknown) => {
    if (!isRecord(payload)) {
      return;
    }
    handler({
      sessionId: toStringOrNull(payload.sessionId) ?? "",
      kind: (toStringOrNull(payload.kind) ?? "error") as LocalTransportEventPayload["kind"],
      text: toStringOrNull(payload.text),
      binaryBase64: toStringOrNull(payload.binaryBase64),
      code: toNumberOrNull(payload.code),
      reason: toStringOrNull(payload.reason),
      error: toStringOrNull(payload.error),
    });
  });
  return typeof unlisten === "function" ? unlisten : () => {};
}

export async function openLocalTransportSession(target: LocalTransportTarget): Promise<string> {
  const raw = await invokeDesktopCommand<unknown>("open_local_daemon_transport", target);
  if (typeof raw !== "string" || raw.trim().length === 0) {
    throw new Error("Unexpected local transport session response.");
  }
  return raw;
}

export async function sendLocalTransportMessage(input: {
  sessionId: string;
  text?: string;
  binaryBase64?: string;
}): Promise<void> {
  await invokeDesktopCommand("send_local_daemon_transport_message", {
    sessionId: input.sessionId,
    ...(input.text ? { text: input.text } : {}),
    ...(input.binaryBase64 ? { binaryBase64: input.binaryBase64 } : {}),
  });
}

export async function closeLocalTransportSession(sessionId: string): Promise<void> {
  await invokeDesktopCommand("close_local_daemon_transport", { sessionId });
}

// ---------------------------------------------------------------------------
// Integrations
// ---------------------------------------------------------------------------

export interface InstallStatus {
  installed: boolean;
}

export interface ModelCliRuntimeToolStatus {
  command: "codex" | "claude";
  packageName: string;
  installed: boolean;
  version: string | null;
  error: string | null;
}

export interface NodeRuntimeStatus {
  installed: boolean;
  version: string | null;
  major: number | null;
  npmVersion: string | null;
  satisfies: boolean;
  manager: "nvm" | "brew" | "managed" | "shell";
  error: string | null;
}

export interface GitRuntimeStatus {
  installed: boolean;
  version: string | null;
  bashPath: string | null;
  error: string | null;
}

export interface ModelCliRuntimeStatus {
  git: GitRuntimeStatus;
  node: NodeRuntimeStatus;
  codex: ModelCliRuntimeToolStatus;
  claude: ModelCliRuntimeToolStatus;
}

export interface ModelCliInstallResult {
  status: ModelCliRuntimeStatus;
  output: string;
}

function parseInstallStatus(raw: unknown): InstallStatus {
  if (!isRecord(raw)) {
    throw new Error("Unexpected install status response.");
  }
  return { installed: raw.installed === true };
}

function parseNodeRuntimeStatus(raw: unknown): NodeRuntimeStatus {
  if (!isRecord(raw)) {
    throw new Error("Unexpected node runtime status response.");
  }
  const manager = toStringOrNull(raw.manager);
  if (manager !== "nvm" && manager !== "brew" && manager !== "managed" && manager !== "shell") {
    throw new Error("Unexpected node runtime manager.");
  }
  return {
    installed: raw.installed === true,
    version: toStringOrNull(raw.version),
    major: toNumberOrNull(raw.major),
    npmVersion: toStringOrNull(raw.npmVersion),
    satisfies: raw.satisfies === true,
    manager,
    error: toStringOrNull(raw.error),
  };
}

function parseGitRuntimeStatus(raw: unknown): GitRuntimeStatus {
  if (!isRecord(raw)) {
    return {
      installed: true,
      version: null,
      bashPath: null,
      error: null,
    };
  }
  return {
    installed: raw.installed === true,
    version: toStringOrNull(raw.version),
    bashPath: toStringOrNull(raw.bashPath),
    error: toStringOrNull(raw.error),
  };
}

function parseModelCliRuntimeToolStatus(raw: unknown): ModelCliRuntimeToolStatus {
  if (!isRecord(raw)) {
    throw new Error("Unexpected CLI runtime tool status response.");
  }
  const command = toStringOrNull(raw.command);
  if (command !== "codex" && command !== "claude") {
    throw new Error("Unexpected CLI runtime tool command.");
  }
  return {
    command,
    packageName: toStringOrNull(raw.packageName) ?? "",
    installed: raw.installed === true,
    version: toStringOrNull(raw.version),
    error: toStringOrNull(raw.error),
  };
}

function parseModelCliRuntimeStatus(raw: unknown): ModelCliRuntimeStatus {
  if (!isRecord(raw)) {
    throw new Error("Unexpected CLI runtime status response.");
  }
  return {
    git: parseGitRuntimeStatus(raw.git),
    node: parseNodeRuntimeStatus(raw.node),
    codex: parseModelCliRuntimeToolStatus(raw.codex),
    claude: parseModelCliRuntimeToolStatus(raw.claude),
  };
}

function parseModelCliInstallResult(raw: unknown): ModelCliInstallResult {
  if (!isRecord(raw)) {
    throw new Error("Unexpected CLI install result response.");
  }
  return {
    status: parseModelCliRuntimeStatus(raw.status),
    output: typeof raw.output === "string" ? raw.output : "",
  };
}

export async function getCliInstallStatus(): Promise<InstallStatus> {
  return parseInstallStatus(await invokeDesktopCommand("get_cli_install_status"));
}

export async function installCli(): Promise<InstallStatus> {
  return parseInstallStatus(await invokeDesktopCommand("install_cli"));
}

export async function getSkillsInstallStatus(): Promise<InstallStatus> {
  return parseInstallStatus(await invokeDesktopCommand("get_skills_install_status"));
}

export async function installSkills(): Promise<InstallStatus> {
  return parseInstallStatus(await invokeDesktopCommand("install_skills"));
}

export async function getModelCliRuntimeStatus(): Promise<ModelCliRuntimeStatus> {
  return parseModelCliRuntimeStatus(await invokeDesktopCommand("get_model_cli_runtime_status"));
}

export async function installGitBashRuntime(): Promise<ModelCliInstallResult> {
  return parseModelCliInstallResult(await invokeDesktopCommand("install_git_bash_runtime"));
}

export async function installNode22Runtime(): Promise<ModelCliInstallResult> {
  return parseModelCliInstallResult(await invokeDesktopCommand("install_node22_runtime"));
}

export async function installCodexCli(): Promise<ModelCliInstallResult> {
  return parseModelCliInstallResult(await invokeDesktopCommand("install_codex_cli"));
}

export async function installClaudeCodeCli(): Promise<ModelCliInstallResult> {
  return parseModelCliInstallResult(await invokeDesktopCommand("install_claude_code_cli"));
}

export async function installAllModelClis(): Promise<ModelCliInstallResult> {
  return parseModelCliInstallResult(await invokeDesktopCommand("install_all_model_clis"));
}

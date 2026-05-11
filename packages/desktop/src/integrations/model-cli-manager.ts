import { existsSync, readdirSync } from "node:fs";
import { cp, mkdir, rm, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { arch } from "node:process";
import { execFile } from "node:child_process";
import path from "node:path";
import { promisify } from "node:util";
import log from "electron-log/main";
import { execCommand, resolvePaseoHome } from "@getpaseo/server";
import { patchClaudeCodeGitBashPathForWindows } from "../features/provider-switch.js";

export const REQUIRED_NODE_MAJOR = 22;
export const CODEX_PACKAGE_NAME = "@openai/codex";
export const CLAUDE_CODE_PACKAGE_NAME = "@anthropic-ai/claude-code";
const WINDOWS_NODE_MIRROR_URL = "https://registry.npmmirror.com/-/binary/node/latest-v22.x/";
const WINDOWS_GIT_MIRROR_URL = "https://registry.npmmirror.com/-/binary/git-for-windows/";
const NPMMIRROR_REGISTRY_URL = "https://registry.npmmirror.com";
const WINDOWS_GIT_WINGET_PACKAGE_ID = "Git.Git";
const WINDOWS_GIT_DIRECT_DOWNLOAD_URL =
  "https://github.com/git-for-windows/git/releases/latest/download/Git-64-bit.exe";
const execFileAsync = promisify(execFile);

type RuntimeManagerId = "nvm" | "brew" | "managed" | "shell";

export interface NodeRuntimeStatus {
  installed: boolean;
  version: string | null;
  major: number | null;
  npmVersion: string | null;
  satisfies: boolean;
  manager: RuntimeManagerId;
  error: string | null;
}

export interface ModelCliStatus {
  command: "codex" | "claude";
  packageName: string;
  installed: boolean;
  version: string | null;
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
  codex: ModelCliStatus;
  claude: ModelCliStatus;
}

export interface ModelCliInstallResult {
  status: ModelCliRuntimeStatus;
  output: string;
}

interface CommandResult {
  stdout: string;
  stderr: string;
}

interface WindowsGitPathSnapshot {
  installDir: string;
  gitCmdPath: string;
  gitBinPath: string;
  gitBashPath: string;
  bashBinPath: string;
  bashUsrPath: string;
  exists: Record<string, boolean>;
}

interface ShellOptions {
  gitBashPath?: string | null;
  forceWindowsCmd?: boolean;
  env?: NodeJS.ProcessEnv;
}

export interface MirrorDirectoryEntry {
  type?: string;
  name?: string;
  url?: string;
  modified?: string;
}

function getErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function simplifyInstallErrorMessage(message: string): string {
  const normalized = message.replace(/\s+/g, " ").trim();
  if (normalized.startsWith("Git Bash setup failed.") && normalized.length <= 900) {
    return normalized;
  }
  if (normalized.length <= 220) {
    return normalized;
  }
  return `${normalized.slice(0, 217).trimEnd()}...`;
}

function getMissingRuntimeDependencyNames(status: ModelCliRuntimeStatus): string[] {
  const missing: string[] = [];
  if (!status.git.installed) missing.push("Git Bash");
  if (!status.node.installed || !status.node.satisfies) missing.push("Node.js 22");
  if (!status.claude.installed) missing.push("Claude Code");
  if (!status.codex.installed) missing.push("Codex");
  return missing;
}

async function buildInstallFailureError(error: unknown): Promise<Error> {
  const status = await getModelCliRuntimeStatus().catch(() => null);
  const missing = status ? getMissingRuntimeDependencyNames(status) : [];
  const message = simplifyInstallErrorMessage(getErrorMessage(error));
  const missingText = missing.length > 0 ? ` Missing: ${missing.join(", ")}` : "";
  return new Error(`Install failed: ${message}${missingText}`);
}

function simplifyAttemptMessage(message: string): string {
  const normalized = message.replace(/\s+/g, " ").trim();
  if (normalized.length <= 260) {
    return normalized;
  }
  return `${normalized.slice(0, 257).trimEnd()}...`;
}

function commandOutputTail(value: string | null | undefined): string | null {
  const trimmed = trimToNull(value);
  if (!trimmed) {
    return null;
  }
  return trimmed.length <= 1200 ? trimmed : trimmed.slice(-1200);
}

export function buildWindowsGitInstallFailureMessage(
  errors: string[],
  status: GitRuntimeStatus,
): string {
  const validation = status.error ?? "Git Bash was not detected after installation.";
  const attempts = errors
    .map((entry) => simplifyAttemptMessage(entry))
    .filter((entry) => entry.length > 0);
  const attemptsText = attempts.length > 0 ? ` Attempts: ${attempts.join(" | ")}` : "";
  return `Git Bash setup failed. ${validation}${attemptsText}`;
}

function trimToNull(value: string | null | undefined): string | null {
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

export function parseMajorVersion(version: string | null): number | null {
  const match = version?.match(/(\d+)(?:\.\d+){0,2}/);
  if (!match) {
    return null;
  }
  const major = Number.parseInt(match[1] ?? "", 10);
  return Number.isFinite(major) ? major : null;
}

export function parseSemanticVersion(output: string | null): string | null {
  const trimmed = trimToNull(output);
  if (!trimmed) {
    return null;
  }
  const match = trimmed.match(/\d+\.\d+\.\d+(?:[-+._0-9A-Za-z]*)?/);
  return match?.[0] ?? trimmed;
}

function getNvmScriptPath(): string {
  return path.join(homedir(), ".nvm", "nvm.sh");
}

function buildShellCommand(command: string): { command: string; args: string[] } {
  if (process.platform === "win32") {
    return { command: "cmd.exe", args: ["/c", command] };
  }
  return { command: "/bin/bash", args: ["-lc", command] };
}

function buildGitBashCommand(
  command: string,
  gitBashPath: string,
): { command: string; args: string[] } {
  return { command: gitBashPath, args: ["-lc", command] };
}

export function shouldUseWindowsGitBash(
  options?: ShellOptions,
  platform: NodeJS.Platform = process.platform,
): boolean {
  return (
    platform === "win32" && !options?.forceWindowsCmd && isWindowsGitBashPath(options?.gitBashPath)
  );
}

async function runShell(command: string, options?: ShellOptions): Promise<CommandResult> {
  const shell = shouldUseWindowsGitBash(options)
    ? buildGitBashCommand(command, options?.gitBashPath ?? "")
    : buildShellCommand(command);
  return await execCommand(shell.command, shell.args, {
    env: options?.env ?? process.env,
    timeout: 10 * 60 * 1000,
    maxBuffer: 16 * 1024 * 1024,
  });
}

async function tryRunShell(command: string, options?: ShellOptions): Promise<CommandResult | null> {
  try {
    return await runShell(command, options);
  } catch {
    return null;
  }
}

async function commandExists(command: string): Promise<boolean> {
  if (process.platform === "win32") {
    return (
      (await tryRunShell(`where ${command}`, {
        forceWindowsCmd: true,
        env: buildWindowsCliSearchEnv(),
      })) !== null
    );
  }

  return (await tryRunShell(`command -v ${command} >/dev/null 2>&1`)) !== null;
}

async function resolveRuntimeManager(): Promise<RuntimeManagerId> {
  if (process.platform !== "win32" && existsSync(getNvmScriptPath())) {
    return "nvm";
  }
  if (process.platform === "darwin" && (await commandExists("brew"))) {
    return "brew";
  }
  if (process.platform === "darwin" && existsSync(resolveManagedNodeBinPath("node"))) {
    return "managed";
  }
  return "shell";
}

async function resolveWindowsGitBashPath(): Promise<string | null> {
  if (process.platform !== "win32") {
    return null;
  }

  const whereResult = await tryRunShell("where bash", {
    forceWindowsCmd: true,
    env: buildWindowsCliSearchEnv(),
  });
  const detected =
    whereResult?.stdout
      .split(/\r?\n/)
      .map((entry) => entry.trim())
      .find((entry) => isWindowsGitBashPath(entry)) ?? null;
  if (detected) {
    return detected;
  }

  const fallbackCandidates = [
    path.win32.join(resolveWindowsPortableGitDir(process.env), "bin", "bash.exe"),
    path.win32.join(resolveWindowsPortableGitDir(process.env), "usr", "bin", "bash.exe"),
    path.join(process.env.ProgramFiles ?? "C:\\Program Files", "Git", "bin", "bash.exe"),
    path.join(process.env.ProgramFiles ?? "C:\\Program Files", "Git", "usr", "bin", "bash.exe"),
    path.join(
      process.env["ProgramFiles(x86)"] ?? "C:\\Program Files (x86)",
      "Git",
      "bin",
      "bash.exe",
    ),
    path.join(
      process.env["ProgramFiles(x86)"] ?? "C:\\Program Files (x86)",
      "Git",
      "usr",
      "bin",
      "bash.exe",
    ),
    path.win32.join(
      process.env.LOCALAPPDATA ??
        path.win32.join(process.env.USERPROFILE ?? "", "AppData", "Local"),
      "Programs",
      "Git",
      "bin",
      "bash.exe",
    ),
    path.win32.join(
      process.env.LOCALAPPDATA ??
        path.win32.join(process.env.USERPROFILE ?? "", "AppData", "Local"),
      "Programs",
      "Git",
      "usr",
      "bin",
      "bash.exe",
    ),
    path.join(process.env.USERPROFILE ?? "", "scoop", "apps", "git", "current", "bin", "bash.exe"),
    path.join(
      process.env.USERPROFILE ?? "",
      "scoop",
      "apps",
      "git",
      "current",
      "usr",
      "bin",
      "bash.exe",
    ),
  ];

  return (
    fallbackCandidates.find((entry) => existsSync(entry) && isWindowsGitBashPath(entry)) ?? null
  );
}

async function resolveWindowsGitExecutablePath(): Promise<string | null> {
  if (process.platform !== "win32") {
    return null;
  }

  const whereResult = await tryRunShell("where git", {
    forceWindowsCmd: true,
    env: buildWindowsCliSearchEnv(),
  });
  const detected =
    whereResult?.stdout
      .split(/\r?\n/)
      .map((entry) => entry.trim())
      .find((entry) => entry.toLowerCase().endsWith(".exe") && existsSync(entry)) ?? null;
  if (detected) {
    return detected;
  }

  const fallbackCandidates = [
    path.win32.join(resolveWindowsPortableGitDir(process.env), "cmd", "git.exe"),
    path.win32.join(resolveWindowsPortableGitDir(process.env), "bin", "git.exe"),
    path.win32.join(process.env.ProgramFiles ?? "C:\\Program Files", "Git", "cmd", "git.exe"),
    path.win32.join(process.env.ProgramFiles ?? "C:\\Program Files", "Git", "bin", "git.exe"),
    path.win32.join(
      process.env["ProgramFiles(x86)"] ?? "C:\\Program Files (x86)",
      "Git",
      "cmd",
      "git.exe",
    ),
    path.win32.join(
      process.env["ProgramFiles(x86)"] ?? "C:\\Program Files (x86)",
      "Git",
      "bin",
      "git.exe",
    ),
    path.win32.join(
      process.env.LOCALAPPDATA ??
        path.win32.join(process.env.USERPROFILE ?? "", "AppData", "Local"),
      "Programs",
      "Git",
      "cmd",
      "git.exe",
    ),
    path.win32.join(
      process.env.LOCALAPPDATA ??
        path.win32.join(process.env.USERPROFILE ?? "", "AppData", "Local"),
      "Programs",
      "Git",
      "bin",
      "git.exe",
    ),
    path.win32.join(
      process.env.USERPROFILE ?? "",
      "scoop",
      "apps",
      "git",
      "current",
      "cmd",
      "git.exe",
    ),
    path.win32.join(
      process.env.USERPROFILE ?? "",
      "scoop",
      "apps",
      "git",
      "current",
      "bin",
      "git.exe",
    ),
  ];

  return fallbackCandidates.find((entry) => existsSync(entry)) ?? null;
}

function normalizeWindowsPath(value: string): string {
  return value.trim().replace(/\\/g, "/").toLowerCase();
}

export function isWindowsGitBashPath(value: string | null | undefined): boolean {
  const normalized = value ? normalizeWindowsPath(value) : "";
  if (!normalized.endsWith("/bash.exe")) {
    return false;
  }
  if (
    normalized.endsWith("/windows/system32/bash.exe") ||
    normalized.endsWith("/windows/syswow64/bash.exe") ||
    normalized.includes("/windows/system32/wsl") ||
    normalized.includes("/windows/syswow64/wsl") ||
    normalized.includes("/mingit-")
  ) {
    return false;
  }

  return (
    normalized.endsWith("/git/bin/bash.exe") ||
    normalized.endsWith("/git/usr/bin/bash.exe") ||
    normalized.endsWith("/portablegit/bin/bash.exe") ||
    normalized.endsWith("/portablegit/usr/bin/bash.exe") ||
    normalized.endsWith("/scoop/apps/git/current/bin/bash.exe") ||
    normalized.endsWith("/scoop/apps/git/current/usr/bin/bash.exe")
  );
}

export function buildWindowsCliExecutableCandidates(command: "codex" | "claude"): string[] {
  return [`${command}.cmd`, `${command}.exe`, command];
}

function appendUniquePath(paths: string[], value: string | null | undefined): void {
  const trimmed = value?.trim();
  if (!trimmed) {
    return;
  }
  const normalized = normalizeWindowsPath(trimmed);
  if (paths.some((entry) => normalizeWindowsPath(entry) === normalized)) {
    return;
  }
  paths.push(trimmed);
}

function resolveWindowsPaseoHome(env: NodeJS.ProcessEnv = process.env): string {
  const configured = env.PASEO_HOME?.trim();
  if (configured) {
    return configured;
  }
  return resolvePaseoHome(env);
}

function resolveWindowsPortableGitDir(env: NodeJS.ProcessEnv = process.env): string {
  return path.win32.join(resolveWindowsPaseoHome(env), "toolchains", "PortableGit");
}

export function resolveWindowsManagedNodeDir(env: NodeJS.ProcessEnv = process.env): string {
  return path.win32.join(resolveWindowsPaseoHome(env), "toolchains", "node22-win-x64");
}

function createWindowsGitPathSnapshot(installDir: string): WindowsGitPathSnapshot {
  const gitCmdPath = path.win32.join(installDir, "cmd", "git.exe");
  const gitBinPath = path.win32.join(installDir, "bin", "git.exe");
  const gitBashPath = path.win32.join(installDir, "git-bash.exe");
  const bashBinPath = path.win32.join(installDir, "bin", "bash.exe");
  const bashUsrPath = path.win32.join(installDir, "usr", "bin", "bash.exe");
  return {
    installDir,
    gitCmdPath,
    gitBinPath,
    gitBashPath,
    bashBinPath,
    bashUsrPath,
    exists: {
      gitCmdPath: existsSync(gitCmdPath),
      gitBinPath: existsSync(gitBinPath),
      gitBashPath: existsSync(gitBashPath),
      bashBinPath: existsSync(bashBinPath),
      bashUsrPath: existsSync(bashUsrPath),
    },
  };
}

export function resolveWindowsPortableGitTempFallbackDirs(
  installerPath: string,
  env: NodeJS.ProcessEnv = process.env,
): string[] {
  const dirs: string[] = [];
  appendUniquePath(dirs, path.win32.join(path.win32.dirname(installerPath), "PortableGit"));
  appendUniquePath(dirs, env.TEMP ? path.win32.join(env.TEMP, "PortableGit") : null);
  appendUniquePath(dirs, env.TMP ? path.win32.join(env.TMP, "PortableGit") : null);
  appendUniquePath(
    dirs,
    env.LOCALAPPDATA ? path.win32.join(env.LOCALAPPDATA, "Temp", "PortableGit") : null,
  );
  return dirs;
}

function resolveManagedNodeDir(): string {
  return path.join(resolvePaseoHome(process.env), "toolchains", "node22");
}

function resolveManagedNodeBinPath(command: "node" | "npm"): string {
  return path.join(resolveManagedNodeDir(), "bin", command);
}

export function buildWindowsCliSearchPath(env: NodeJS.ProcessEnv = process.env): string {
  const paths: string[] = [];
  appendUniquePath(paths, env.APPDATA ? path.win32.join(env.APPDATA, "npm") : null);
  appendUniquePath(paths, resolveWindowsManagedNodeDir(env));
  appendUniquePath(paths, env.ProgramFiles ? path.win32.join(env.ProgramFiles, "nodejs") : null);
  appendUniquePath(paths, path.win32.join(resolveWindowsPortableGitDir(env), "cmd"));
  appendUniquePath(paths, path.win32.join(resolveWindowsPortableGitDir(env), "bin"));
  appendUniquePath(paths, path.win32.join(resolveWindowsPortableGitDir(env), "usr", "bin"));
  appendUniquePath(
    paths,
    env.ProgramFiles ? path.win32.join(env.ProgramFiles, "Git", "cmd") : null,
  );
  appendUniquePath(
    paths,
    env.ProgramFiles ? path.win32.join(env.ProgramFiles, "Git", "bin") : null,
  );
  appendUniquePath(
    paths,
    env.ProgramFiles ? path.win32.join(env.ProgramFiles, "Git", "usr", "bin") : null,
  );
  appendUniquePath(
    paths,
    env["ProgramFiles(x86)"] ? path.win32.join(env["ProgramFiles(x86)"], "Git", "cmd") : null,
  );
  appendUniquePath(
    paths,
    env["ProgramFiles(x86)"] ? path.win32.join(env["ProgramFiles(x86)"], "Git", "bin") : null,
  );
  appendUniquePath(
    paths,
    env["ProgramFiles(x86)"]
      ? path.win32.join(env["ProgramFiles(x86)"], "Git", "usr", "bin")
      : null,
  );
  appendUniquePath(
    paths,
    env.LOCALAPPDATA ? path.win32.join(env.LOCALAPPDATA, "Programs", "Git", "cmd") : null,
  );
  appendUniquePath(
    paths,
    env.LOCALAPPDATA ? path.win32.join(env.LOCALAPPDATA, "Programs", "Git", "bin") : null,
  );
  appendUniquePath(
    paths,
    env.LOCALAPPDATA ? path.win32.join(env.LOCALAPPDATA, "Programs", "Git", "usr", "bin") : null,
  );
  appendUniquePath(
    paths,
    env.USERPROFILE
      ? path.win32.join(env.USERPROFILE, "scoop", "apps", "git", "current", "cmd")
      : null,
  );
  appendUniquePath(
    paths,
    env.USERPROFILE
      ? path.win32.join(env.USERPROFILE, "scoop", "apps", "git", "current", "bin")
      : null,
  );
  appendUniquePath(
    paths,
    env.USERPROFILE
      ? path.win32.join(env.USERPROFILE, "scoop", "apps", "git", "current", "usr", "bin")
      : null,
  );
  const currentPath = env.PATH ?? env.Path ?? env.path ?? "";
  for (const entry of currentPath.split(";")) {
    appendUniquePath(paths, entry);
  }
  return paths.join(";");
}

export function buildWindowsUserPathValue(currentPath: string, entriesToAdd: string[]): string {
  const entries: string[] = [];
  for (const entry of currentPath.split(";")) {
    appendUniquePath(entries, entry);
  }
  for (const entry of entriesToAdd) {
    appendUniquePath(entries, entry);
  }
  return entries.join(";");
}

export function resolveWindowsNpmGlobalBinPathFromPrefix(prefixOutput: string): string | null {
  const prefix = trimToNull(prefixOutput);
  if (!prefix) {
    return null;
  }
  const normalized = prefix.replace(/[\\/]+node_modules[\\/]?$/i, "");
  return normalized || null;
}

function buildWindowsCliSearchEnv(env: NodeJS.ProcessEnv = process.env): NodeJS.ProcessEnv {
  const searchPath = buildWindowsCliSearchPath(env);
  return {
    ...env,
    PATH: searchPath,
    Path: searchPath,
  };
}

function buildManagedNodeEnv(env: NodeJS.ProcessEnv = process.env): NodeJS.ProcessEnv {
  const managedNodeBin = path.join(resolveManagedNodeDir(), "bin");
  const currentPath = env.PATH ?? "";
  return {
    ...env,
    PATH: currentPath ? `${managedNodeBin}:${currentPath}` : managedNodeBin,
  };
}

async function tryRunWindowsExecutable(
  command: string,
  args: string[],
  env: NodeJS.ProcessEnv,
): Promise<CommandResult | null> {
  try {
    return await execCommand(command, args, {
      env,
      timeout: 10 * 60 * 1000,
      maxBuffer: 16 * 1024 * 1024,
    });
  } catch {
    return null;
  }
}

export function wrapWithRuntimeManager(command: string, manager: RuntimeManagerId): string {
  if (manager === "managed") {
    return `export PATH="${path.join(resolveManagedNodeDir(), "bin")}:$PATH"; ${command}`;
  }
  if (manager === "nvm") {
    const nvmScriptPath = getNvmScriptPath();
    return `export NVM_DIR="${path.dirname(nvmScriptPath)}"; . "${nvmScriptPath}"; nvm use default >/dev/null 2>&1 || true; if [ -n "$NVM_BIN" ]; then export PATH="$NVM_BIN:$PATH"; fi; ${command}`;
  }
  if (manager === "brew") {
    return `BREW_NODE_PREFIX="$(brew --prefix node@22 2>/dev/null || true)"; if [ -n "$BREW_NODE_PREFIX" ]; then export PATH="$BREW_NODE_PREFIX/bin:$PATH"; fi; ${command}`;
  }
  return command;
}

export function wrapWithNode22Runtime(command: string, manager: RuntimeManagerId): string {
  if (manager === "managed") {
    return `export PATH="${path.join(resolveManagedNodeDir(), "bin")}:$PATH"; ${command}`;
  }
  if (manager === "nvm") {
    const nvmScriptPath = getNvmScriptPath();
    return `export NVM_DIR="${path.dirname(nvmScriptPath)}"; . "${nvmScriptPath}"; nvm install ${REQUIRED_NODE_MAJOR}; nvm alias default ${REQUIRED_NODE_MAJOR}; nvm use ${REQUIRED_NODE_MAJOR} >/dev/null; if [ -n "$NVM_BIN" ]; then export PATH="$NVM_BIN:$PATH"; fi; ${command}`;
  }
  if (manager === "brew") {
    return `brew install node@${REQUIRED_NODE_MAJOR}; BREW_NODE_PREFIX="$(brew --prefix node@${REQUIRED_NODE_MAJOR})"; export PATH="$BREW_NODE_PREFIX/bin:$PATH"; ${command}`;
  }
  return command;
}

export function buildWindowsGitBashInstallCommand(): string {
  return `winget install --id ${WINDOWS_GIT_WINGET_PACKAGE_ID} -e --accept-package-agreements --accept-source-agreements`;
}

export function buildWindowsGitBashChocolateyInstallCommand(): string {
  return "choco install git -y --no-progress";
}

export function buildWindowsGitBashScoopInstallCommand(): string {
  return "scoop install git";
}

function quotePowerShellString(value: string): string {
  return `'${value.replace(/'/g, "''")}'`;
}

function quoteShellString(value: string): string {
  return `'${value.replace(/'/g, "'\\''")}'`;
}

export function buildWindowsNodeDirectInstallCommand(installerUrl: string): string {
  const quotedUrl = quotePowerShellString(installerUrl);
  return `powershell -NoProfile -ExecutionPolicy Bypass -Command "$ErrorActionPreference='Stop'; $installerUrl=${quotedUrl}; $installerPath=Join-Path $env:TEMP ('paseo-node-installer-' + [Guid]::NewGuid().ToString('N') + '.msi'); $logPath=Join-Path $env:TEMP ('paseo-node-installer-' + [Guid]::NewGuid().ToString('N') + '.log'); try { Invoke-WebRequest -Uri $installerUrl -OutFile $installerPath -UseBasicParsing -TimeoutSec 60; $process=Start-Process -FilePath 'msiexec.exe' -ArgumentList '/i',$installerPath,'/qn','/norestart','/L*v',$logPath -Wait -PassThru; if ($process.ExitCode -ne 0) { throw ('Node.js installer failed with exit code ' + $process.ExitCode + '. Log: ' + $logPath) }; $nodePath='C:\\\\Program Files\\\\nodejs\\\\node.exe'; $npmPath='C:\\\\Program Files\\\\nodejs\\\\npm.cmd'; if (Test-Path $nodePath) { & $nodePath --version } else { node --version }; if (Test-Path $npmPath) { & $npmPath --version } else { npm --version } } catch { throw ('Node.js mirror installer failed: ' + $_.Exception.Message) } finally { Remove-Item -Path $installerPath -Force -ErrorAction SilentlyContinue }"`;
}

export function buildWindowsNodeZipExtractPowerShellArgs(
  zipPath: string,
  destinationPath: string,
): string[] {
  return [
    "-NoProfile",
    "-ExecutionPolicy",
    "Bypass",
    "-Command",
    `$ErrorActionPreference='Stop'; Expand-Archive -LiteralPath ${quotePowerShellString(zipPath)} -DestinationPath ${quotePowerShellString(destinationPath)} -Force`,
  ];
}

export function buildMacOSNodeDirectInstallCommand(tarballUrl: string, installDir: string): string {
  const quotedUrl = quoteShellString(tarballUrl);
  const quotedInstallDir = quoteShellString(installDir);
  const quotedNodePath = quoteShellString(path.join(installDir, "bin", "node"));
  const quotedNpmPath = quoteShellString(path.join(installDir, "bin", "npm"));
  return `set -euo pipefail; url=${quotedUrl}; install_dir=${quotedInstallDir}; archive="$(mktemp -t paseo-node.XXXXXX.tar.gz)"; cleanup() { rm -f "$archive"; }; trap cleanup EXIT; rm -rf "$install_dir"; mkdir -p "$install_dir"; curl -fL --connect-timeout 20 --retry 2 --retry-delay 1 "$url" -o "$archive"; tar -xzf "$archive" -C "$install_dir" --strip-components 1; ${quotedNodePath} --version; ${quotedNpmPath} --version`;
}

export function buildWindowsGitBashMirrorInstallCommand(installerUrl: string): string {
  const quotedUrl = quotePowerShellString(installerUrl);
  return `powershell -NoProfile -ExecutionPolicy Bypass -Command "$ErrorActionPreference='Stop'; $installerUrl=${quotedUrl}; $installerPath=Join-Path $env:TEMP ('paseo-git-installer-' + [Guid]::NewGuid().ToString('N') + '.exe'); try { Invoke-WebRequest -Uri $installerUrl -OutFile $installerPath -UseBasicParsing -TimeoutSec 60; $installerArgs=@('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/SP-','/NOCANCEL','/CURRENTUSER','/CLOSEAPPLICATIONS','/RESTARTAPPLICATIONS','/o:PathOption=Cmd','/o:BashTerminalOption=MinTTY'); $process=Start-Process -FilePath $installerPath -ArgumentList $installerArgs -Wait -PassThru; if ($process.ExitCode -ne 0) { throw ('Git for Windows installer failed with exit code ' + $process.ExitCode) } } catch { throw ('Git mirror installer failed: ' + $_.Exception.Message) } finally { Remove-Item -Path $installerPath -Force -ErrorAction SilentlyContinue }"`;
}

export function buildWindowsGitBashPortableExtractArgs(installDir: string): string[] {
  const escapedInstallDir = installDir.replace(/\\/g, "\\\\");
  return ["-y", "-gm2", `-InstallPath=${escapedInstallDir}`];
}

export function buildWindowsGitBashDirectInstallCommand(): string {
  return buildWindowsGitBashMirrorInstallCommand(WINDOWS_GIT_DIRECT_DOWNLOAD_URL);
}

async function downloadFileWithFetch(url: string, destination: string): Promise<void> {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`Download failed with HTTP ${response.status}: ${url}`);
  }
  await mkdir(path.dirname(destination), { recursive: true });
  await writeFile(destination, Buffer.from(await response.arrayBuffer()));
}

function getWindowsExternalCliPathEntries(env: NodeJS.ProcessEnv = process.env): string[] {
  const managedNodeDir = resolveWindowsManagedNodeDir(env);
  const portableGitDir = resolveWindowsPortableGitDir(env);
  return [
    existsSync(path.win32.join(managedNodeDir, "node.exe")) ? managedNodeDir : null,
    env.APPDATA ? path.win32.join(env.APPDATA, "npm") : null,
    existsSync(path.win32.join(portableGitDir, "cmd", "git.exe"))
      ? path.win32.join(portableGitDir, "cmd")
      : null,
    existsSync(path.win32.join(portableGitDir, "bin", "bash.exe"))
      ? path.win32.join(portableGitDir, "bin")
      : null,
    existsSync(path.win32.join(portableGitDir, "usr", "bin", "bash.exe"))
      ? path.win32.join(portableGitDir, "usr", "bin")
      : null,
  ].filter((entry): entry is string => typeof entry === "string" && entry.trim().length > 0);
}

async function ensureWindowsUserPathEntries(entriesToAdd: string[]): Promise<void> {
  if (process.platform !== "win32") {
    return;
  }
  const filtered = entriesToAdd.filter((entry) => entry.trim().length > 0);
  if (filtered.length === 0) {
    return;
  }

  const readResult = await tryRunShell(
    "powershell -NoProfile -ExecutionPolicy Bypass -Command \"[Environment]::GetEnvironmentVariable('Path', 'User')\"",
    { forceWindowsCmd: true, env: buildWindowsCliSearchEnv() },
  );
  const before = readResult?.stdout ?? "";
  const after = buildWindowsUserPathValue(before, filtered);
  if (normalizeWindowsPath(before) === normalizeWindowsPath(after)) {
    return;
  }

  await runShell(
    `powershell -NoProfile -ExecutionPolicy Bypass -Command "$ErrorActionPreference='Stop'; [Environment]::SetEnvironmentVariable('Path', ${quotePowerShellString(after)}, 'User')"`,
    { forceWindowsCmd: true, env: buildWindowsCliSearchEnv() },
  );
  const mergedPath = buildWindowsUserPathValue(
    process.env.PATH ?? process.env.Path ?? "",
    filtered,
  );
  process.env.PATH = mergedPath;
  process.env.Path = mergedPath;
  log.info("[model-cli-manager] updated Windows user PATH", {
    added: filtered,
    beforeLength: before.length,
    afterLength: after.length,
  });
}

async function resolveWindowsNpmGlobalBinPath(options?: ShellOptions): Promise<string | null> {
  const result = await tryRunShell("npm prefix -g", {
    ...options,
    forceWindowsCmd: true,
    env: buildWindowsCliSearchEnv(),
  });
  return resolveWindowsNpmGlobalBinPathFromPrefix(result?.stdout ?? result?.stderr ?? "");
}

async function execFileForInstall(
  command: string,
  args: string[],
  env: NodeJS.ProcessEnv = process.env,
): Promise<CommandResult & { exitCode: number }> {
  try {
    const result = await execFileAsync(command, args, {
      env,
      timeout: 10 * 60 * 1000,
      maxBuffer: 16 * 1024 * 1024,
      windowsHide: true,
    });
    return {
      stdout: result.stdout?.toString() ?? "",
      stderr: result.stderr?.toString() ?? "",
      exitCode: 0,
    };
  } catch (error) {
    const maybeError = error as Error & {
      stdout?: string | Buffer;
      stderr?: string | Buffer;
      code?: number | string;
    };
    const exitCode = typeof maybeError.code === "number" ? maybeError.code : 1;
    const stdout = maybeError.stdout?.toString() ?? "";
    const stderr = maybeError.stderr?.toString() ?? "";
    throw new Error(
      `exit code ${exitCode}${stderr ? `: ${commandOutputTail(stderr)}` : ""}${stdout ? ` stdout: ${commandOutputTail(stdout)}` : ""}`,
    );
  }
}

async function installWindowsNodeZipFromUrl(zipUrl: string): Promise<string> {
  const installDir = resolveWindowsManagedNodeDir(process.env);
  const stagingDir = path.win32.join(
    process.env.TEMP ?? process.env.TMP ?? path.win32.dirname(installDir),
    `paseo-node22-${Date.now()}`,
  );
  const zipPath = path.win32.join(stagingDir, "node22.zip");
  const outputs: string[] = [];

  log.info("[model-cli-manager] installing app-managed Windows Node.js from zip", {
    zipUrl,
    installDir,
    stagingDir,
    zipPath,
  });

  try {
    try {
      await mkdir(stagingDir, { recursive: true });
      await downloadFileWithFetch(zipUrl, zipPath);
      outputs.push(`Downloaded Node.js from ${zipUrl}`);
    } catch (error) {
      log.warn("[model-cli-manager] Node.js zip download failed", {
        zipUrl,
        zipPath,
        error: getErrorMessage(error),
      });
      throw new Error(`Node zip download failed: ${getErrorMessage(error)}`);
    }

    try {
      await execFileForInstall(
        "powershell.exe",
        buildWindowsNodeZipExtractPowerShellArgs(zipPath, stagingDir),
        buildWindowsCliSearchEnv(),
      );
    } catch (error) {
      log.warn("[model-cli-manager] Node.js zip extraction failed", {
        zipUrl,
        zipPath,
        stagingDir,
        error: getErrorMessage(error),
      });
      throw new Error(`Node zip extract failed: ${getErrorMessage(error)}`);
    }

    const extractedCandidates = [
      path.win32.join(stagingDir, path.win32.basename(new URL(zipUrl).pathname, ".zip")),
      path.win32.join(
        stagingDir,
        readdirSync(stagingDir).find((entry) => {
          const candidate = path.win32.join(stagingDir, entry);
          return existsSync(path.win32.join(candidate, "node.exe"));
        }) ?? "",
      ),
    ];
    const extractedDir =
      extractedCandidates.find((candidate) => existsSync(path.win32.join(candidate, "node.exe"))) ??
      "";

    if (!existsSync(path.win32.join(extractedDir, "node.exe"))) {
      throw new Error("Node zip verify failed: extracted archive did not contain node.exe");
    }

    await rm(installDir, { recursive: true, force: true });
    await mkdir(path.win32.dirname(installDir), { recursive: true });
    await cp(extractedDir, installDir, { recursive: true });

    const env = buildWindowsCliSearchEnv();
    const nodePath = path.win32.join(installDir, "node.exe");
    const npmPath = path.win32.join(installDir, "npm.cmd");
    const [nodeResult, npmResult] = await Promise.all([
      execFileForInstall(nodePath, ["--version"], env),
      execFileForInstall(npmPath, ["--version"], env),
    ]);
    outputs.push([nodeResult.stdout, nodeResult.stderr].filter(Boolean).join("\n").trim());
    outputs.push([npmResult.stdout, npmResult.stderr].filter(Boolean).join("\n").trim());
    await ensureWindowsUserPathEntries(getWindowsExternalCliPathEntries(process.env));
    return outputs.filter(Boolean).join("\n").trim();
  } finally {
    await rm(stagingDir, { recursive: true, force: true }).catch(() => undefined);
  }
}

function resolvePortableGitExecutables(snapshot: WindowsGitPathSnapshot): {
  gitPath: string | null;
  bashPath: string | null;
} {
  return {
    gitPath: snapshot.exists.gitCmdPath
      ? snapshot.gitCmdPath
      : snapshot.exists.gitBinPath
        ? snapshot.gitBinPath
        : null,
    bashPath: snapshot.exists.bashBinPath
      ? snapshot.bashBinPath
      : snapshot.exists.bashUsrPath
        ? snapshot.bashUsrPath
        : null,
  };
}

async function adoptPortableGitFromFallbackDir(
  fallbackDir: string,
  installDir: string,
): Promise<boolean> {
  const fallbackSnapshot = createWindowsGitPathSnapshot(fallbackDir);
  const fallbackExecutables = resolvePortableGitExecutables(fallbackSnapshot);
  if (!fallbackExecutables.gitPath || !fallbackExecutables.bashPath) {
    return false;
  }

  await rm(installDir, { recursive: true, force: true });
  await mkdir(path.win32.dirname(installDir), { recursive: true });
  await cp(fallbackDir, installDir, { recursive: true });
  return true;
}

async function installPortableGitFromUrl(
  installerUrl: string,
  installDir: string,
): Promise<string> {
  const installerPath = path.win32.join(
    process.env.TEMP ?? process.env.TMP ?? installDir,
    `paseo-portable-git-${Date.now()}.7z.exe`,
  );
  const outputs: string[] = [];
  log.info("[model-cli-manager] installing app-managed PortableGit", {
    installerUrl,
    installDir,
    installerPath,
  });

  try {
    try {
      await downloadFileWithFetch(installerUrl, installerPath);
      outputs.push(`Downloaded PortableGit from ${installerUrl}`);
    } catch (error) {
      log.warn("[model-cli-manager] PortableGit download failed", {
        installerUrl,
        installerPath,
        error: getErrorMessage(error),
      });
      throw new Error(`PortableGit download: ${getErrorMessage(error)}`);
    }

    await rm(installDir, { recursive: true, force: true });
    await mkdir(installDir, { recursive: true });

    const extractArgs = buildWindowsGitBashPortableExtractArgs(installDir);
    let extractResult: CommandResult & { exitCode: number };
    try {
      extractResult = await execFileForInstall(
        installerPath,
        extractArgs,
        buildWindowsCliSearchEnv(),
      );
      outputs.push([extractResult.stdout, extractResult.stderr].filter(Boolean).join("\n").trim());
    } catch (error) {
      const snapshot = createWindowsGitPathSnapshot(installDir);
      log.warn("[model-cli-manager] PortableGit extraction failed", {
        installerPath,
        installDir,
        extractArgs,
        snapshot,
        error: getErrorMessage(error),
      });
      throw new Error(`PortableGit extract: ${getErrorMessage(error)}`);
    }

    let snapshot = createWindowsGitPathSnapshot(installDir);
    let executables = resolvePortableGitExecutables(snapshot);
    if (!executables.gitPath || !executables.bashPath) {
      const fallbackDirs = resolveWindowsPortableGitTempFallbackDirs(installerPath, process.env);
      const fallbackSnapshots = fallbackDirs.map((dir) => createWindowsGitPathSnapshot(dir));
      const adoptedFrom = await (async () => {
        for (const fallbackDir of fallbackDirs) {
          if (await adoptPortableGitFromFallbackDir(fallbackDir, installDir)) {
            return fallbackDir;
          }
        }
        return null;
      })();
      if (adoptedFrom) {
        snapshot = createWindowsGitPathSnapshot(installDir);
        executables = resolvePortableGitExecutables(snapshot);
        outputs.push(
          `PortableGit extracted to temporary directory and was moved from ${adoptedFrom}`,
        );
        log.info("[model-cli-manager] adopted PortableGit from temporary extraction directory", {
          installerUrl,
          installDir,
          adoptedFrom,
          snapshot,
        });
      }
      if (executables.gitPath && executables.bashPath) {
        // Continue with the normal bash validation below.
      } else {
        log.warn("[model-cli-manager] PortableGit verification paths missing", {
          installerUrl,
          installDir,
          snapshot,
          fallbackSnapshots,
          extractExitCode: extractResult.exitCode,
          stdoutTail: commandOutputTail(extractResult.stdout),
          stderrTail: commandOutputTail(extractResult.stderr),
        });
        throw new Error(
          "PortableGit verify: PortableGit extraction did not create git.exe or bash.exe in the target or temporary extraction directories",
        );
      }
    }

    let bashResult: CommandResult;
    try {
      bashResult = (await execFileAsync(executables.bashPath, ["-lc", "echo ok && git --version"], {
        env: buildWindowsCliSearchEnv(),
        timeout: 10 * 60 * 1000,
        maxBuffer: 16 * 1024 * 1024,
        windowsHide: true,
      })) as CommandResult;
    } catch (error) {
      log.warn("[model-cli-manager] PortableGit Bash validation failed", {
        installerUrl,
        installDir,
        snapshot,
        bashPath: executables.bashPath,
        error: getErrorMessage(error),
      });
      throw new Error(`PortableGit verify: ${getErrorMessage(error)}`);
    }

    log.info("[model-cli-manager] PortableGit is ready", {
      installDir,
      gitPath: executables.gitPath,
      bashPath: executables.bashPath,
      bashStdoutTail: commandOutputTail(bashResult.stdout),
      bashStderrTail: commandOutputTail(bashResult.stderr),
    });
    outputs.push(`PortableGit installed to ${installDir}`);
    outputs.push(`Git executable: ${executables.gitPath}`);
    outputs.push(`Git Bash executable: ${executables.bashPath}`);
    outputs.push([bashResult.stdout, bashResult.stderr].filter(Boolean).join("\n").trim());
    return outputs.filter(Boolean).join("\n").trim();
  } finally {
    await rm(installerPath, { force: true }).catch(() => undefined);
  }
}

export function buildWindowsNpmPackageInstallCommand(
  packageName: string,
  registry: "npmmirror" | "official",
): string {
  const baseCommand = `npm install -g ${packageName}@latest`;
  if (registry === "official") {
    return baseCommand;
  }
  return `${baseCommand} --registry=${NPMMIRROR_REGISTRY_URL} --fetch-retries=2 --fetch-timeout=60000`;
}

function parseVersionParts(value: string): number[] {
  return value.split(".").map((part) => Number.parseInt(part, 10) || 0);
}

function compareVersionStrings(left: string, right: string): number {
  const leftParts = parseVersionParts(left);
  const rightParts = parseVersionParts(right);
  const max = Math.max(leftParts.length, rightParts.length);
  for (let index = 0; index < max; index += 1) {
    const diff = (leftParts[index] ?? 0) - (rightParts[index] ?? 0);
    if (diff !== 0) {
      return diff;
    }
  }
  return 0;
}

export function resolveLatestNode22WindowsMsiUrlFromMirror(
  entries: MirrorDirectoryEntry[],
): string | null {
  const candidates = entries
    .map((entry) => {
      const name = entry.name ?? "";
      const match = name.match(/^node-v(22\.\d+\.\d+)-x64\.msi$/i);
      if (!match || entry.type === "dir" || !entry.url) {
        return null;
      }
      return { version: match[1]!, url: entry.url };
    })
    .filter((entry): entry is { version: string; url: string } => entry !== null)
    .sort((left, right) => compareVersionStrings(right.version, left.version));

  return candidates[0]?.url ?? null;
}

export function resolveLatestNode22WindowsZipUrlFromMirror(
  entries: MirrorDirectoryEntry[],
): string | null {
  const candidates = entries
    .map((entry) => {
      const name = entry.name ?? "";
      const match = name.match(/^node-v(22\.\d+\.\d+)-win-x64\.zip$/i);
      if (!match || entry.type === "dir" || !entry.url) {
        return null;
      }
      return { version: match[1]!, url: entry.url };
    })
    .filter((entry): entry is { version: string; url: string } => entry !== null)
    .sort((left, right) => compareVersionStrings(right.version, left.version));

  return candidates[0]?.url ?? null;
}

export function resolveLatestNode22DarwinTarballUrlFromMirror(
  entries: MirrorDirectoryEntry[],
  nodeArch: "arm64" | "x64" = arch === "arm64" ? "arm64" : "x64",
): string | null {
  const candidates = entries
    .map((entry) => {
      const name = entry.name ?? "";
      const match = name.match(
        new RegExp(`^node-v(22\\.\\d+\\.\\d+)-darwin-${nodeArch}\\.tar\\.gz$`, "i"),
      );
      if (!match || entry.type === "dir" || !entry.url) {
        return null;
      }
      return { version: match[1]!, url: entry.url };
    })
    .filter((entry): entry is { version: string; url: string } => entry !== null)
    .sort((left, right) => compareVersionStrings(right.version, left.version));

  return candidates[0]?.url ?? null;
}

function parseGitForWindowsReleaseVersion(name: string): string | null {
  const match = name.match(/^v(\d+\.\d+\.\d+)\.windows\.(\d+)\/?$/i);
  if (!match) {
    return null;
  }
  return `${match[1]}.${match[2]}`;
}

export function resolveLatestGitForWindowsInstallerUrlFromMirror(
  releaseDirs: MirrorDirectoryEntry[],
  releaseEntries: MirrorDirectoryEntry[],
): string | null {
  const latestRelease = releaseDirs
    .map((entry) => {
      const name = entry.name ?? "";
      const version = parseGitForWindowsReleaseVersion(name);
      if (!version || entry.type !== "dir") {
        return null;
      }
      return { version, url: entry.url ?? "" };
    })
    .filter((entry): entry is { version: string; url: string } => entry !== null)
    .sort((left, right) => compareVersionStrings(right.version, left.version))[0];

  const installerCandidates = releaseEntries
    .filter((entry) => {
      const name = entry.name ?? "";
      if (entry.type === "dir" || !entry.url || !/^Git-\d+\.\d+\.\d+-64-bit\.exe$/i.test(name)) {
        return false;
      }
      return !latestRelease?.url || entry.url.startsWith(latestRelease.url);
    })
    .map((entry) => entry.url!)
    .sort();

  return installerCandidates[0] ?? null;
}

export function resolveLatestGitForWindowsPortableUrlFromMirror(
  releaseDirs: MirrorDirectoryEntry[],
  releaseEntries: MirrorDirectoryEntry[],
): string | null {
  const latestRelease = releaseDirs
    .map((entry) => {
      const name = entry.name ?? "";
      const version = parseGitForWindowsReleaseVersion(name);
      if (!version || entry.type !== "dir") {
        return null;
      }
      return { version, url: entry.url ?? "" };
    })
    .filter((entry): entry is { version: string; url: string } => entry !== null)
    .sort((left, right) => compareVersionStrings(right.version, left.version))[0];

  const portableCandidates = releaseEntries
    .filter((entry) => {
      const name = entry.name ?? "";
      if (
        entry.type === "dir" ||
        !entry.url ||
        !/^PortableGit-\d+\.\d+\.\d+-64-bit\.7z\.exe$/i.test(name)
      ) {
        return false;
      }
      return !latestRelease?.url || entry.url.startsWith(latestRelease.url);
    })
    .map((entry) => entry.url!)
    .sort();

  return portableCandidates[0] ?? null;
}

async function fetchMirrorEntries(url: string): Promise<MirrorDirectoryEntry[]> {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`Mirror request failed with HTTP ${response.status}: ${url}`);
  }
  const json = (await response.json()) as unknown;
  if (!Array.isArray(json)) {
    throw new Error(`Mirror response was not a directory listing: ${url}`);
  }
  return json.filter((entry): entry is MirrorDirectoryEntry => {
    return typeof entry === "object" && entry !== null;
  });
}

async function resolveLatestNode22WindowsMsiUrl(): Promise<string | null> {
  return resolveLatestNode22WindowsMsiUrlFromMirror(
    await fetchMirrorEntries(WINDOWS_NODE_MIRROR_URL),
  );
}

async function resolveLatestNode22WindowsZipUrl(): Promise<string | null> {
  return resolveLatestNode22WindowsZipUrlFromMirror(
    await fetchMirrorEntries(WINDOWS_NODE_MIRROR_URL),
  );
}

async function resolveLatestNode22DarwinTarballUrl(): Promise<string | null> {
  return resolveLatestNode22DarwinTarballUrlFromMirror(
    await fetchMirrorEntries(WINDOWS_NODE_MIRROR_URL),
  );
}

async function resolveLatestGitForWindowsInstallerUrl(): Promise<string | null> {
  const releaseDirs = await fetchMirrorEntries(WINDOWS_GIT_MIRROR_URL);
  const sortedDirs = releaseDirs
    .map((entry) => {
      const version = parseGitForWindowsReleaseVersion(entry.name ?? "");
      if (!version || entry.type !== "dir" || !entry.url) {
        return null;
      }
      return { entry, version };
    })
    .filter((entry): entry is { entry: MirrorDirectoryEntry; version: string } => entry !== null)
    .sort((left, right) => compareVersionStrings(right.version, left.version));

  for (const candidate of sortedDirs) {
    try {
      const releaseEntries = await fetchMirrorEntries(candidate.entry.url!);
      const installerUrl = resolveLatestGitForWindowsInstallerUrlFromMirror(
        [candidate.entry],
        releaseEntries,
      );
      if (installerUrl) {
        return installerUrl;
      }
    } catch (error) {
      log.warn("[model-cli-manager] failed to read Git mirror release", {
        url: candidate.entry.url,
        error: getErrorMessage(error),
      });
    }
  }

  return null;
}

async function resolveLatestGitForWindowsPortableUrl(): Promise<string | null> {
  const releaseDirs = await fetchMirrorEntries(WINDOWS_GIT_MIRROR_URL);
  const sortedDirs = releaseDirs
    .map((entry) => {
      const version = parseGitForWindowsReleaseVersion(entry.name ?? "");
      if (!version || entry.type !== "dir" || !entry.url) {
        return null;
      }
      return { entry, version };
    })
    .filter((entry): entry is { entry: MirrorDirectoryEntry; version: string } => entry !== null)
    .sort((left, right) => compareVersionStrings(right.version, left.version));

  for (const candidate of sortedDirs) {
    try {
      const releaseEntries = await fetchMirrorEntries(candidate.entry.url!);
      const portableUrl = resolveLatestGitForWindowsPortableUrlFromMirror(
        [candidate.entry],
        releaseEntries,
      );
      if (portableUrl) {
        return portableUrl;
      }
    } catch (error) {
      log.warn("[model-cli-manager] failed to read PortableGit mirror release", {
        url: candidate.entry.url,
        error: getErrorMessage(error),
      });
    }
  }

  return null;
}

async function readNodeStatus(
  manager: RuntimeManagerId,
  options?: ShellOptions,
): Promise<NodeRuntimeStatus> {
  if (manager === "managed") {
    const env = buildManagedNodeEnv();
    const [nodeProbe, npmProbe] = await Promise.all([
      tryRunShell("node -v", { ...options, env }),
      tryRunShell("npm -v", { ...options, env }),
    ]);
    const nodeVersion = parseSemanticVersion(nodeProbe?.stdout ?? nodeProbe?.stderr ?? null);
    const npmVersion = parseSemanticVersion(npmProbe?.stdout ?? npmProbe?.stderr ?? null);
    const major = parseMajorVersion(nodeVersion);

    return {
      installed: Boolean(nodeVersion),
      version: nodeVersion,
      major,
      npmVersion,
      satisfies: major !== null && major >= REQUIRED_NODE_MAJOR,
      manager,
      error:
        nodeVersion && npmVersion
          ? null
          : (trimToNull(nodeProbe?.stderr) ??
            trimToNull(npmProbe?.stderr) ??
            `Managed Node.js was not found at ${resolveManagedNodeDir()}.`),
    };
  }

  if (process.platform === "win32" && manager === "shell") {
    const env = buildWindowsCliSearchEnv();
    const [nodeProbe, npmProbe] = await Promise.all([
      tryRunShell("node -v", { ...options, forceWindowsCmd: true, env }),
      tryRunShell("npm -v", { ...options, forceWindowsCmd: true, env }),
    ]);
    const nodeVersion = parseSemanticVersion(nodeProbe?.stdout ?? nodeProbe?.stderr ?? null);
    const npmVersion = parseSemanticVersion(npmProbe?.stdout ?? npmProbe?.stderr ?? null);
    const major = parseMajorVersion(nodeVersion);

    return {
      installed: Boolean(nodeVersion),
      version: nodeVersion,
      major,
      npmVersion,
      satisfies: major !== null && major >= REQUIRED_NODE_MAJOR,
      manager,
      error:
        nodeVersion && npmVersion
          ? null
          : (trimToNull(nodeProbe?.stderr) ??
            trimToNull(npmProbe?.stderr) ??
            "Node.js and npm were not found in the Windows PATH. Install Node.js 22+ or add Node's install directory and %APPDATA%\\npm to PATH."),
    };
  }

  try {
    const result = await runShell(
      wrapWithRuntimeManager(
        'printf "NODE=%s\\n" "$(node -v 2>/dev/null)"; printf "NPM=%s\\n" "$(npm -v 2>/dev/null)"',
        manager,
      ),
      options,
    );
    const lines = result.stdout.split(/\r?\n/);
    const nodeVersion = parseSemanticVersion(
      lines.find((line) => line.startsWith("NODE="))?.slice(5) ?? null,
    );
    const npmVersion = parseSemanticVersion(
      lines.find((line) => line.startsWith("NPM="))?.slice(4) ?? null,
    );
    const major = parseMajorVersion(nodeVersion);

    return {
      installed: Boolean(nodeVersion),
      version: nodeVersion,
      major,
      npmVersion,
      satisfies: major !== null && major >= REQUIRED_NODE_MAJOR,
      manager,
      error: nodeVersion ? null : (trimToNull(result.stderr) ?? "Node.js was not found."),
    };
  } catch (error) {
    return {
      installed: false,
      version: null,
      major: null,
      npmVersion: null,
      satisfies: false,
      manager,
      error: getErrorMessage(error),
    };
  }
}

async function readGitStatus(): Promise<GitRuntimeStatus> {
  if (process.platform !== "win32") {
    return {
      installed: true,
      version: null,
      bashPath: null,
      error: null,
    };
  }

  const env = buildWindowsCliSearchEnv();
  const bashPath = await resolveWindowsGitBashPath();
  const gitPath = await resolveWindowsGitExecutablePath();
  const [gitProbe, bashProbe] = await Promise.all([
    gitPath
      ? tryRunWindowsExecutable(gitPath, ["--version"], env)
      : tryRunShell("git --version", { forceWindowsCmd: true, env }),
    bashPath
      ? tryRunShell("echo ok && git --version", {
          gitBashPath: bashPath,
          env,
        })
      : Promise.resolve(null),
  ]);
  const version = parseSemanticVersion(gitProbe?.stdout ?? gitProbe?.stderr ?? null);
  const bashVersion = parseSemanticVersion(bashProbe?.stdout ?? bashProbe?.stderr ?? null);
  const installed = Boolean(version && bashPath && bashVersion);

  return {
    installed,
    version,
    bashPath,
    error: installed
      ? null
      : [
          version
            ? null
            : "Git executable was not found in app-managed PortableGit or Git for Windows paths.",
          bashPath
            ? null
            : "Git Bash was not found in app-managed PortableGit or Git for Windows paths.",
          bashPath && !bashVersion
            ? `Git Bash was found at ${bashPath} but could not execute git --version.`
            : null,
        ]
          .filter((entry): entry is string => entry !== null)
          .join(" "),
  };
}

async function readCliStatus(
  command: "codex" | "claude",
  packageName: string,
  manager: RuntimeManagerId,
  options?: ShellOptions,
): Promise<ModelCliStatus> {
  try {
    const commandOptions = resolveCliStatusShellOptions(manager, options);
    const versionCommand =
      process.platform === "win32" && manager === "shell"
        ? buildWindowsCliVersionCommand(command)
        : `${command} --version`;
    const result = await runShell(wrapWithRuntimeManager(versionCommand, manager), commandOptions);
    const version = parseSemanticVersion(result.stdout) ?? parseSemanticVersion(result.stderr);
    return {
      command,
      packageName,
      installed: Boolean(version),
      version,
      error: version
        ? null
        : (trimToNull(result.stderr) ??
          `${command} did not report a version. Ensure %APPDATA%\\npm is available in PATH.`),
    };
  } catch (error) {
    return {
      command,
      packageName,
      installed: false,
      version: null,
      error:
        process.platform === "win32" && manager === "shell"
          ? `${getErrorMessage(error)} Ensure %APPDATA%\\npm is available in PATH.`
          : getErrorMessage(error),
    };
  }
}

export function resolveCliStatusShellOptions(
  manager: RuntimeManagerId,
  options?: ShellOptions,
  platform: NodeJS.Platform = process.platform,
): ShellOptions | undefined {
  if (platform !== "win32" || manager !== "shell") {
    return options;
  }
  return { ...options, forceWindowsCmd: true, env: buildWindowsCliSearchEnv() };
}

export function buildWindowsCliVersionCommand(command: "codex" | "claude"): string {
  return buildWindowsCliExecutableCandidates(command)
    .map((candidate) => `${candidate} --version`)
    .join(" || ");
}

export async function getModelCliRuntimeStatus(): Promise<ModelCliRuntimeStatus> {
  const manager = await resolveRuntimeManager();
  const gitBashPath = await resolveWindowsGitBashPath();
  const [git, node, codex, claude] = await Promise.all([
    readGitStatus(),
    readNodeStatus(manager, { gitBashPath }),
    readCliStatus("codex", CODEX_PACKAGE_NAME, manager, { gitBashPath }),
    readCliStatus("claude", CLAUDE_CODE_PACKAGE_NAME, manager, { gitBashPath }),
  ]);

  return { git, node, codex, claude };
}

async function installNode22IntoManager(
  manager: RuntimeManagerId,
  options?: ShellOptions,
): Promise<string> {
  if (manager === "nvm") {
    const result = await runShell(wrapWithNode22Runtime("node -v && npm -v", manager), options);
    return [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
  }
  if (manager === "brew") {
    const result = await runShell(wrapWithNode22Runtime("node -v && npm -v", manager), options);
    return [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
  }
  if (manager === "managed") {
    const result = await runShell(wrapWithNode22Runtime("node -v && npm -v", manager), {
      ...options,
      env: buildManagedNodeEnv(),
    });
    return [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
  }
  if (manager === "shell" && process.platform === "win32") {
    const outputs: string[] = [];
    const errors: string[] = [];

    try {
      const zipUrl = await resolveLatestNode22WindowsZipUrl();
      if (!zipUrl) {
        throw new Error("No Node.js 22 win-x64 zip was found on npmmirror.");
      }
      outputs.push(await installWindowsNodeZipFromUrl(zipUrl));
    } catch (error) {
      errors.push(`Node zip: ${getErrorMessage(error)}`);
      log.warn("[model-cli-manager] mirrored Node.js zip install failed", {
        error: getErrorMessage(error),
      });
    }

    let status = await readNodeStatus(manager, { ...options, forceWindowsCmd: true });

    if (!status.satisfies) {
      try {
        const installerUrl = await resolveLatestNode22WindowsMsiUrl();
        if (!installerUrl) {
          throw new Error("No Node.js 22 x64 MSI was found on npmmirror.");
        }
        const mirrorResult = await runShell(buildWindowsNodeDirectInstallCommand(installerUrl), {
          ...options,
          forceWindowsCmd: true,
          env: buildWindowsCliSearchEnv(),
        });
        outputs.push([mirrorResult.stdout, mirrorResult.stderr].filter(Boolean).join("\n").trim());
      } catch (error) {
        errors.push(`Node MSI: ${getErrorMessage(error)}`);
        log.warn("[model-cli-manager] mirrored Node.js MSI install failed", {
          error: getErrorMessage(error),
        });
      }
      status = await readNodeStatus(manager, { ...options, forceWindowsCmd: true });
    }

    if (!status.satisfies) {
      if (await commandExists("winget")) {
        try {
          const installResult = await runShell(
            "winget install --id OpenJS.NodeJS.LTS -e --accept-package-agreements --accept-source-agreements",
            { ...options, forceWindowsCmd: true },
          );
          outputs.push(
            [installResult.stdout, installResult.stderr].filter(Boolean).join("\n").trim(),
          );
          status = await readNodeStatus(manager, { ...options, forceWindowsCmd: true });
        } catch (error) {
          errors.push(`WinGet: ${getErrorMessage(error)}`);
        }
      } else {
        errors.push("WinGet: not available");
      }
    }

    if (!status.satisfies) {
      throw new Error(
        `Automatic Node.js 22 installation failed. ${errors.join(" ")} Detected runtime: ${status.version ?? "unknown"}.`,
      );
    }

    const verifyResult = await runShell("node -v && npm -v", {
      ...options,
      forceWindowsCmd: true,
      env: buildWindowsCliSearchEnv(),
    });
    outputs.push([verifyResult.stdout, verifyResult.stderr].filter(Boolean).join("\n").trim());
    return outputs.filter(Boolean).join("\n").trim();
  }

  if (manager === "shell" && process.platform === "darwin") {
    const outputs: string[] = [];
    let mirrorError: string | null = null;

    try {
      const tarballUrl = await resolveLatestNode22DarwinTarballUrl();
      if (!tarballUrl) {
        throw new Error(`No Node.js 22 macOS ${arch} tarball was found on npmmirror.`);
      }
      const installDir = resolveManagedNodeDir();
      const mirrorResult = await runShell(
        buildMacOSNodeDirectInstallCommand(tarballUrl, installDir),
        options,
      );
      outputs.push([mirrorResult.stdout, mirrorResult.stderr].filter(Boolean).join("\n").trim());
    } catch (error) {
      mirrorError = getErrorMessage(error);
      log.warn("[model-cli-manager] mirrored macOS Node.js install failed", {
        error: mirrorError,
      });
    }

    const status = await readNodeStatus("managed", options);
    if (!status.satisfies) {
      throw new Error(
        `Automatic Node.js 22 installation failed via npmmirror${mirrorError ? `: ${mirrorError}` : ""}.`,
      );
    }

    const verifyResult = await runShell("node -v && npm -v", {
      ...options,
      env: buildManagedNodeEnv(),
    });
    outputs.push([verifyResult.stdout, verifyResult.stderr].filter(Boolean).join("\n").trim());
    return outputs.filter(Boolean).join("\n").trim();
  }

  throw new Error(
    "Automatic Node.js 22 installation currently requires nvm or Homebrew in this environment.",
  );
}

async function installWindowsGitBash(): Promise<string> {
  if (process.platform !== "win32") {
    return "";
  }

  const before = await readGitStatus();
  if (before.installed) {
    return "";
  }

  const outputs: string[] = [];
  const errors: string[] = [];

  try {
    const portableUrl = await resolveLatestGitForWindowsPortableUrl();
    if (!portableUrl) {
      throw new Error("No PortableGit 64-bit full distribution was found on npmmirror.");
    }
    const installDir = resolveWindowsPortableGitDir(process.env);
    outputs.push(await installPortableGitFromUrl(portableUrl, installDir));
  } catch (error) {
    errors.push(`PortableGit npmmirror: ${getErrorMessage(error)}`);
    log.warn("[model-cli-manager] mirrored PortableGit install failed", {
      error: getErrorMessage(error),
      snapshot: createWindowsGitPathSnapshot(resolveWindowsPortableGitDir(process.env)),
    });
  }

  let status = await readGitStatus();
  if (status.installed) {
    await patchClaudeCodeGitBashPathForWindows(status.bashPath);
    log.info("[model-cli-manager] app-managed PortableGit is ready", {
      gitVersion: status.version,
      gitBashPath: status.bashPath,
    });
    return outputs.filter(Boolean).join("\n").trim();
  }
  errors.push(`PortableGit npmmirror validation: ${status.error}`);

  try {
    const installerUrl = await resolveLatestGitForWindowsInstallerUrl();
    if (!installerUrl) {
      throw new Error("No Git for Windows 64-bit installer was found on npmmirror.");
    }
    const mirrorResult = await runShell(buildWindowsGitBashMirrorInstallCommand(installerUrl), {
      forceWindowsCmd: true,
      env: buildWindowsCliSearchEnv(),
    });
    outputs.push([mirrorResult.stdout, mirrorResult.stderr].filter(Boolean).join("\n").trim());
  } catch (error) {
    errors.push(`npmmirror: ${getErrorMessage(error)}`);
    log.warn("[model-cli-manager] mirrored Git install failed", {
      error: getErrorMessage(error),
    });
  }

  status = await readGitStatus();
  if (!status.installed) {
    errors.push(`Git for Windows mirror validation: ${status.error}`);
  }
  if (!status.installed && (await commandExists("winget"))) {
    try {
      const wingetResult = await runShell(buildWindowsGitBashInstallCommand(), {
        forceWindowsCmd: true,
        env: buildWindowsCliSearchEnv(),
      });
      outputs.push([wingetResult.stdout, wingetResult.stderr].filter(Boolean).join("\n").trim());
    } catch (error) {
      errors.push(`WinGet: ${getErrorMessage(error)}`);
    }
    status = await readGitStatus();
    if (!status.installed) {
      errors.push(`WinGet validation: ${status.error}`);
    }
  }

  if (!status.installed) {
    try {
      const directResult = await runShell(buildWindowsGitBashDirectInstallCommand(), {
        forceWindowsCmd: true,
        env: buildWindowsCliSearchEnv(),
      });
      outputs.push([directResult.stdout, directResult.stderr].filter(Boolean).join("\n").trim());
    } catch (error) {
      errors.push(`GitHub: ${getErrorMessage(error)}`);
    }
    status = await readGitStatus();
    if (!status.installed) {
      errors.push(`GitHub direct installer validation: ${status.error}`);
    }
  }

  if (!status.installed) {
    throw new Error(buildWindowsGitInstallFailureMessage(errors.filter(Boolean), status));
  }

  await patchClaudeCodeGitBashPathForWindows(status.bashPath);
  await ensureWindowsUserPathEntries(getWindowsExternalCliPathEntries(process.env));

  return outputs.filter(Boolean).join("\n").trim();
}

export async function installNode22Runtime(): Promise<ModelCliInstallResult> {
  const manager = await resolveRuntimeManager();
  const gitBashPath = await resolveWindowsGitBashPath();
  const status = await readNodeStatus(manager, { gitBashPath });
  let output = "";

  if (!status.satisfies) {
    output = await installNode22IntoManager(manager, { gitBashPath });
  }

  return {
    status: await getModelCliRuntimeStatus(),
    output,
  };
}

export function resolvePackageInstallShellOptions(
  manager: RuntimeManagerId,
  options?: ShellOptions,
  platform: NodeJS.Platform = process.platform,
): ShellOptions | undefined {
  if (platform !== "win32" || manager !== "shell") {
    return options;
  }
  return { ...options, forceWindowsCmd: true, env: buildWindowsCliSearchEnv() };
}

async function installPackageIntoRuntime(
  packageName: string,
  manager: RuntimeManagerId,
  options?: ShellOptions,
): Promise<string> {
  const runtimeOptions = resolvePackageInstallShellOptions(manager, options);
  if (process.platform === "win32" && manager === "shell") {
    const outputs: string[] = [];
    const errors: string[] = [];
    for (const registry of ["npmmirror", "official"] as const) {
      try {
        const result = await runShell(
          buildWindowsNpmPackageInstallCommand(packageName, registry),
          runtimeOptions,
        );
        outputs.push([result.stdout, result.stderr].filter(Boolean).join("\n").trim());
        const npmGlobalBin = await resolveWindowsNpmGlobalBinPath(runtimeOptions);
        await ensureWindowsUserPathEntries([
          ...getWindowsExternalCliPathEntries(process.env),
          ...(npmGlobalBin ? [npmGlobalBin] : []),
        ]);
        return outputs.filter(Boolean).join("\n").trim();
      } catch (error) {
        const label = registry === "npmmirror" ? "npmmirror npm registry" : "npm official registry";
        errors.push(`${label}: ${getErrorMessage(error)}`);
        log.warn("[model-cli-manager] npm package install failed", {
          packageName,
          registry,
          error: getErrorMessage(error),
        });
      }
    }
    throw new Error(`Failed to install ${packageName}. ${errors.join(" ")}`);
  }

  const result = await runShell(
    wrapWithNode22Runtime(`npm install -g ${packageName}@latest`, manager),
    runtimeOptions,
  );
  return [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
}

export async function installCodexCli(): Promise<ModelCliInstallResult> {
  const manager = await resolveRuntimeManager();
  const gitBashPath = await resolveWindowsGitBashPath();
  const nodeStatus = await readNodeStatus(manager, { gitBashPath });
  const outputs: string[] = [];

  if (!nodeStatus.satisfies) {
    outputs.push(await installNode22IntoManager(manager, { gitBashPath }));
  }
  outputs.push(await installPackageIntoRuntime(CODEX_PACKAGE_NAME, manager, { gitBashPath }));

  return {
    status: await getModelCliRuntimeStatus(),
    output: outputs.filter(Boolean).join("\n").trim(),
  };
}

export async function installClaudeCodeCli(): Promise<ModelCliInstallResult> {
  const manager = await resolveRuntimeManager();
  const outputs: string[] = [];
  const gitBashPath = await resolveWindowsGitBashPath();
  const nodeStatus = await readNodeStatus(manager, { gitBashPath });

  if (!nodeStatus.satisfies) {
    outputs.push(await installNode22IntoManager(manager, { gitBashPath }));
  }
  outputs.push(await installPackageIntoRuntime(CLAUDE_CODE_PACKAGE_NAME, manager, { gitBashPath }));

  return {
    status: await getModelCliRuntimeStatus(),
    output: outputs.filter(Boolean).join("\n").trim(),
  };
}

export async function installAllModelClis(): Promise<ModelCliInstallResult> {
  const manager = await resolveRuntimeManager();
  const outputs: string[] = [];

  try {
    if (process.platform === "win32") {
      outputs.push(await installWindowsGitBash());
    }

    const gitBashPath = await resolveWindowsGitBashPath();
    await patchClaudeCodeGitBashPathForWindows(gitBashPath);
    const nodeStatus = await readNodeStatus(manager, { gitBashPath });

    if (!nodeStatus.satisfies) {
      outputs.push(await installNode22IntoManager(manager, { gitBashPath }));
    }
    outputs.push(await installPackageIntoRuntime(CODEX_PACKAGE_NAME, manager, { gitBashPath }));
    outputs.push(
      await installPackageIntoRuntime(CLAUDE_CODE_PACKAGE_NAME, manager, { gitBashPath }),
    );

    const status = await getModelCliRuntimeStatus();
    log.info("[model-cli-manager] installed runtime stack", {
      gitVersion: status.git.version,
      gitBashPath: status.git.bashPath,
      nodeVersion: status.node.version,
      codexVersion: status.codex.version,
      claudeVersion: status.claude.version,
    });

    return {
      status,
      output: outputs.filter(Boolean).join("\n").trim(),
    };
  } catch (error) {
    throw await buildInstallFailureError(error);
  }
}

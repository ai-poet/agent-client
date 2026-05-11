import { existsSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";
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

type RuntimeManagerId = "nvm" | "brew" | "shell";

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
  if (normalized.length <= 180) {
    return normalized;
  }
  return `${normalized.slice(0, 177).trimEnd()}...`;
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
    path.join(resolveWindowsPaseoHome(process.env), "toolchains", "PortableGit", "bin", "bash.exe"),
    path.join(
      resolveWindowsPaseoHome(process.env),
      "toolchains",
      "PortableGit",
      "usr",
      "bin",
      "bash.exe",
    ),
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

export function buildWindowsCliSearchPath(env: NodeJS.ProcessEnv = process.env): string {
  const paths: string[] = [];
  appendUniquePath(paths, env.APPDATA ? path.win32.join(env.APPDATA, "npm") : null);
  appendUniquePath(paths, env.ProgramFiles ? path.win32.join(env.ProgramFiles, "nodejs") : null);
  appendUniquePath(paths, path.win32.join(resolveWindowsPaseoHome(env), "toolchains", "PortableGit", "cmd"));
  appendUniquePath(paths, path.win32.join(resolveWindowsPaseoHome(env), "toolchains", "PortableGit", "bin"));
  appendUniquePath(
    paths,
    path.win32.join(resolveWindowsPaseoHome(env), "toolchains", "PortableGit", "usr", "bin"),
  );
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

function buildWindowsCliSearchEnv(env: NodeJS.ProcessEnv = process.env): NodeJS.ProcessEnv {
  const searchPath = buildWindowsCliSearchPath(env);
  return {
    ...env,
    PATH: searchPath,
    Path: searchPath,
  };
}

export function wrapWithRuntimeManager(command: string, manager: RuntimeManagerId): string {
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

export function buildWindowsNodeDirectInstallCommand(installerUrl: string): string {
  const quotedUrl = quotePowerShellString(installerUrl);
  return `powershell -NoProfile -ExecutionPolicy Bypass -Command "$ErrorActionPreference='Stop'; $installerUrl=${quotedUrl}; $installerPath=Join-Path $env:TEMP ('paseo-node-installer-' + [Guid]::NewGuid().ToString('N') + '.msi'); try { Invoke-WebRequest -Uri $installerUrl -OutFile $installerPath -UseBasicParsing -TimeoutSec 60; $process=Start-Process -FilePath 'msiexec.exe' -ArgumentList '/i',$installerPath,'/qn','/norestart' -Wait -PassThru; if ($process.ExitCode -ne 0) { throw ('Node.js installer failed with exit code ' + $process.ExitCode) } } catch { throw ('Node.js mirror installer failed: ' + $_.Exception.Message) } finally { Remove-Item -Path $installerPath -Force -ErrorAction SilentlyContinue }"`;
}

export function buildWindowsGitBashMirrorInstallCommand(installerUrl: string): string {
  const quotedUrl = quotePowerShellString(installerUrl);
  return `powershell -NoProfile -ExecutionPolicy Bypass -Command "$ErrorActionPreference='Stop'; $installerUrl=${quotedUrl}; $installerPath=Join-Path $env:TEMP ('paseo-git-installer-' + [Guid]::NewGuid().ToString('N') + '.exe'); try { Invoke-WebRequest -Uri $installerUrl -OutFile $installerPath -UseBasicParsing -TimeoutSec 60; $process=Start-Process -FilePath $installerPath -ArgumentList '/VERYSILENT','/NORESTART','/SP-','/NOCANCEL' -Wait -PassThru; if ($process.ExitCode -ne 0) { throw ('Git for Windows installer failed with exit code ' + $process.ExitCode) } } catch { throw ('Git mirror installer failed: ' + $_.Exception.Message) } finally { Remove-Item -Path $installerPath -Force -ErrorAction SilentlyContinue }"`;
}

export function buildWindowsGitBashPortableInstallCommand(
  installerUrl: string,
  installDir: string,
): string {
  const quotedUrl = quotePowerShellString(installerUrl);
  const quotedInstallDir = quotePowerShellString(installDir);
  return `powershell -NoProfile -ExecutionPolicy Bypass -Command "$ErrorActionPreference='Stop'; $installerUrl=${quotedUrl}; $installDir=${quotedInstallDir}; $installerPath=Join-Path $env:TEMP ('paseo-portable-git-' + [Guid]::NewGuid().ToString('N') + '.7z.exe'); try { if (Test-Path $installDir) { Remove-Item -Path $installDir -Recurse -Force }; New-Item -ItemType Directory -Path $installDir -Force | Out-Null; Invoke-WebRequest -Uri $installerUrl -OutFile $installerPath -UseBasicParsing -TimeoutSec 60; $process=Start-Process -FilePath $installerPath -ArgumentList '-y',('-o' + $installDir) -Wait -PassThru; if ($process.ExitCode -ne 0) { throw ('PortableGit extractor failed with exit code ' + $process.ExitCode) }; $gitBash=Join-Path $installDir 'git-bash.exe'; $bash=Join-Path $installDir 'bin\\\\bash.exe'; if ((-not (Test-Path $gitBash)) -and (-not (Test-Path $bash))) { throw 'PortableGit extracted but Git Bash was not found' } } catch { throw ('PortableGit mirror install failed: ' + $_.Exception.Message) } finally { Remove-Item -Path $installerPath -Force -ErrorAction SilentlyContinue }"`;
}

export function buildWindowsGitBashDirectInstallCommand(): string {
  return buildWindowsGitBashMirrorInstallCommand(WINDOWS_GIT_DIRECT_DOWNLOAD_URL);
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
  const [gitProbe, bashProbe] = await Promise.all([
    tryRunShell("git --version", { forceWindowsCmd: true, env }),
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
          version ? null : "Git was not found in the Windows PATH.",
          bashPath ? null : "Git Bash was not found.",
          bashPath && !bashVersion
            ? "Git Bash was found but could not execute git --version."
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
  if (manager === "shell" && process.platform === "win32") {
    const outputs: string[] = [];
    let mirrorError: string | null = null;

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
      mirrorError = getErrorMessage(error);
      log.warn("[model-cli-manager] mirrored Node.js install failed", { error: mirrorError });
    }

    let status = await readNodeStatus(manager, { ...options, forceWindowsCmd: true });
    if (!status.satisfies) {
      if (!(await commandExists("winget"))) {
        throw new Error(
          `Automatic Node.js 22 installation failed via npmmirror${mirrorError ? `: ${mirrorError}` : ""}. WinGet is not available for fallback.`,
        );
      }

      const installResult = await runShell(
        "winget install --id OpenJS.NodeJS.LTS -e --accept-package-agreements --accept-source-agreements",
        { ...options, forceWindowsCmd: true },
      );
      outputs.push([installResult.stdout, installResult.stderr].filter(Boolean).join("\n").trim());
      status = await readNodeStatus(manager, { ...options, forceWindowsCmd: true });
    }

    if (!status.satisfies) {
      throw new Error(
        `Node.js installation finished but the detected runtime is ${status.version ?? "unknown"}. Please ensure Node.js ${REQUIRED_NODE_MAJOR}+ is available in PATH.`,
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
    const installDir = path.join(resolveWindowsPaseoHome(process.env), "toolchains", "PortableGit");
    const portableResult = await runShell(
      buildWindowsGitBashPortableInstallCommand(portableUrl, installDir),
      {
        forceWindowsCmd: true,
        env: buildWindowsCliSearchEnv(),
      },
    );
    outputs.push([portableResult.stdout, portableResult.stderr].filter(Boolean).join("\n").trim());
  } catch (error) {
    errors.push(`PortableGit npmmirror: ${getErrorMessage(error)}`);
    log.warn("[model-cli-manager] mirrored PortableGit install failed", {
      error: getErrorMessage(error),
    });
  }

  let status = await readGitStatus();
  if (status.installed) {
    await patchClaudeCodeGitBashPathForWindows(status.bashPath);
    return outputs.filter(Boolean).join("\n").trim();
  }

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
  }

  if (!status.installed) {
    throw new Error(
      `Git for Windows installation did not complete. ${errors.filter(Boolean).join(" ") || status.error || "Git Bash was not detected after installation."}`,
    );
  }

  await patchClaudeCodeGitBashPathForWindows(status.bashPath);

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

import { describe, expect, it } from "vitest";
import {
  buildWindowsGitBashChocolateyInstallCommand,
  buildWindowsGitBashDirectInstallCommand,
  buildWindowsGitBashInstallCommand,
  buildWindowsGitBashScoopInstallCommand,
  REQUIRED_NODE_MAJOR,
  buildWindowsCliExecutableCandidates,
  buildWindowsCliSearchPath,
  isWindowsGitBashPath,
  parseMajorVersion,
  parseSemanticVersion,
  resolvePackageInstallShellOptions,
  wrapWithNode22Runtime,
  wrapWithRuntimeManager,
} from "./model-cli-manager";

describe("model-cli-manager", () => {
  it("extracts semantic versions from common CLI outputs", () => {
    expect(parseSemanticVersion("codex-cli 0.118.0")).toBe("0.118.0");
    expect(parseSemanticVersion("2.1.89 (Claude Code)")).toBe("2.1.89");
  });

  it("extracts the node major version", () => {
    expect(parseMajorVersion("22.15.1")).toBe(22);
    expect(parseMajorVersion("v20.20.1")).toBe(20);
    expect(parseMajorVersion(null)).toBeNull();
  });

  it("wraps Node 22 install commands for nvm", () => {
    const command = wrapWithNode22Runtime("npm install -g @openai/codex@latest", "nvm");

    expect(command).toContain(`nvm install ${REQUIRED_NODE_MAJOR}`);
    expect(command).toContain(`nvm alias default ${REQUIRED_NODE_MAJOR}`);
    expect(command).toContain(`nvm use ${REQUIRED_NODE_MAJOR}`);
    expect(command).toContain('export PATH="$NVM_BIN:$PATH"');
  });

  it("leaves shell runtime commands unchanged", () => {
    const command = "codex --version";

    expect(wrapWithRuntimeManager(command, "shell")).toBe(command);
    expect(wrapWithNode22Runtime(command, "shell")).toBe(command);
  });

  it("uses cmd shell for Codex install on Windows shell manager", () => {
    const options = resolvePackageInstallShellOptions(
      "shell",
      { gitBashPath: "C:/Program Files/Git/bin/bash.exe" },
      "win32",
    );
    expect(options?.forceWindowsCmd).toBe(true);
  });

  it("uses cmd shell for Claude Code install on Windows shell manager", () => {
    const gitBashPath = "C:/Program Files/Git/bin/bash.exe";
    const options = resolvePackageInstallShellOptions("shell", { gitBashPath }, "win32");
    expect(options?.gitBashPath).toBe(gitBashPath);
    expect(options?.forceWindowsCmd).toBe(true);
  });

  it("rejects WSL bash launchers when detecting Git Bash on Windows", () => {
    expect(isWindowsGitBashPath("C:/Windows/System32/bash.exe")).toBe(false);
    expect(isWindowsGitBashPath("C:\\Windows\\SysWOW64\\bash.exe")).toBe(false);
    expect(isWindowsGitBashPath("C:/Windows/System32/wsl.exe")).toBe(false);
  });

  it("accepts Git for Windows and Scoop Git Bash paths", () => {
    expect(isWindowsGitBashPath("C:/Program Files/Git/bin/bash.exe")).toBe(true);
    expect(isWindowsGitBashPath("C:/Program Files/Git/usr/bin/bash.exe")).toBe(true);
    expect(isWindowsGitBashPath("C:/Users/alice/scoop/apps/git/current/bin/bash.exe")).toBe(true);
  });

  it("builds Windows CLI executable candidates with cmd and exe suffixes", () => {
    expect(buildWindowsCliExecutableCandidates("claude")).toEqual([
      "claude.cmd",
      "claude.exe",
      "claude",
    ]);
    expect(buildWindowsCliExecutableCandidates("codex")).toEqual([
      "codex.cmd",
      "codex.exe",
      "codex",
    ]);
  });

  it("extends Windows CLI search path with npm and node install directories", () => {
    const searchPath = buildWindowsCliSearchPath({
      PATH: "C:\\Windows\\System32",
      APPDATA: "C:\\Users\\alice\\AppData\\Roaming",
      ProgramFiles: "C:\\Program Files",
    });

    expect(searchPath.split(";")).toEqual([
      "C:\\Users\\alice\\AppData\\Roaming\\npm",
      "C:\\Program Files\\nodejs",
      "C:\\Windows\\System32",
    ]);
  });

  it("builds the expected WinGet command for Git Bash auto-install", () => {
    const command = buildWindowsGitBashInstallCommand();
    expect(command).toContain("winget install");
    expect(command).toContain("--id Git.Git");
    expect(command).toContain("--accept-package-agreements");
    expect(command).toContain("--accept-source-agreements");
  });

  it("builds Chocolatey and Scoop Git Bash install commands", () => {
    expect(buildWindowsGitBashChocolateyInstallCommand()).toContain("choco install git");
    expect(buildWindowsGitBashScoopInstallCommand()).toBe("scoop install git");
  });

  it("builds a direct PowerShell Git Bash installer command", () => {
    const command = buildWindowsGitBashDirectInstallCommand();
    expect(command).toContain("powershell -NoProfile");
    expect(command).toContain("Git-64-bit.exe");
    expect(command).toContain("/VERYSILENT");
    expect(command).toContain("Invoke-WebRequest");
    expect(command).toContain("Start-Process");
  });
});

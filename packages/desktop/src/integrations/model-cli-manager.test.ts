import { describe, expect, it } from "vitest";
import {
  buildWindowsGitBashChocolateyInstallCommand,
  buildWindowsGitBashDirectInstallCommand,
  buildWindowsGitBashInstallCommand,
  buildWindowsGitBashMirrorInstallCommand,
  buildWindowsGitBashPortableExtractArgs,
  buildWindowsGitInstallFailureMessage,
  buildMacOSNodeDirectInstallCommand,
  buildWindowsNodeDirectInstallCommand,
  buildWindowsNodeZipExtractPowerShellArgs,
  buildWindowsGitBashScoopInstallCommand,
  buildWindowsGetUserPathPowerShellArgs,
  buildWindowsNpmPackageInstallCommand,
  buildWindowsNotifyEnvironmentChangePowerShellCommand,
  buildWindowsNotifyEnvironmentChangePowerShellArgs,
  buildWindowsSetUserPathPowerShellArgs,
  buildWindowsUserPathValue,
  REQUIRED_NODE_MAJOR,
  buildWindowsCliExecutableCandidates,
  buildWindowsCliSearchPath,
  buildWindowsCliVersionCommand,
  isWindowsGitBashPath,
  parseMajorVersion,
  parseSemanticVersion,
  resolveLatestGitForWindowsInstallerUrlFromMirror,
  resolveLatestGitForWindowsPortableUrlFromMirror,
  resolveLatestNode22DarwinTarballUrlFromMirror,
  resolveLatestNode22WindowsZipUrlFromMirror,
  resolveLatestNode22WindowsMsiUrlFromMirror,
  resolveCliStatusShellOptions,
  resolvePackageInstallShellOptions,
  resolveWindowsManagedNodeDir,
  resolveWindowsNpmGlobalBinPathFromPrefix,
  resolveWindowsPortableGitTempFallbackDirs,
  shouldUseWindowsGitBash,
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

  it("uses cmd shell for Claude Code status checks on Windows shell manager", () => {
    const options = resolveCliStatusShellOptions(
      "shell",
      { gitBashPath: "C:/Windows/System32/bash.exe" },
      "win32",
    );

    expect(options?.forceWindowsCmd).toBe(true);
    expect(buildWindowsCliVersionCommand("claude")).toBe(
      "claude.cmd --version || claude.exe --version || claude --version",
    );
  });

  it("rejects WSL bash launchers when detecting Git Bash on Windows", () => {
    expect(isWindowsGitBashPath("C:/Windows/System32/bash.exe")).toBe(false);
    expect(isWindowsGitBashPath("C:\\Windows\\SysWOW64\\bash.exe")).toBe(false);
    expect(isWindowsGitBashPath("C:/Windows/System32/wsl.exe")).toBe(false);
    expect(
      isWindowsGitBashPath("C:/Users/alice/.paseo/toolchains/mingit-2.54.0/bin/bash.exe"),
    ).toBe(false);
  });

  it("accepts Git for Windows and Scoop Git Bash paths", () => {
    expect(isWindowsGitBashPath("C:/Program Files/Git/bin/bash.exe")).toBe(true);
    expect(isWindowsGitBashPath("C:/Program Files/Git/usr/bin/bash.exe")).toBe(true);
    expect(isWindowsGitBashPath("C:/Users/alice/.paseo/toolchains/PortableGit/bin/bash.exe")).toBe(
      true,
    );
    expect(isWindowsGitBashPath("C:/Users/alice/scoop/apps/git/current/bin/bash.exe")).toBe(true);
  });

  it("only allows real Git Bash paths to opt into Windows bash execution", () => {
    expect(shouldUseWindowsGitBash({ gitBashPath: "C:/Windows/System32/bash.exe" }, "win32")).toBe(
      false,
    );
    expect(
      shouldUseWindowsGitBash({ gitBashPath: "C:/Program Files/Git/bin/bash.exe" }, "win32"),
    ).toBe(true);
    expect(
      shouldUseWindowsGitBash(
        { gitBashPath: "C:/Program Files/Git/bin/bash.exe", forceWindowsCmd: true },
        "win32",
      ),
    ).toBe(false);
  });

  it("builds Windows CLI version probes with cmd and exe suffixes first", () => {
    expect(buildWindowsCliVersionCommand("claude")).toBe(
      "claude.cmd --version || claude.exe --version || claude --version",
    );
    expect(buildWindowsCliVersionCommand("codex")).toBe(
      "codex.cmd --version || codex.exe --version || codex --version",
    );
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
      "ProgramFiles(x86)": "C:\\Program Files (x86)",
      PASEO_HOME: "C:\\Users\\alice\\.paseo",
      LOCALAPPDATA: "C:\\Users\\alice\\AppData\\Local",
      USERPROFILE: "C:\\Users\\alice",
    });

    expect(searchPath.split(";")).toEqual([
      "C:\\Users\\alice\\AppData\\Roaming\\npm",
      "C:\\Users\\alice\\.paseo\\toolchains\\node22-win-x64",
      "C:\\Program Files\\nodejs",
      "C:\\Users\\alice\\.paseo\\toolchains\\PortableGit\\cmd",
      "C:\\Users\\alice\\.paseo\\toolchains\\PortableGit\\bin",
      "C:\\Users\\alice\\.paseo\\toolchains\\PortableGit\\usr\\bin",
      "C:\\Program Files\\Git\\cmd",
      "C:\\Program Files\\Git\\bin",
      "C:\\Program Files\\Git\\usr\\bin",
      "C:\\Program Files (x86)\\Git\\cmd",
      "C:\\Program Files (x86)\\Git\\bin",
      "C:\\Program Files (x86)\\Git\\usr\\bin",
      "C:\\Users\\alice\\AppData\\Local\\Programs\\Git\\cmd",
      "C:\\Users\\alice\\AppData\\Local\\Programs\\Git\\bin",
      "C:\\Users\\alice\\AppData\\Local\\Programs\\Git\\usr\\bin",
      "C:\\Users\\alice\\scoop\\apps\\git\\current\\cmd",
      "C:\\Users\\alice\\scoop\\apps\\git\\current\\bin",
      "C:\\Users\\alice\\scoop\\apps\\git\\current\\usr\\bin",
      "C:\\Windows\\System32",
    ]);
  });

  it("builds user PATH values without replacing existing entries", () => {
    const value = buildWindowsUserPathValue(
      "C:\\Windows\\System32;C:\\Users\\alice\\AppData\\Roaming\\npm",
      [
        "C:\\Users\\alice\\.paseo\\toolchains\\node22-win-x64",
        "C:\\Users\\alice\\AppData\\Roaming\\npm",
        "C:\\Users\\alice\\.paseo\\toolchains\\PortableGit\\bin",
      ],
    );

    expect(value.split(";")).toEqual([
      "C:\\Windows\\System32",
      "C:\\Users\\alice\\AppData\\Roaming\\npm",
      "C:\\Users\\alice\\.paseo\\toolchains\\node22-win-x64",
      "C:\\Users\\alice\\.paseo\\toolchains\\PortableGit\\bin",
    ]);
  });

  it("builds a Windows environment-change broadcast command", () => {
    const command = buildWindowsNotifyEnvironmentChangePowerShellCommand();

    expect(command).toContain("SendMessageTimeout");
    expect(command).toContain("WM_SETTINGCHANGE");
    expect(command).toContain("Environment");
    expect(command).toContain("0xffff");
  });

  it("builds Windows user PATH PowerShell arguments for direct execFile calls", () => {
    expect(buildWindowsGetUserPathPowerShellArgs()).toEqual([
      "-NoProfile",
      "-ExecutionPolicy",
      "Bypass",
      "-Command",
      "[Environment]::GetEnvironmentVariable('Path', 'User')",
    ]);

    const setArgs = buildWindowsSetUserPathPowerShellArgs(
      "C:\\Tools\\node;C:\\Users\\alice\\AppData\\Roaming\\npm",
    );
    expect(setArgs.slice(0, 4)).toEqual(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command"]);
    expect(setArgs[4]).toContain("[Environment]::SetEnvironmentVariable('Path'");
    expect(setArgs[4]).toContain("'C:\\Tools\\node;C:\\Users\\alice\\AppData\\Roaming\\npm'");
    expect(setArgs[4]).toContain("'User'");
    expect(setArgs.join(" ")).not.toContain("cmd /c");

    const notifyArgs = buildWindowsNotifyEnvironmentChangePowerShellArgs();
    expect(notifyArgs.slice(0, 4)).toEqual([
      "-NoProfile",
      "-ExecutionPolicy",
      "Bypass",
      "-Command",
    ]);
    expect(notifyArgs[4]).toContain("SendMessageTimeout");
    expect(notifyArgs[4]).toContain("Environment");
  });

  it("resolves Windows npm global bin directories from prefix output", () => {
    expect(
      resolveWindowsNpmGlobalBinPathFromPrefix("C:\\Users\\alice\\AppData\\Roaming\\npm\r\n"),
    ).toBe("C:\\Users\\alice\\AppData\\Roaming\\npm");
    expect(
      resolveWindowsNpmGlobalBinPathFromPrefix(
        "C:\\Users\\alice\\AppData\\Roaming\\npm\\node_modules\n",
      ),
    ).toBe("C:\\Users\\alice\\AppData\\Roaming\\npm");
  });

  it("picks the newest Node 22 x64 MSI from npmmirror directory entries", () => {
    const url = resolveLatestNode22WindowsMsiUrlFromMirror([
      {
        type: "file",
        name: "node-v22.10.0-x64.msi",
        url: "https://registry.npmmirror.com/-/binary/node/latest-v22.x/node-v22.10.0-x64.msi",
      },
      {
        type: "file",
        name: "node-v22.11.0-arm64.msi",
        url: "https://registry.npmmirror.com/-/binary/node/latest-v22.x/node-v22.11.0-arm64.msi",
      },
      {
        type: "file",
        name: "node-v22.11.0-x64.msi",
        url: "https://registry.npmmirror.com/-/binary/node/latest-v22.x/node-v22.11.0-x64.msi",
      },
    ]);

    expect(url).toBe(
      "https://registry.npmmirror.com/-/binary/node/latest-v22.x/node-v22.11.0-x64.msi",
    );
  });

  it("picks the newest Node 22 win-x64 zip from npmmirror directory entries", () => {
    const url = resolveLatestNode22WindowsZipUrlFromMirror([
      {
        type: "file",
        name: "node-v22.10.0-win-x64.zip",
        url: "https://registry.npmmirror.com/-/binary/node/latest-v22.x/node-v22.10.0-win-x64.zip",
      },
      {
        type: "file",
        name: "node-v22.12.0-win-arm64.zip",
        url: "https://registry.npmmirror.com/-/binary/node/latest-v22.x/node-v22.12.0-win-arm64.zip",
      },
      {
        type: "file",
        name: "node-v22.11.0-x64.msi",
        url: "https://registry.npmmirror.com/-/binary/node/latest-v22.x/node-v22.11.0-x64.msi",
      },
      {
        type: "file",
        name: "node-v22.12.0-win-x64.zip",
        url: "https://registry.npmmirror.com/-/binary/node/latest-v22.x/node-v22.12.0-win-x64.zip",
      },
    ]);

    expect(url).toBe(
      "https://registry.npmmirror.com/-/binary/node/latest-v22.x/node-v22.12.0-win-x64.zip",
    );
  });

  it("builds Node zip extraction command with literal paths embedded", () => {
    const args = buildWindowsNodeZipExtractPowerShellArgs(
      "C:\\Users\\alice\\AppData\\Local\\Temp\\paseo node\\node22.zip",
      "C:\\Users\\alice\\AppData\\Local\\Temp\\paseo node",
    );

    expect(args).toEqual([
      "-NoProfile",
      "-ExecutionPolicy",
      "Bypass",
      "-Command",
      "$ErrorActionPreference='Stop'; Expand-Archive -LiteralPath 'C:\\Users\\alice\\AppData\\Local\\Temp\\paseo node\\node22.zip' -DestinationPath 'C:\\Users\\alice\\AppData\\Local\\Temp\\paseo node' -Force",
    ]);
    expect(args.join(" ")).not.toContain("$args[0]");
    expect(args.join(" ")).not.toContain("$args[1]");
  });

  it("resolves the app-managed Windows Node directory under Paseo home", () => {
    expect(resolveWindowsManagedNodeDir({ PASEO_HOME: "C:\\Users\\alice\\.paseo" })).toBe(
      "C:\\Users\\alice\\.paseo\\toolchains\\node22-win-x64",
    );
  });

  it("picks the newest Node 22 macOS tarball for the current architecture", () => {
    const entries = [
      {
        type: "file",
        name: "node-v22.10.0-darwin-arm64.tar.gz",
        url: "https://registry.npmmirror.com/-/binary/node/latest-v22.x/node-v22.10.0-darwin-arm64.tar.gz",
      },
      {
        type: "file",
        name: "node-v22.11.0-darwin-x64.tar.gz",
        url: "https://registry.npmmirror.com/-/binary/node/latest-v22.x/node-v22.11.0-darwin-x64.tar.gz",
      },
      {
        type: "file",
        name: "node-v22.12.0-darwin-arm64.tar.gz",
        url: "https://registry.npmmirror.com/-/binary/node/latest-v22.x/node-v22.12.0-darwin-arm64.tar.gz",
      },
    ];

    expect(resolveLatestNode22DarwinTarballUrlFromMirror(entries, "arm64")).toBe(
      "https://registry.npmmirror.com/-/binary/node/latest-v22.x/node-v22.12.0-darwin-arm64.tar.gz",
    );
    expect(resolveLatestNode22DarwinTarballUrlFromMirror(entries, "x64")).toBe(
      "https://registry.npmmirror.com/-/binary/node/latest-v22.x/node-v22.11.0-darwin-x64.tar.gz",
    );
  });

  it("picks the newest Git for Windows 64-bit installer from npmmirror release entries", () => {
    const url = resolveLatestGitForWindowsInstallerUrlFromMirror(
      [
        {
          type: "dir",
          name: "v2.53.0.windows.3/",
          url: "https://registry.npmmirror.com/-/binary/git-for-windows/v2.53.0.windows.3/",
        },
        {
          type: "dir",
          name: "v2.54.0.windows.1/",
          url: "https://registry.npmmirror.com/-/binary/git-for-windows/v2.54.0.windows.1/",
        },
      ],
      [
        {
          type: "file",
          name: "Git-2.54.0-64-bit.exe",
          url: "https://registry.npmmirror.com/-/binary/git-for-windows/v2.54.0.windows.1/Git-2.54.0-64-bit.exe",
        },
        {
          type: "file",
          name: "PortableGit-2.54.0-64-bit.7z.exe",
          url: "https://registry.npmmirror.com/-/binary/git-for-windows/v2.54.0.windows.1/PortableGit-2.54.0-64-bit.7z.exe",
        },
      ],
    );

    expect(url).toBe(
      "https://registry.npmmirror.com/-/binary/git-for-windows/v2.54.0.windows.1/Git-2.54.0-64-bit.exe",
    );
  });

  it("picks PortableGit full distribution and ignores MinGit from npmmirror release entries", () => {
    const url = resolveLatestGitForWindowsPortableUrlFromMirror(
      [
        {
          type: "dir",
          name: "v2.54.0.windows.1/",
          url: "https://registry.npmmirror.com/-/binary/git-for-windows/v2.54.0.windows.1/",
        },
      ],
      [
        {
          type: "file",
          name: "MinGit-2.54.0-64-bit.zip",
          url: "https://registry.npmmirror.com/-/binary/git-for-windows/v2.54.0.windows.1/MinGit-2.54.0-64-bit.zip",
        },
        {
          type: "file",
          name: "PortableGit-2.54.0-64-bit.7z.exe",
          url: "https://registry.npmmirror.com/-/binary/git-for-windows/v2.54.0.windows.1/PortableGit-2.54.0-64-bit.7z.exe",
        },
      ],
    );

    expect(url).toBe(
      "https://registry.npmmirror.com/-/binary/git-for-windows/v2.54.0.windows.1/PortableGit-2.54.0-64-bit.7z.exe",
    );
  });

  it("builds silent direct installer commands for mirrored Node and Git installers", () => {
    const nodeCommand = buildWindowsNodeDirectInstallCommand(
      "https://registry.npmmirror.com/-/binary/node/latest-v22.x/node-v22.11.0-x64.msi",
    );
    expect(nodeCommand).toContain("powershell -NoProfile");
    expect(nodeCommand).toContain("Invoke-WebRequest");
    expect(nodeCommand).toContain("msiexec.exe");
    expect(nodeCommand).toContain("/qn");
    expect(nodeCommand).toContain("/norestart");
    expect(nodeCommand).toContain("/L*v");
    expect(nodeCommand).toContain("node --version");
    expect(nodeCommand).toContain("npm --version");

    const gitCommand = buildWindowsGitBashMirrorInstallCommand(
      "https://registry.npmmirror.com/-/binary/git-for-windows/v2.54.0.windows.1/Git-2.54.0-64-bit.exe",
    );
    expect(gitCommand).toContain("powershell -NoProfile");
    expect(gitCommand).toContain("Invoke-WebRequest");
    expect(gitCommand).toContain("/VERYSILENT");
    expect(gitCommand).toContain("/NORESTART");
    expect(gitCommand).toContain("/SUPPRESSMSGBOXES");
    expect(gitCommand).toContain("/CLOSEAPPLICATIONS");
    expect(gitCommand).toContain("/RESTARTAPPLICATIONS");
    expect(gitCommand).toContain("/CURRENTUSER");
    expect(gitCommand).toContain("/o:PathOption=Cmd");
    expect(gitCommand).toContain("/o:BashTerminalOption=MinTTY");
  });

  it("builds native PortableGit extractor arguments without shell wrappers", () => {
    const args = buildWindowsGitBashPortableExtractArgs(
      "C:\\Users\\alice\\.paseo\\toolchains\\PortableGit",
    );

    expect(args).toEqual([
      "-y",
      "-gm2",
      "-InstallPath=C:\\\\Users\\\\alice\\\\.paseo\\\\toolchains\\\\PortableGit",
    ]);
  });

  it("searches temporary PortableGit extraction directories when the SFX ignores InstallPath", () => {
    const dirs = resolveWindowsPortableGitTempFallbackDirs(
      "C:\\Users\\alice\\AppData\\Local\\Temp\\paseo-portable-git.7z.exe",
      {
        TEMP: "C:\\Users\\alice\\AppData\\Local\\Temp",
        TMP: "C:\\Users\\alice\\AppData\\Local\\Temp",
        LOCALAPPDATA: "C:\\Users\\alice\\AppData\\Local",
      },
    );

    expect(dirs).toEqual(["C:\\Users\\alice\\AppData\\Local\\Temp\\PortableGit"]);
  });

  it("builds app-managed macOS Node install commands without sudo", () => {
    const command = buildMacOSNodeDirectInstallCommand(
      "https://registry.npmmirror.com/-/binary/node/latest-v22.x/node-v22.12.0-darwin-arm64.tar.gz",
      "/Users/alice/.paseo/toolchains/node22",
    );

    expect(command).toContain("curl -fL --connect-timeout 20");
    expect(command).toContain("tar -xzf");
    expect(command).toContain("--strip-components 1");
    expect(command).toContain("'/Users/alice/.paseo/toolchains/node22/bin/node' --version");
    expect(command).toContain("'/Users/alice/.paseo/toolchains/node22/bin/npm' --version");
    expect(command).not.toContain("sudo");
    expect(command).not.toContain("installer -pkg");
  });

  it("summarizes Git Bash installation failures with validation details", () => {
    const message = buildWindowsGitInstallFailureMessage(
      [
        "PortableGit download: connection reset",
        "PortableGit verify: PortableGit extraction did not create git.exe or bash.exe",
        "Git installer verify: Git Bash was not found in app-managed PortableGit or Git for Windows paths.",
      ],
      {
        installed: false,
        version: null,
        bashPath: null,
        error: "Git Bash was not found in app-managed PortableGit or Git for Windows paths.",
      },
    );

    expect(message).toContain("Git Bash setup failed.");
    expect(message).toContain("Git Bash was not found");
    expect(message).toContain("PortableGit download");
    expect(message).toContain("PortableGit verify");
    expect(message).toContain("Attempts:");
    expect(message).not.toContain("Windows PATH");
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
    expect(command).toContain("/CURRENTUSER");
    expect(command).toContain("/o:PathOption=Cmd");
    expect(command).toContain("Invoke-WebRequest");
    expect(command).toContain("Start-Process");
  });

  it("builds Windows npm package install commands with npmmirror first", () => {
    const mirrorCommand = buildWindowsNpmPackageInstallCommand("@openai/codex", "npmmirror");
    expect(mirrorCommand).toBe(
      "npm install -g @openai/codex@latest --registry=https://registry.npmmirror.com --fetch-retries=2 --fetch-timeout=60000",
    );

    const officialCommand = buildWindowsNpmPackageInstallCommand(
      "@anthropic-ai/claude-code",
      "official",
    );
    expect(officialCommand).toBe("npm install -g @anthropic-ai/claude-code@latest");
  });
});

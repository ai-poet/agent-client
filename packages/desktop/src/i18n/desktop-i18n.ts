export type DesktopLocale = "en" | "zh";

type DesktopMessageParams = Record<string, string | number | null | undefined>;
type DesktopMessageFormatter = (params: DesktopMessageParams) => string;
type DesktopMessageKey = keyof typeof DESKTOP_MESSAGES.en;

const DESKTOP_MESSAGES = {
  en: {
    "provider.notFound": ({ id }) => `Provider not found: ${id ?? ""}`,
    "provider.notForClaude": () => "This endpoint does not apply to Claude Code.",
    "provider.notForCodex": () => "This endpoint does not apply to Codex.",
    "provider.notForGrok": () => "This endpoint does not apply to Grok.",
    "opener.unsupportedExternalUrl": () => "Unsupported external URL.",
    "cli.installFailed": ({ message, missingText }) =>
      `Install failed: ${message ?? ""}${missingText ? ` Missing: ${missingText}` : ""}`,
    "cli.gitBashSetupFailed": ({ validation, attemptsText }) =>
      `Git Bash setup failed. ${validation ?? ""}${attemptsText ? ` Attempts: ${attemptsText}` : ""}`,
    "cli.gitBashNotDetected": () => "Git Bash was not detected after installation.",
    "cli.downloadFailed": ({ status, url }) =>
      `Download failed with HTTP ${status ?? ""}: ${url ?? ""}`,
    "cli.windowsPathReadFailed": ({ message }) => `Windows user PATH read failed: ${message ?? ""}`,
    "cli.windowsPathWriteFailed": ({ message }) =>
      `Windows user PATH write failed: ${message ?? ""}`,
    "cli.windowsPathVerificationFailed": ({ message }) =>
      `Windows user PATH verification failed: ${message ?? ""}`,
    "cli.windowsPathMissingEntries": ({ entries }) =>
      `Windows user PATH verification failed. Missing persisted entries: ${entries ?? ""}`,
    "cli.nodeZipDownloadFailed": ({ message }) => `Node zip download failed: ${message ?? ""}`,
    "cli.nodeZipExtractFailed": ({ message }) => `Node zip extract failed: ${message ?? ""}`,
    "cli.nodeZipVerifyFailed": () =>
      "Node zip verify failed: extracted archive did not contain node.exe",
    "cli.portableGitDownloadFailed": ({ message }) => `PortableGit download: ${message ?? ""}`,
    "cli.portableGitExtractFailed": ({ message }) => `PortableGit extract: ${message ?? ""}`,
    "cli.portableGitVerifyFailed": ({ message }) => `PortableGit verify: ${message ?? ""}`,
    "cli.mirrorRequestFailed": ({ status, url }) =>
      `Mirror request failed with HTTP ${status ?? ""}: ${url ?? ""}`,
    "cli.mirrorDirectoryInvalid": ({ url }) =>
      `Mirror response was not a directory listing: ${url ?? ""}`,
    "cli.nodeMissingFromWindowsPath": () =>
      "Node.js and npm were not found in the Windows PATH. Install Node.js 22+ or add Node's install directory and %APPDATA%\\npm to PATH.",
    "cli.commandNoVersion": ({ command }) =>
      `${command ?? "CLI"} did not report a version. Ensure %APPDATA%\\npm is available in PATH.`,
    "cli.ensureNpmPath": ({ message }) =>
      `${message ?? ""} Ensure %APPDATA%\\npm is available in PATH.`,
    "cli.noNodeZip": () => "No Node.js 22 win-x64 zip was found on npmmirror.",
    "cli.noNodeMsi": () => "No Node.js 22 x64 MSI was found on npmmirror.",
    "cli.automaticNodeInstallFailed": ({ errors, version }) =>
      `Automatic Node.js 22 installation failed. ${errors ?? ""} Detected runtime: ${version ?? "unknown"}.`,
    "cli.noMacNodeTarball": ({ arch }) =>
      `No Node.js 22 macOS ${arch ?? ""} tarball was found on npmmirror.`,
    "cli.automaticNodeMirrorInstallFailed": ({ mirrorError }) =>
      `Automatic Node.js 22 installation failed via npmmirror${mirrorError ? `: ${mirrorError}` : ""}.`,
    "cli.automaticNodeUnsupported": ({ platform }) =>
      `Automatic Node.js 22 installation currently requires nvm or Homebrew in this environment. Detected ${platform ?? "unknown"}.`,
    "cli.noPortableGit": () => "No PortableGit 64-bit full distribution was found on npmmirror.",
    "cli.noGitInstaller": () => "No Git for Windows 64-bit installer was found on npmmirror.",
    "cli.packageInstallFailed": ({ packageName, errors }) =>
      `Failed to install ${packageName ?? "package"}. ${errors ?? ""}`,
    "cli.nodeVersionTooLow": ({ command, required, current }) =>
      `${command ?? "CLI"} requires Node.js ${required ?? ""} or newer. Detected ${current ?? "unknown"}.`,
  },
  zh: {
    "provider.notFound": ({ id }) => `未找到提供商：${id ?? ""}`,
    "provider.notForClaude": () => "此端点不适用于 Claude Code。",
    "provider.notForCodex": () => "此端点不适用于 Codex。",
    "provider.notForGrok": () => "此端点不适用于 Grok。",
    "opener.unsupportedExternalUrl": () => "不支持此外部 URL。",
    "cli.installFailed": ({ message, missingText }) =>
      `安装失败：${message ?? ""}${missingText ? ` 缺少：${missingText}` : ""}`,
    "cli.gitBashSetupFailed": ({ validation, attemptsText }) =>
      `Git Bash 设置失败。${validation ?? ""}${attemptsText ? ` 尝试记录：${attemptsText}` : ""}`,
    "cli.gitBashNotDetected": () => "安装后未检测到 Git Bash。",
    "cli.downloadFailed": ({ status, url }) => `下载失败，HTTP ${status ?? ""}：${url ?? ""}`,
    "cli.windowsPathReadFailed": ({ message }) => `读取 Windows 用户 PATH 失败：${message ?? ""}`,
    "cli.windowsPathWriteFailed": ({ message }) => `写入 Windows 用户 PATH 失败：${message ?? ""}`,
    "cli.windowsPathVerificationFailed": ({ message }) =>
      `验证 Windows 用户 PATH 失败：${message ?? ""}`,
    "cli.windowsPathMissingEntries": ({ entries }) =>
      `验证 Windows 用户 PATH 失败。缺少已持久化的条目：${entries ?? ""}`,
    "cli.nodeZipDownloadFailed": ({ message }) => `下载 Node zip 失败：${message ?? ""}`,
    "cli.nodeZipExtractFailed": ({ message }) => `解压 Node zip 失败：${message ?? ""}`,
    "cli.nodeZipVerifyFailed": () => "验证 Node zip 失败：解压后的归档中没有 node.exe",
    "cli.portableGitDownloadFailed": ({ message }) => `下载 PortableGit 失败：${message ?? ""}`,
    "cli.portableGitExtractFailed": ({ message }) => `解压 PortableGit 失败：${message ?? ""}`,
    "cli.portableGitVerifyFailed": ({ message }) => `验证 PortableGit 失败：${message ?? ""}`,
    "cli.mirrorRequestFailed": ({ status, url }) =>
      `镜像请求失败，HTTP ${status ?? ""}：${url ?? ""}`,
    "cli.mirrorDirectoryInvalid": ({ url }) => `镜像响应不是目录列表：${url ?? ""}`,
    "cli.nodeMissingFromWindowsPath": () =>
      "Windows PATH 中未找到 Node.js 和 npm。请安装 Node.js 22+，或将 Node 安装目录和 %APPDATA%\\npm 添加到 PATH。",
    "cli.commandNoVersion": ({ command }) =>
      `${command ?? "CLI"} 未返回版本号。请确保 %APPDATA%\\npm 已加入 PATH。`,
    "cli.ensureNpmPath": ({ message }) => `${message ?? ""} 请确保 %APPDATA%\\npm 已加入 PATH。`,
    "cli.noNodeZip": () => "npmmirror 上没有找到 Node.js 22 win-x64 zip。",
    "cli.noNodeMsi": () => "npmmirror 上没有找到 Node.js 22 x64 MSI。",
    "cli.automaticNodeInstallFailed": ({ errors, version }) =>
      `自动安装 Node.js 22 失败。${errors ?? ""} 检测到的运行时：${version ?? "unknown"}。`,
    "cli.noMacNodeTarball": ({ arch }) =>
      `npmmirror 上没有找到 Node.js 22 macOS ${arch ?? ""} tarball。`,
    "cli.automaticNodeMirrorInstallFailed": ({ mirrorError }) =>
      `通过 npmmirror 自动安装 Node.js 22 失败${mirrorError ? `：${mirrorError}` : ""}。`,
    "cli.automaticNodeUnsupported": ({ platform }) =>
      `当前环境自动安装 Node.js 22 需要 nvm 或 Homebrew。检测到的平台：${platform ?? "unknown"}。`,
    "cli.noPortableGit": () => "npmmirror 上没有找到 PortableGit 64-bit 完整发行版。",
    "cli.noGitInstaller": () => "npmmirror 上没有找到 Git for Windows 64-bit 安装包。",
    "cli.packageInstallFailed": ({ packageName, errors }) =>
      `安装 ${packageName ?? "package"} 失败。${errors ?? ""}`,
    "cli.nodeVersionTooLow": ({ command, required, current }) =>
      `${command ?? "CLI"} 需要 Node.js ${required ?? ""} 或更高版本。当前检测到 ${current ?? "unknown"}。`,
  },
} satisfies Record<DesktopLocale, Record<string, DesktopMessageFormatter>>;

let localeOverride: DesktopLocale | null = null;

export function normalizeDesktopLocale(locale: string | null | undefined): DesktopLocale {
  const normalized = locale?.toLowerCase().trim() ?? "";
  return normalized.startsWith("zh") ? "zh" : "en";
}

export function getDesktopLocale(): DesktopLocale {
  if (localeOverride) {
    return localeOverride;
  }
  return normalizeDesktopLocale(Intl.DateTimeFormat().resolvedOptions().locale);
}

export function getDesktopMessage(
  key: DesktopMessageKey,
  params: DesktopMessageParams = {},
): string {
  return DESKTOP_MESSAGES[getDesktopLocale()][key](params);
}

export function __setDesktopLocaleForTests(locale: DesktopLocale | null): void {
  localeOverride = locale;
}

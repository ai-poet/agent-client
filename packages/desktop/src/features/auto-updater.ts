import { app } from "electron";
import { autoUpdater, type UpdateInfo } from "electron-updater";
import { getDesktopBranding, normalizeDesktopUpdateUrl } from "../branding.js";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type AppUpdateCheckResult = {
  hasUpdate: boolean;
  readyToInstall: boolean;
  currentVersion: string;
  latestVersion: string;
  body: string | null;
  date: string | null;
  disabledReason: string | null;
};

export type AppUpdateInstallResult = {
  installed: boolean;
  version: string | null;
  message: string;
};

export type AppReleaseChannel = "stable" | "beta";

export type LatestUpdateInfo = {
  version: string;
  channelFile: string;
  url: string;
};

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

let cachedUpdateInfo: UpdateInfo | null = null;
let downloadedUpdateVersion: string | null = null;
let downloading = false;
let autoUpdaterConfigured = false;
let configuredReleaseChannel: AppReleaseChannel | null = null;
let configuredUpdateUrl: string | null = null;

const UPDATE_DISABLED_REASON = "Desktop updates are not configured for this brand.";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function resetUpdateState(): void {
  cachedUpdateInfo = null;
  downloadedUpdateVersion = null;
  downloading = false;
}

function normalizeVersion(value: string): string {
  return value.trim().replace(/^v/i, "");
}

type ParsedVersion = {
  core: number[];
  prerelease: string[];
};

function parseVersion(value: string): ParsedVersion | null {
  const normalized = normalizeVersion(value);
  const match = /^(\d+(?:\.\d+)*)(?:-([^+]+))?(?:\+.+)?$/.exec(normalized);
  if (!match) {
    return null;
  }

  return {
    core: match[1].split(".").map((part) => Number.parseInt(part, 10)),
    prerelease: match[2]?.split(".") ?? [],
  };
}

function comparePrereleaseIdentifier(left: string, right: string): number {
  const leftNumber = /^\d+$/.test(left) ? Number.parseInt(left, 10) : null;
  const rightNumber = /^\d+$/.test(right) ? Number.parseInt(right, 10) : null;

  if (leftNumber !== null && rightNumber !== null) {
    return Math.sign(leftNumber - rightNumber);
  }

  if (leftNumber !== null) {
    return -1;
  }

  if (rightNumber !== null) {
    return 1;
  }

  return left.localeCompare(right);
}

export function compareAppVersions(left: string, right: string): number {
  const parsedLeft = parseVersion(left);
  const parsedRight = parseVersion(right);
  if (!parsedLeft || !parsedRight) {
    return 0;
  }

  const coreLength = Math.max(parsedLeft.core.length, parsedRight.core.length);
  for (let index = 0; index < coreLength; index += 1) {
    const diff = (parsedLeft.core[index] ?? 0) - (parsedRight.core[index] ?? 0);
    if (diff !== 0) {
      return Math.sign(diff);
    }
  }

  if (parsedLeft.prerelease.length === 0 && parsedRight.prerelease.length === 0) {
    return 0;
  }

  if (parsedLeft.prerelease.length === 0) {
    return 1;
  }

  if (parsedRight.prerelease.length === 0) {
    return -1;
  }

  const prereleaseLength = Math.max(parsedLeft.prerelease.length, parsedRight.prerelease.length);
  for (let index = 0; index < prereleaseLength; index += 1) {
    const leftPart = parsedLeft.prerelease[index];
    const rightPart = parsedRight.prerelease[index];
    if (leftPart === undefined) {
      return -1;
    }
    if (rightPart === undefined) {
      return 1;
    }
    const diff = comparePrereleaseIdentifier(leftPart, rightPart);
    if (diff !== 0) {
      return Math.sign(diff);
    }
  }

  return 0;
}

export function isRemoteVersionNewer(remoteVersion: string, currentVersion: string): boolean {
  return compareAppVersions(remoteVersion, currentVersion) > 0;
}

export function channelFileForPlatform(
  releaseChannel: AppReleaseChannel,
  platform: NodeJS.Platform = process.platform,
): string {
  const channel = releaseChannel === "beta" ? "beta" : "latest";
  if (platform === "darwin") {
    return `${channel}-mac.yml`;
  }
  if (platform === "linux") {
    return `${channel}-linux.yml`;
  }
  return `${channel}.yml`;
}

export function parseLatestVersionFromUpdateInfo(rawYaml: string): string | null {
  const match = /^version:\s*['"]?([^'"\s]+)['"]?/m.exec(rawYaml);
  return match?.[1] ?? null;
}

export async function fetchLatestUpdateInfo({
  updateUrl,
  releaseChannel,
  platform = process.platform,
  fetcher = fetch,
}: {
  updateUrl: string;
  releaseChannel: AppReleaseChannel;
  platform?: NodeJS.Platform;
  fetcher?: typeof fetch;
}): Promise<LatestUpdateInfo | null> {
  const normalizedUpdateUrl = normalizeDesktopUpdateUrl(updateUrl);
  if (!normalizedUpdateUrl) {
    return null;
  }

  const channelFile = channelFileForPlatform(releaseChannel, platform);
  const url = new URL(channelFile, normalizedUpdateUrl);
  url.searchParams.set("noCache", Date.now().toString(32));

  const response = await fetcher(url, {
    headers: {
      "Cache-Control": "no-cache",
      "User-Agent": "Paseo Desktop",
    },
  });

  if (!response.ok) {
    throw new Error(`Update metadata request failed: HTTP ${response.status}`);
  }

  const version = parseLatestVersionFromUpdateInfo(await response.text());
  return version ? { version, channelFile, url: url.toString() } : null;
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

function configureAutoUpdater({
  releaseChannel,
  updateUrl,
}: {
  releaseChannel: AppReleaseChannel;
  updateUrl: string;
}): void {
  const normalizedUpdateUrl = normalizeDesktopUpdateUrl(updateUrl);
  if (!normalizedUpdateUrl) {
    throw new Error(UPDATE_DISABLED_REASON);
  }

  // Download updates in the background and only prompt once they are ready to install.
  autoUpdater.autoDownload = true;
  autoUpdater.autoInstallOnAppQuit = true;

  // Suppress built-in dialogs; the renderer handles UI.
  autoUpdater.autoRunAppAfterInstall = true;
  autoUpdater.allowPrerelease = releaseChannel === "beta";
  autoUpdater.channel = releaseChannel === "beta" ? "beta" : "latest";
  autoUpdater.allowDowngrade = false;

  if (configuredReleaseChannel !== releaseChannel || configuredUpdateUrl !== normalizedUpdateUrl) {
    resetUpdateState();
    configuredReleaseChannel = releaseChannel;
    configuredUpdateUrl = normalizedUpdateUrl;
    autoUpdater.setFeedURL({
      provider: "generic",
      url: normalizedUpdateUrl,
    });
  }

  if (autoUpdaterConfigured) {
    return;
  }

  autoUpdaterConfigured = true;

  autoUpdater.on("update-available", (info) => {
    cachedUpdateInfo = info;
    downloadedUpdateVersion = null;
    downloading = true;
  });

  autoUpdater.on("update-downloaded", (info) => {
    cachedUpdateInfo = info;
    downloadedUpdateVersion = info.version;
    downloading = false;
  });

  autoUpdater.on("update-not-available", () => {
    resetUpdateState();
  });

  autoUpdater.on("error", (error) => {
    downloading = false;
    console.error("[auto-updater] Updater event failed:", error);
  });
}

function isReadyToInstallVersion(version: string): boolean {
  return downloadedUpdateVersion === version;
}

function buildCheckResult(input: {
  currentVersion: string;
  hasUpdate: boolean;
  readyToInstall: boolean;
  info?: UpdateInfo | null;
  latestVersion?: string | null;
  disabledReason?: string | null;
}): AppUpdateCheckResult {
  const { currentVersion, hasUpdate, readyToInstall, info, latestVersion, disabledReason } = input;

  return {
    hasUpdate,
    readyToInstall,
    currentVersion,
    latestVersion: latestVersion ?? info?.version ?? currentVersion,
    body: typeof info?.releaseNotes === "string" ? info.releaseNotes : null,
    date: typeof info?.releaseDate === "string" ? info.releaseDate : null,
    disabledReason: disabledReason ?? null,
  };
}

function scheduleQuitAndInstall(onBeforeQuit?: () => Promise<void>): void {
  // Use a short delay to allow the renderer to receive the response.
  setTimeout(async () => {
    try {
      if (onBeforeQuit) await onBeforeQuit();
      autoUpdater.quitAndInstall(/* isSilent */ false, /* isForceRunAfter */ true);
    } catch (error) {
      console.error("[auto-updater] quitAndInstall failed:", error);
    }
  }, 1500);
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export async function checkForAppUpdate({
  currentVersion,
  releaseChannel,
}: {
  currentVersion: string;
  releaseChannel: AppReleaseChannel;
}): Promise<AppUpdateCheckResult> {
  if (!app.isPackaged) {
    return buildCheckResult({
      currentVersion,
      hasUpdate: false,
      readyToInstall: false,
    });
  }

  const updateUrl = getDesktopBranding().desktopUpdateUrl;
  if (!updateUrl) {
    resetUpdateState();
    return buildCheckResult({
      currentVersion,
      hasUpdate: false,
      readyToInstall: false,
      disabledReason: UPDATE_DISABLED_REASON,
    });
  }

  configureAutoUpdater({ releaseChannel, updateUrl });

  const cachedVersion = cachedUpdateInfo?.version ?? null;
  if (cachedVersion && isRemoteVersionNewer(cachedVersion, currentVersion)) {
    return buildCheckResult({
      currentVersion,
      hasUpdate: true,
      readyToInstall: isReadyToInstallVersion(cachedVersion),
      info: cachedUpdateInfo,
    });
  }

  try {
    try {
      const latest = await fetchLatestUpdateInfo({ updateUrl, releaseChannel });
      if (latest && !isRemoteVersionNewer(latest.version, currentVersion)) {
        resetUpdateState();
        return buildCheckResult({
          currentVersion,
          latestVersion: latest.version,
          hasUpdate: false,
          readyToInstall: false,
        });
      }
    } catch (error) {
      console.warn("[auto-updater] Failed to preflight update metadata:", error);
    }

    const result = await autoUpdater.checkForUpdates();

    if (!result || !result.updateInfo) {
      return buildCheckResult({
        currentVersion,
        hasUpdate: false,
        readyToInstall: false,
      });
    }

    const info = result.updateInfo;
    const latestVersion = info.version;
    const hasUpdate = isRemoteVersionNewer(latestVersion, currentVersion);

    if (hasUpdate) {
      cachedUpdateInfo = info;
      downloading = !isReadyToInstallVersion(latestVersion);
      return buildCheckResult({
        currentVersion,
        hasUpdate: true,
        readyToInstall: isReadyToInstallVersion(latestVersion),
        info,
      });
    }

    resetUpdateState();

    return buildCheckResult({
      currentVersion,
      latestVersion,
      hasUpdate: false,
      readyToInstall: false,
    });
  } catch (error) {
    console.error("[auto-updater] Failed to check for updates:", error);
    return buildCheckResult({
      currentVersion,
      hasUpdate: false,
      readyToInstall: false,
    });
  }
}

export async function downloadAndInstallUpdate(
  {
    currentVersion,
    releaseChannel,
  }: {
    currentVersion: string;
    releaseChannel: AppReleaseChannel;
  },
  onBeforeQuit?: () => Promise<void>,
): Promise<AppUpdateInstallResult> {
  if (!app.isPackaged) {
    return {
      installed: false,
      version: currentVersion,
      message: "Auto-update is not available in development mode.",
    };
  }

  const updateUrl = getDesktopBranding().desktopUpdateUrl;
  if (!updateUrl) {
    resetUpdateState();
    return {
      installed: false,
      version: currentVersion,
      message: UPDATE_DISABLED_REASON,
    };
  }

  if (!cachedUpdateInfo) {
    return {
      installed: false,
      version: currentVersion,
      message: "No update available. Check for updates first.",
    };
  }

  configureAutoUpdater({ releaseChannel, updateUrl });

  const readyVersion = cachedUpdateInfo.version;
  if (isReadyToInstallVersion(readyVersion)) {
    scheduleQuitAndInstall(onBeforeQuit);
    return {
      installed: true,
      version: readyVersion,
      message: "Update downloaded. The app will restart shortly.",
    };
  }

  if (downloading) {
    return {
      installed: false,
      version: currentVersion,
      message: "Update is still being prepared. Try again in a moment.",
    };
  }

  downloading = true;

  try {
    await autoUpdater.downloadUpdate();
    downloadedUpdateVersion = readyVersion;
    downloading = false;
    scheduleQuitAndInstall(onBeforeQuit);

    return {
      installed: true,
      version: readyVersion,
      message: "Update downloaded. The app will restart shortly.",
    };
  } catch (error) {
    downloading = false;
    const message = error instanceof Error ? error.message : String(error);
    console.error("[auto-updater] Failed to download/install update:", message);
    return {
      installed: false,
      version: currentVersion,
      message: `Update failed: ${message}`,
    };
  }
}

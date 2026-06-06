import log from "electron-log/main";
log.transports.console.level = "info";
log.initialize({ spyRendererConsole: true });

import { inheritLoginShellEnv } from "./login-shell-env.js";
inheritLoginShellEnv();

import path from "node:path";
import { pathToFileURL } from "node:url";
import { existsSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { app, BrowserWindow, ipcMain, nativeImage, net, protocol } from "electron";
import { registerDaemonManager } from "./daemon/daemon-manager.js";
import {
  parseCliPassthroughArgsFromArgv,
  runCliPassthroughCommand,
} from "./daemon/runtime-paths.js";
import { closeAllTransportSessions } from "./daemon/local-transport.js";
import {
  registerWindowManager,
  getMainWindowChromeOptions,
  getWindowBackgroundColor,
  resolveSystemWindowTheme,
  setupWindowResizeEvents,
  setupDefaultContextMenu,
  setupDragDropPrevention,
} from "./window/window-manager.js";
import { registerDialogHandlers } from "./features/dialogs.js";
import {
  registerNotificationHandlers,
  ensureNotificationCenterRegistration,
} from "./features/notifications.js";
import { registerOpenerHandlers } from "./features/opener.js";
import { setupApplicationMenu } from "./features/menu.js";
import { parseOpenProjectPathFromArgv } from "./open-project-routing.js";
import { getDesktopBranding } from "./branding.js";

const DEV_SERVER_URL = process.env.EXPO_DEV_URL ?? "http://localhost:8081";
const APP_SCHEME = "paseo";
const OPEN_PROJECT_EVENT = "paseo:event:open-project";
const AUTH_CALLBACK_EVENT = "paseo:event:auth-callback";
const desktopBranding = getDesktopBranding();
app.setName(desktopBranding.appName);

function normalizeAuthCallbackUrl(rawUrl: string | null | undefined): string | null {
  if (typeof rawUrl !== "string") {
    return null;
  }

  const trimmed = rawUrl.trim();
  if (!trimmed) {
    return null;
  }

  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== `${APP_SCHEME}:`) {
      return null;
    }

    const route = `${parsed.host}${parsed.pathname}`.replace(/\/+$/, "");
    return route === "auth/callback" ? trimmed : null;
  } catch {
    return null;
  }
}

function parseAuthCallbackUrlFromArgv(argv: string[]): string | null {
  for (const arg of argv) {
    const authCallbackUrl = normalizeAuthCallbackUrl(arg);
    if (authCallbackUrl) {
      return authCallbackUrl;
    }
  }
  return null;
}

// In dev mode, detect git worktrees and isolate each instance so multiple
// Electron windows can run side-by-side (separate userData = separate lock).
let devWorktreeName: string | null = null;
if (!app.isPackaged) {
  try {
    const topLevel = execFileSync("git", ["rev-parse", "--show-toplevel"], {
      encoding: "utf-8",
      timeout: 3000,
    }).trim();
    devWorktreeName = path.basename(topLevel);
    // Main checkout (e.g. "paseo") gets default userData — only worktrees diverge.
    const commonDir = path.resolve(
      topLevel,
      execFileSync("git", ["rev-parse", "--git-common-dir"], {
        cwd: topLevel,
        encoding: "utf-8",
        timeout: 3000,
      }).trim(),
    );
    const isWorktree = path.resolve(topLevel, ".git") !== commonDir;
    if (isWorktree) {
      app.setPath(
        "userData",
        path.join(app.getPath("appData"), `${desktopBranding.appName}-${devWorktreeName}`),
      );
      log.info("[worktree] isolated userData for worktree:", devWorktreeName);
    } else {
      devWorktreeName = null;
    }
  } catch {
    devWorktreeName = null;
  }
}

// Allow users to pass Chromium flags via PASEO_ELECTRON_FLAGS for debugging
// rendering issues (e.g. "--disable-gpu --ozone-platform=x11").
// Must run before app.whenReady().
const electronFlags = process.env.PASEO_ELECTRON_FLAGS?.trim();
if (electronFlags) {
  for (const token of electronFlags.split(/\s+/)) {
    const [key, ...rest] = token.replace(/^--/, "").split("=");
    app.commandLine.appendSwitch(key, rest.join("=") || undefined);
  }
  log.info("[electron-flags]", electronFlags);
}

let pendingOpenProjectPath = parseOpenProjectPathFromArgv({
  argv: process.argv,
  isDefaultApp: process.defaultApp,
});
let pendingAuthCallbackUrl = parseAuthCallbackUrlFromArgv(process.argv);

log.info("[open-project] argv:", process.argv);
log.info("[open-project] isDefaultApp:", process.defaultApp);
log.info("[open-project] pendingOpenProjectPath:", pendingOpenProjectPath);
log.info("[auth-callback] pendingAuthCallbackUrl:", pendingAuthCallbackUrl);

// The renderer pulls the pending path on mount via IPC — this avoids
// a race where the push event arrives before React registers its listener.
ipcMain.handle("paseo:get-pending-open-project", () => {
  log.info("[open-project] renderer requested pending path:", pendingOpenProjectPath);
  const result = pendingOpenProjectPath;
  pendingOpenProjectPath = null;
  return result;
});

ipcMain.handle("paseo:get-pending-auth-callback", () => {
  log.info("[auth-callback] renderer requested pending url:", pendingAuthCallbackUrl);
  const result = pendingAuthCallbackUrl;
  pendingAuthCallbackUrl = null;
  return result;
});

protocol.registerSchemesAsPrivileged([
  { scheme: APP_SCHEME, privileges: { standard: true, secure: true, supportFetchAPI: true } },
]);

// ---------------------------------------------------------------------------
// Window creation
// ---------------------------------------------------------------------------

function getPreloadPath(): string {
  return path.join(__dirname, "preload.js");
}

function getAppDistDir(): string {
  if (app.isPackaged) {
    return path.join(process.resourcesPath, "app-dist");
  }

  return path.resolve(__dirname, "../../app/dist");
}

function resolveConfiguredIconPath(iconPath: string): string {
  if (path.isAbsolute(iconPath)) {
    return iconPath;
  }
  return path.resolve(__dirname, "..", iconPath);
}

function getConfiguredWindowIconCandidates(): string[] {
  if (process.platform === "win32") {
    return [
      resolveConfiguredIconPath(desktopBranding.desktopIconWin),
      resolveConfiguredIconPath(desktopBranding.desktopIconPng),
    ];
  }

  return [resolveConfiguredIconPath(desktopBranding.desktopIconPng)];
}

function getWindowIconPath(): string | null {
  const packagedCandidates = app.isPackaged
    ? process.platform === "win32"
      ? [path.join(process.resourcesPath, "icon.ico"), path.join(process.resourcesPath, "icon.png")]
      : [path.join(process.resourcesPath, "icon.png")]
    : [];

  const candidates = [...getConfiguredWindowIconCandidates(), ...packagedCandidates];
  return candidates.find((candidate) => existsSync(candidate)) ?? null;
}

function applyAppIcon(): void {
  if (process.platform !== "darwin") {
    return;
  }

  const iconPath = getWindowIconPath();
  if (!iconPath || !existsSync(iconPath)) {
    return;
  }

  const icon = nativeImage.createFromPath(iconPath);
  if (icon.isEmpty()) {
    return;
  }

  app.dock?.setIcon(icon);
}

async function createMainWindow(): Promise<void> {
  const iconPath = getWindowIconPath();
  const systemTheme = resolveSystemWindowTheme();

  const title = devWorktreeName
    ? `${desktopBranding.appName} (${devWorktreeName})`
    : desktopBranding.appName;
  const mainWindow = new BrowserWindow({
    title,
    width: 1400,
    height: 900,
    show: false,
    backgroundColor: getWindowBackgroundColor(systemTheme),
    ...(iconPath ? { icon: iconPath } : {}),
    ...getMainWindowChromeOptions({
      platform: process.platform,
      theme: systemTheme,
    }),
    webPreferences: {
      preload: getPreloadPath(),
      contextIsolation: true,
      nodeIntegration: false,
    },
  });

  if (devWorktreeName) {
    app.dock?.setBadge(devWorktreeName);
  }

  setupWindowResizeEvents(mainWindow);
  setupDefaultContextMenu(mainWindow);
  setupDragDropPrevention(mainWindow);

  mainWindow.once("ready-to-show", () => {
    mainWindow.show();
  });

  if (!app.isPackaged) {
    const { loadReactDevTools } = await import("./features/react-devtools.js");
    await loadReactDevTools();
    await mainWindow.loadURL(DEV_SERVER_URL);
    mainWindow.webContents.openDevTools({ mode: "detach" });
    return;
  }

  await mainWindow.loadURL(`${APP_SCHEME}://app/`);
}

function sendOpenProjectEvent(win: BrowserWindow, projectPath: string): void {
  const send = () => {
    log.info("[open-project] sending event to renderer:", projectPath);
    win.webContents.send(OPEN_PROJECT_EVENT, { path: projectPath });
  };

  if (win.webContents.isLoadingMainFrame()) {
    log.info("[open-project] waiting for did-finish-load before sending event");
    win.webContents.once("did-finish-load", send);
    return;
  }

  send();
}

function sendAuthCallbackEvent(win: BrowserWindow, callbackUrl: string): void {
  const send = () => {
    log.info("[auth-callback] sending event to renderer:", callbackUrl);
    win.webContents.send(AUTH_CALLBACK_EVENT, { url: callbackUrl });
  };

  if (win.webContents.isLoadingMainFrame()) {
    log.info("[auth-callback] waiting for did-finish-load before sending event");
    win.webContents.once("did-finish-load", send);
    return;
  }

  send();
}

function revealAndFocusWindow(win: BrowserWindow): void {
  win.show();
  if (win.isMinimized()) {
    win.restore();
  }
  win.focus();
}

function forwardAuthCallback(url: string): void {
  const authCallbackUrl = normalizeAuthCallbackUrl(url);
  if (!authCallbackUrl) {
    return;
  }

  const win = BrowserWindow.getAllWindows()[0];
  if (!win || win.webContents.isLoadingMainFrame()) {
    pendingAuthCallbackUrl = authCallbackUrl;
  }

  if (win) {
    sendAuthCallbackEvent(win, authCallbackUrl);
    revealAndFocusWindow(win);
  }
}

function registerAppProtocolClient(): void {
  const registered =
    process.defaultApp && process.argv.length >= 2
      ? app.setAsDefaultProtocolClient(APP_SCHEME, process.execPath, [
          path.resolve(process.argv[1]),
        ])
      : app.setAsDefaultProtocolClient(APP_SCHEME);

  log.info("[protocol] registered app protocol client", {
    scheme: APP_SCHEME,
    registered,
    defaultApp: process.defaultApp,
  });
}

// ---------------------------------------------------------------------------
// App lifecycle
// ---------------------------------------------------------------------------

function setupSingleInstanceLock(): boolean {
  const gotLock = app.requestSingleInstanceLock();
  if (!gotLock) {
    app.quit();
    return false;
  }

  app.on("second-instance", (_event, commandLine) => {
    log.info("[open-project] second-instance commandLine:", commandLine);
    const openProjectPath = parseOpenProjectPathFromArgv({
      argv: commandLine,
      isDefaultApp: false,
    });
    const authCallbackUrl = parseAuthCallbackUrlFromArgv(commandLine);
    log.info("[open-project] second-instance openProjectPath:", openProjectPath);
    log.info("[auth-callback] second-instance url:", authCallbackUrl);
    const win = BrowserWindow.getAllWindows()[0];
    if (win) {
      revealAndFocusWindow(win);
      if (openProjectPath) {
        sendOpenProjectEvent(win, openProjectPath);
      }
      if (authCallbackUrl) {
        forwardAuthCallback(authCallbackUrl);
      }
    } else if (authCallbackUrl) {
      pendingAuthCallbackUrl = authCallbackUrl;
    }
  });

  return true;
}

async function runCliPassthroughIfRequested(): Promise<boolean> {
  const cliArgs = parseCliPassthroughArgsFromArgv(process.argv);
  if (!cliArgs) {
    return false;
  }

  try {
    const exitCode = runCliPassthroughCommand(cliArgs);
    process.exit(exitCode);
  } catch (error) {
    const message = error instanceof Error ? (error.stack ?? error.message) : String(error);
    process.stderr.write(`${message}\n`);
    process.exit(1);
  }

  return true;
}

async function bootstrap(): Promise<void> {
  if (
    !pendingOpenProjectPath &&
    !pendingAuthCallbackUrl &&
    (await runCliPassthroughIfRequested())
  ) {
    return;
  }

  if (!setupSingleInstanceLock()) {
    return;
  }

  registerAppProtocolClient();
  await app.whenReady();

  const appDistDir = getAppDistDir();
  protocol.handle(APP_SCHEME, (request) => {
    const parsedUrl = new URL(request.url);
    const { host, pathname, search, hash } = parsedUrl;
    const decodedPath = decodeURIComponent(pathname);

    // Handle OAuth callback: paseo://auth/callback#access_token=...
    if (host === "auth" && (decodedPath === "/callback" || decodedPath.startsWith("/callback/"))) {
      forwardAuthCallback(request.url);
      // Return a simple HTML page that closes itself
      return new Response(
        "<html><body><script>window.close()</script><p>Login complete. You can close this window.</p></body></html>",
        { headers: { "Content-Type": "text/html" } },
      );
    }

    // Chromium can occasionally request the exported entrypoint directly.
    // Canonicalize it back to the route URL so Expo Router sees `/`, not `/index.html`.
    if (decodedPath.endsWith("/index.html")) {
      const normalizedPath = decodedPath.slice(0, -"/index.html".length) || "/";
      return Response.redirect(`${APP_SCHEME}://app${normalizedPath}${search}${hash}`, 307);
    }

    const filePath = path.join(appDistDir, decodedPath);
    const relativePath = path.relative(appDistDir, filePath);

    if (relativePath.startsWith("..") || path.isAbsolute(relativePath)) {
      return new Response("Not found", { status: 404 });
    }

    // SPA fallback: serve index.html for routes without a file extension
    if (!relativePath || !path.extname(relativePath)) {
      return net.fetch(pathToFileURL(path.join(appDistDir, "index.html")).toString());
    }

    return net.fetch(pathToFileURL(filePath).toString());
  });

  applyAppIcon();
  setupApplicationMenu();
  ensureNotificationCenterRegistration();
  registerDaemonManager();
  registerWindowManager();
  registerDialogHandlers();
  registerNotificationHandlers();
  registerOpenerHandlers();
  await createMainWindow();
  if (pendingAuthCallbackUrl) {
    const win = BrowserWindow.getAllWindows()[0];
    if (win) {
      sendAuthCallbackEvent(win, pendingAuthCallbackUrl);
    }
  }

  app.on("activate", async () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      await createMainWindow();
      if (pendingAuthCallbackUrl) {
        const win = BrowserWindow.getAllWindows()[0];
        if (win) {
          sendAuthCallbackEvent(win, pendingAuthCallbackUrl);
        }
      }
    }
  });
}

void bootstrap().catch((error) => {
  const message = error instanceof Error ? (error.stack ?? error.message) : String(error);
  process.stderr.write(`${message}\n`);
  process.exit(1);
});

app.on("before-quit", () => {
  closeAllTransportSessions();
});

app.on("open-url", (event, url) => {
  event.preventDefault();
  log.info("[open-url] received:", url);
  forwardAuthCallback(url);
});

app.on("window-all-closed", () => {
  if (process.platform !== "darwin") {
    app.quit();
  }
});

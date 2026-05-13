#!/usr/bin/env node
import { spawn } from "node:child_process";

const [, , ...args] = process.argv;

if (args.length === 0) {
  console.error("Usage: npm run with:cyberaicoding -- <command> [...args]");
  process.exit(1);
}

const cyberAiCodingEnv = {
  PASEO_APP_NAME: "CyberAICoding",
  PASEO_APP_NAME_DEVELOPMENT: "CyberAICoding",
  PASEO_CLOUD_NAME: "CyberAICoding Cloud",
  PASEO_LOGO_VARIANT: "cybercode",
  EXPO_PUBLIC_PASEO_APP_NAME: "CyberAICoding",
  EXPO_PUBLIC_PASEO_CLOUD_NAME: "CyberAICoding Cloud",
  EXPO_PUBLIC_PASEO_LOGO_VARIANT: "cybercode",
  PASEO_EXPO_ICON: "./assets/images/cybercode-icon.png",
  PASEO_EXPO_ANDROID_FOREGROUND_ICON: "./assets/images/cybercode-android-icon-foreground.png",
  PASEO_EXPO_SPLASH_ICON: "./assets/images/cybercode-splash-icon.png",
  PASEO_EXPO_NOTIFICATION_ICON: "./assets/images/cybercode-notification-icon.png",
  PASEO_WEB_FAVICON: "./assets/images/cybercode-favicon.png",
  PASEO_DESKTOP_APP_ID: "com.cyberaicoding.desktop",
  PASEO_DESKTOP_ICON_PNG: "assets/cybercode-icon.png",
  PASEO_DESKTOP_ICON_MAC: "assets/cybercode-icon.icns",
  PASEO_DESKTOP_ICON_WIN: "assets/cybercode-icon.ico",
  PASEO_DESKTOP_ICON_LINUX: "assets/cybercode",
  PASEO_DESKTOP_UPDATE_URL: "https://minio.cyberspirit.io/",
};

const [command, ...commandArgs] = args;
const child = spawn(command, commandArgs, {
  stdio: "inherit",
  shell: process.platform === "win32",
  env: {
    ...process.env,
    ...cyberAiCodingEnv,
  },
});

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code ?? 1);
});

child.on("error", (error) => {
  console.error(error);
  process.exit(1);
});

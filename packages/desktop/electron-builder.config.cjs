const { resolveDesktopBrandingFromEnv } = require("./branding.cjs");

const brand = resolveDesktopBrandingFromEnv(process.env);
const artifactName = `${brand.appName}-\${version}-\${arch}.\${ext}`;
const publish = brand.desktopUpdateUrl
  ? [
      {
        provider: "generic",
        url: brand.desktopUpdateUrl,
      },
    ]
  : null;

module.exports = {
  npmRebuild: false,
  appId: brand.desktopAppId,
  productName: brand.appName,
  executableName: brand.appName,
  afterPack: "./scripts/after-pack.js",
  directories: {
    output: "release",
  },
  files: ["dist/**/*"],
  asarUnpack: ["dist/daemon/node-entrypoint-runner.js"],
  extraResources: [
    { from: "../app/dist", to: "app-dist" },
    { from: "../../skills", to: "skills" },
    { from: brand.desktopIconPng, to: "icon.png" },
    { from: brand.desktopIconWin, to: "icon.ico" },
  ],
  publish,
  mac: {
    artifactName,
    category: "public.app-category.developer-tools",
    icon: brand.desktopIconMac,
    hardenedRuntime: true,
    notarize: true,
    entitlements: "build/entitlements.mac.plist",
    entitlementsInherit: "build/entitlements.mac.inherit.plist",
    extraResources: [{ from: "bin/paseo", to: "bin/paseo" }],
    target: ["dmg", "zip"],
  },
  linux: {
    category: "Development",
    icon: brand.desktopIconLinux,
    artifactName,
    maintainer: "Mohamed Boudra <hello@moboudra.com>",
    vendor: brand.appName,
    extraResources: [{ from: "bin/paseo", to: "bin/paseo" }],
    target: ["AppImage", "deb", "rpm", "tar.gz"],
  },
  win: {
    artifactName,
    icon: brand.desktopIconWin,
    extraResources: [{ from: "bin/paseo.cmd", to: "bin/paseo.cmd" }],
    target: ["nsis", "zip"],
  },
  nsis: {
    oneClick: false,
    perMachine: false,
    allowToChangeInstallationDirectory: true,
  },
};

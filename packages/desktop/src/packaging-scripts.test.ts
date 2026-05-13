import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";

const desktopPackageRoot = path.resolve(__dirname, "..");

function copyScriptToTemp(scriptRelativePath: string): string {
  const sourcePath = path.join(desktopPackageRoot, scriptRelativePath);
  const tempDir = mkdtempSync(path.join(tmpdir(), "desktop-script-"));
  const targetPath = path.join(tempDir, path.basename(scriptRelativePath));
  writeFileSync(targetPath, readFileSync(sourcePath));
  return targetPath;
}

describe("desktop packaging scripts", () => {
  it("after-pack derives the executable name from electron-builder context", async () => {
    const scriptPath = copyScriptToTemp("scripts/after-pack.js");
    const afterPack = await import(`${scriptPath}?cacheBust=${Date.now()}`);

    expect(afterPack.resolveExecutableNameForContext({ packager: { appInfo: {} } })).toBe("Paseo");
    expect(
      afterPack.resolveExecutableNameForContext({
        packager: { appInfo: { productFilename: "CyberAICoding" } },
      }),
    ).toBe("CyberAICoding");
  });

  it("after-pack uses the branded executable name for macOS resource pruning", async () => {
    const scriptPath = copyScriptToTemp("scripts/after-pack.js");
    const afterPack = await import(`${scriptPath}?cacheBust=${Date.now()}`);

    expect(
      afterPack.resolveResourcesDir({
        appOutDir: "/tmp/out/mac-arm64",
        platform: "darwin",
        executableName: "CyberAICoding",
      }),
    ).toBe("/tmp/out/mac-arm64/CyberAICoding.app/Contents/Resources");
  });

  it("after-pack writes executable metadata into the bundled bin directory", async () => {
    const scriptPath = copyScriptToTemp("scripts/after-pack.js");
    const afterPack = await import(`${scriptPath}?cacheBust=${Date.now()}`);
    const appOutDir = mkdtempSync(path.join(tmpdir(), "desktop-after-pack-"));
    const resourcesDir = path.join(appOutDir, "CyberAICoding.app", "Contents", "Resources");
    mkdirSync(path.join(resourcesDir, "bin"), { recursive: true });

    afterPack.writeExecutableMetadata({
      appOutDir,
      platform: "darwin",
      executableName: "CyberAICoding",
    });

    const metadataPath = path.join(resourcesDir, "bin", "app-executable-name");
    expect(existsSync(metadataPath)).toBe(true);
    expect(readFileSync(metadataPath, "utf-8")).toBe("CyberAICoding\n");
  });

  it("unix shim discovers the sibling macOS executable without hard-coding Paseo", () => {
    const script = readFileSync(path.join(desktopPackageRoot, "bin/paseo"), "utf-8");

    expect(script).toContain('EXECUTABLE_NAME_FILE="${RESOURCES_DIR}/bin/app-executable-name"');
    expect(script).toContain('MACOS_DIR="${RESOURCES_DIR}/../MacOS"');
    expect(script).toContain("find");
    expect(script).not.toContain("../MacOS/Paseo");
    expect(script).not.toContain("../Paseo");
  });

  it("windows shim discovers the packaged executable without hard-coding Paseo.exe", () => {
    const script = readFileSync(path.join(desktopPackageRoot, "bin/paseo.cmd"), "utf-8");

    expect(script).toContain(
      'set "EXECUTABLE_NAME_FILE=%RESOURCES_DIR%\\bin\\app-executable-name"',
    );
    expect(script).toContain('for %%F in ("%RESOURCES_DIR%\\..\\*.exe") do');
    expect(script).not.toContain("Paseo.exe");
  });
});

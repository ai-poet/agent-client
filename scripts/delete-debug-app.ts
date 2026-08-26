#!/usr/bin/env bun

import { lstat, readdir, rm } from "node:fs/promises";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline/promises";

const projectRoot = resolve(import.meta.dir, "..");
const userHome = homedir();
const library = join(userHome, "Library");
const debugBundleIdentifiers = ["sh.waku.dev", "codes.waku.dev"];

type Target = {
  path: string;
  kind: "directory" | "file" | "link";
};

const candidatePaths = new Set<string>();

function addCandidate(path: string): void {
  candidatePaths.add(resolve(path));
}

function isDebugDiagnostic(name: string): boolean {
  return /^Waku Debug(?: Computer Use)?[-_.]/.test(name);
}

async function addMatchingChildren(
  directory: string,
  matches: (name: string) => boolean,
): Promise<void> {
  let entries;
  try {
    entries = await readdir(directory, { withFileTypes: true });
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return;
    throw error;
  }

  for (const entry of entries) {
    if (matches(entry.name)) addCandidate(join(directory, entry.name));
  }
}

async function existingTargets(): Promise<Target[]> {
  const targets: Target[] = [];
  for (const path of [...candidatePaths].sort()) {
    try {
      const metadata = await lstat(path);
      targets.push({
        path,
        kind: metadata.isSymbolicLink()
          ? "link"
          : metadata.isDirectory()
            ? "directory"
            : "file",
      });
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
  }
  return targets;
}

// Checkout-local state and build artifacts. Keep the release cache intact.
addCandidate(join(projectRoot, "temp"));
addCandidate(join(projectRoot, ".waku-cache", "computer-use", "debug"));
addCandidate(join(projectRoot, "target", "debug", "Waku Debug.app"));

if (process.env.CARGO_TARGET_DIR) {
  addCandidate(
    join(
      resolve(projectRoot, process.env.CARGO_TARGET_DIR),
      "debug",
      "Waku Debug.app",
    ),
  );
}

// Debug app bundles that may have been copied outside the checkout.
addCandidate(join(userHome, "Applications", "Waku Debug.app"));
addCandidate("/Applications/Waku Debug.app");

// Debug-only app data. The release app uses Waku/sh.waku and is not included.
addCandidate(join(library, "Application Support", "Waku Debug"));
addCandidate(
  join(
    library,
    "Application Support",
    "Waku",
    "Computer Use",
    "Waku Debug Computer Use.app",
  ),
);
addCandidate(join(library, "Caches", "Waku Debug"));
addCandidate(join(library, "Logs", "Waku Debug"));

// codes.waku.dev was Waku Debug's bundle ID before sh.waku.dev.
for (const bundleIdentifier of debugBundleIdentifiers) {
  for (const path of [
    join(library, "Application Support", bundleIdentifier),
    join(library, "Application Scripts", bundleIdentifier),
    join(library, "Caches", bundleIdentifier),
    join(library, "Containers", bundleIdentifier),
    join(library, "Group Containers", bundleIdentifier),
    join(library, "HTTPStorages", bundleIdentifier),
    join(library, "Logs", bundleIdentifier),
    join(library, "Preferences", `${bundleIdentifier}.plist`),
    join(library, "Saved Application State", `${bundleIdentifier}.savedState`),
    join(library, "WebKit", bundleIdentifier),
  ]) {
    addCandidate(path);
  }
}

// macOS may suffix preference, HTTP-storage, and diagnostic files.
for (const bundleIdentifier of debugBundleIdentifiers) {
  await addMatchingChildren(join(library, "Preferences", "ByHost"), (name) =>
    name.startsWith(`${bundleIdentifier}.`),
  );
  await addMatchingChildren(join(library, "HTTPStorages"), (name) =>
    name.startsWith(`${bundleIdentifier}.`),
  );
}
await addMatchingChildren(
  join(library, "Application Support", "CrashReporter"),
  isDebugDiagnostic,
);
await addMatchingChildren(
  join(library, "Logs", "DiagnosticReports"),
  isDebugDiagnostic,
);
await addMatchingChildren(
  join(library, "Logs", "DiagnosticReports", "Retired"),
  isDebugDiagnostic,
);

const targets = await existingTargets();
if (targets.length === 0) {
  console.log("No Waku Debug files or directories found.");
  process.exit(0);
}

console.log(
  "The following Waku Debug paths, including directory contents, will be permanently deleted:\n",
);
for (const target of targets) {
  console.log(`  [${target.kind}] ${target.path}`);
}

const runningProcesses = ["Waku Debug", "Waku Debug Computer Use"].filter(
  (name) =>
    Bun.spawnSync(["/usr/bin/pgrep", "-x", name], {
      stdout: "ignore",
      stderr: "ignore",
    }).exitCode === 0,
);
if (runningProcesses.length > 0) {
  console.warn(
    `\nWarning: ${runningProcesses.join(" and ")} is running. Quit it before confirming, or it may recreate debug data.`,
  );
}

const readline = createInterface({ input: process.stdin, output: process.stdout });
let answer: string;
try {
  answer = (await readline.question("\nDelete these paths? [Y/n] "))
    .trim()
    .toLowerCase();
} finally {
  readline.close();
}

if (answer !== "" && answer !== "y" && answer !== "yes") {
  console.log("Cancelled; nothing was deleted.");
  process.exit(0);
}

const failures: Array<{ path: string; error: unknown }> = [];
for (const target of targets) {
  try {
    await rm(target.path, { recursive: true, force: true });
    console.log(`Deleted ${target.path}`);
  } catch (error) {
    failures.push({ path: target.path, error });
    console.error(`Could not delete ${target.path}:`, error);
  }
}

if (failures.length > 0) {
  console.error(
    `\nDeleted ${targets.length - failures.length} of ${targets.length} paths; ${failures.length} failed.`,
  );
  process.exit(1);
}

console.log(`\nDeleted ${targets.length} paths.`);

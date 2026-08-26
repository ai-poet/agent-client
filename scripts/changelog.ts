// Pull a single version's notes out of a Keep-a-Changelog-style CHANGELOG.md.

/** The version token from a level-2 heading, or null if it isn't one.
 *  Handles `## [0.2.0] - 2026-08-08`, `## 0.2.0`, `## v0.2.0`, etc. */
function headingVersion(line: string): string | null {
  const match = line.match(/^##\s+(.+)$/); // level 2 only — `### …` won't match
  const token = match?.[1]?.trim().split(/\s+/)[0]; // before any ` - date`
  if (!token) return null;
  return token
    .replace(/^\[|\]$/g, "") // strip [ ]
    .replace(/^v/i, ""); // strip a leading v
}

/** Return the notes body for `version` (without its heading), or null if the
 *  changelog has no section for it. */
export function extractReleaseNotes(
  changelog: string,
  version: string,
): string | null {
  const lines = changelog.split("\n");

  let start = -1;
  for (let i = 0; i < lines.length; i++) {
    if (headingVersion(lines[i] ?? "") === version) {
      start = i + 1;
      break;
    }
  }
  if (start === -1) return null;

  let end = lines.length;
  for (let i = start; i < lines.length; i++) {
    if (/^##\s+/.test(lines[i] ?? "")) {
      end = i;
      break;
    }
  }

  const body = lines.slice(start, end).join("\n").trim();
  return body || null;
}

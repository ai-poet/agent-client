export function buildMagicCommitPrompt(paths: readonly string[]): string {
  const selectedPaths = [...paths];
  return [
    "Please create a git commit for the selected files only.",
    "",
    "Selected files JSON:",
    JSON.stringify(selectedPaths, null, 2),
    "",
    "Instructions:",
    "1. Inspect the diff for only the selected files.",
    "2. Summarize the selected changes and choose a concise commit message.",
    "3. Stage only the selected files. Do not stage or commit any unselected files.",
    "4. Run git commit with the generated message.",
    "5. If you cannot commit, explain the blocker briefly.",
  ].join("\n");
}

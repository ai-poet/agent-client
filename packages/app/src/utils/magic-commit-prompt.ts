import type { Sub2APILocale } from "@/i18n/sub2api";

export function buildMagicCommitPrompt(paths: readonly string[], locale: Sub2APILocale): string {
  const selectedPaths = [...paths];
  if (locale === "zh") {
    return [
      "请只为选中的文件创建一个 git commit。",
      "",
      "选中文件 JSON：",
      JSON.stringify(selectedPaths, null, 2),
      "",
      "要求：",
      "1. 只检查这些选中文件的 diff。",
      "2. 总结选中文件的改动，并生成一条简洁的提交信息。",
      "3. 只暂存选中的文件，不要暂存或提交任何未选文件。",
      "4. 使用生成的提交信息执行 git commit。",
      "5. 如果无法提交，请简要说明阻塞原因。",
    ].join("\n");
  }

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

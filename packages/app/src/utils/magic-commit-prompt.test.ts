import { describe, expect, it } from "vitest";
import { buildMagicCommitPrompt } from "./magic-commit-prompt";

describe("buildMagicCommitPrompt", () => {
  it("builds a Chinese prompt for the Chinese UI locale", () => {
    const prompt = buildMagicCommitPrompt(["src/app.ts"], "zh");

    expect(prompt).toContain("请只为选中的文件创建一个 git commit。");
    expect(prompt).toContain('"src/app.ts"');
    expect(prompt).not.toContain("Please create a git commit");
  });

  it("builds an English prompt for the English UI locale", () => {
    const prompt = buildMagicCommitPrompt(["src/app.ts"], "en");

    expect(prompt).toContain("Please create a git commit for the selected files only.");
    expect(prompt).toContain('"src/app.ts"');
    expect(prompt).not.toContain("请只为选中的文件");
  });
});

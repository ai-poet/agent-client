import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { execSync } from "child_process";
import { mkdtempSync, writeFileSync } from "fs";
import { join } from "path";
import { tmpdir } from "os";
import { getCommitGraph } from "./git-graph.js";

function createTempDir(): string {
  return mkdtempSync(join(tmpdir(), "git-graph-test-"));
}

function initRepo(cwd: string): void {
  execSync("git init", { cwd });
  execSync("git config user.email 'test@test.com'", { cwd });
  execSync("git config user.name 'Test User'", { cwd });
  execSync("git branch -m master", { cwd });
}

function commit(cwd: string, message: string): void {
  writeFileSync(join(cwd, "file.txt"), message + "\n", { flag: "a" });
  execSync("git add file.txt", { cwd });
  execSync(`git commit -m ${JSON.stringify(message)}`, { cwd });
}

describe("getCommitGraph", () => {
  let tempDir: string;

  beforeEach(() => {
    tempDir = createTempDir();
  });

  afterEach(() => {
    try {
      execSync(`rm -rf "${tempDir}"`);
    } catch {
      // ignore cleanup errors
    }
  });

  it("returns empty graph for non-git directory", async () => {
    const graph = await getCommitGraph(tempDir);

    expect(graph.commits).toHaveLength(0);
    expect(graph.branches).toHaveLength(0);
    expect(graph.headCommit).toBeNull();
    expect(graph.rootCommits).toHaveLength(0);
  });

  it("returns single commit for a simple repo", async () => {
    initRepo(tempDir);
    commit(tempDir, "Initial commit");

    const graph = await getCommitGraph(tempDir);

    expect(graph.commits).toHaveLength(1);
    expect(graph.commits[0].message).toBe("Initial commit");
    expect(graph.commits[0].author).toBe("Test User");
    expect(graph.commits[0].parents).toHaveLength(0);
    expect(graph.commits[0].isMerge).toBe(false);
    expect(graph.commits[0].branchTips).toContain("master");
    expect(graph.branches).toHaveLength(1);
    expect(graph.branches[0].name).toBe("master");
    expect(graph.branches[0].isCurrent).toBe(true);
    expect(graph.headCommit).toBe(graph.commits[0].fullHash);
    expect(graph.rootCommits).toHaveLength(1);
  });

  it("returns multiple commits in a linear history", async () => {
    initRepo(tempDir);
    commit(tempDir, "First");
    commit(tempDir, "Second");
    commit(tempDir, "Third");

    const graph = await getCommitGraph(tempDir);

    expect(graph.commits).toHaveLength(3);
    expect(graph.commits[0].message).toBe("Third");
    expect(graph.commits[1].message).toBe("Second");
    expect(graph.commits[2].message).toBe("First");
    expect(graph.commits[0].parents).toHaveLength(1);
    expect(graph.commits[2].parents).toHaveLength(0);
    expect(graph.rootCommits).toHaveLength(1);
  });

  it("detects branches and their tip commits", async () => {
    initRepo(tempDir);
    commit(tempDir, "First");
    commit(tempDir, "Second");

    execSync("git checkout -b feature", { cwd: tempDir });
    commit(tempDir, "Feature commit");

    const graph = await getCommitGraph(tempDir);

    expect(graph.branches).toHaveLength(2);
    const branchNames = graph.branches.map((b) => b.name).sort();
    expect(branchNames).toEqual(["feature", "master"]);

    const featureBranch = graph.branches.find((b) => b.name === "feature");
    expect(featureBranch?.isCurrent).toBe(true);
    expect(featureBranch?.tipCommit).toBe(graph.commits[0].fullHash);

    const masterBranch = graph.branches.find((b) => b.name === "master");
    expect(masterBranch?.isCurrent).toBe(false);
    expect(masterBranch?.tipCommit).toBe(graph.commits[1].fullHash);
  });

  it("detects merge commits", async () => {
    initRepo(tempDir);
    commit(tempDir, "First");

    execSync("git checkout -b feature", { cwd: tempDir });
    writeFileSync(join(tempDir, "feature.txt"), "feature\n");
    execSync("git add feature.txt", { cwd: tempDir });
    execSync("git commit -m 'Feature'", { cwd: tempDir });

    execSync("git checkout master", { cwd: tempDir });
    writeFileSync(join(tempDir, "master.txt"), "master\n");
    execSync("git add master.txt", { cwd: tempDir });
    execSync("git commit -m 'Master advance'", { cwd: tempDir });

    execSync("git merge feature --no-ff -m 'Merge feature'", { cwd: tempDir });

    const graph = await getCommitGraph(tempDir);

    const mergeCommit = graph.commits.find((c) => c.message === "Merge feature");
    expect(mergeCommit).toBeDefined();
    expect(mergeCommit?.isMerge).toBe(true);
    expect(mergeCommit?.parents).toHaveLength(2);
  });

  it("respects the limit option", async () => {
    initRepo(tempDir);
    for (let i = 0; i < 10; i++) {
      commit(tempDir, `Commit ${i}`);
    }

    const graph = await getCommitGraph(tempDir, { limit: 5 });

    expect(graph.commits).toHaveLength(5);
  });

  it("detects tags on commits", async () => {
    initRepo(tempDir);
    commit(tempDir, "First");
    execSync("git tag v1.0.0", { cwd: tempDir });
    commit(tempDir, "Second");

    const graph = await getCommitGraph(tempDir);

    const firstCommit = graph.commits.find((c) => c.message === "First");
    expect(firstCommit).toBeDefined();
    expect(firstCommit?.tags).toContain("v1.0.0");

    const secondCommit = graph.commits.find((c) => c.message === "Second");
    expect(secondCommit?.tags).toHaveLength(0);
  });
});

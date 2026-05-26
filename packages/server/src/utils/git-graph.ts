import { runGitCommand } from "./run-git-command.js";

export interface GitGraphCommit {
  hash: string;
  fullHash: string;
  message: string;
  author: string;
  authorEmail: string;
  date: number;
  parents: string[];
  branchTips: string[];
  tags: string[];
  isMerge: boolean;
}

export interface GitGraphBranch {
  name: string;
  isRemote: boolean;
  isCurrent: boolean;
  tipCommit: string;
}

export interface GitGraph {
  commits: GitGraphCommit[];
  branches: GitGraphBranch[];
  headCommit: string | null;
  rootCommits: string[];
}

export interface GetCommitGraphOptions {
  limit?: number;
}

const DEFAULT_COMMIT_LIMIT = 100;
const MAX_COMMIT_LIMIT = 500;
const TAB = "\t";
const NUL = "\x00";

async function isGitRepo(cwd: string): Promise<boolean> {
  try {
    const result = await runGitCommand(["rev-parse", "--git-dir"], { cwd });
    return result.exitCode === 0 && result.stdout.trim().length > 0;
  } catch {
    return false;
  }
}

function parseCommitLogLine(
  line: string,
): Omit<GitGraphCommit, "branchTips" | "tags" | "isMerge"> | null {
  const parts = line.split(NUL);
  if (parts.length < 7) {
    return null;
  }

  const [fullHash, hash, message, author, authorEmail, dateStr, parentsStr] = parts;

  if (!fullHash || !hash) {
    return null;
  }

  const parents = parentsStr?.trim() ? parentsStr.trim().split(" ") : [];

  return {
    fullHash: fullHash.trim(),
    hash: hash.trim(),
    message: message?.trim() ?? "",
    author: author?.trim() ?? "",
    authorEmail: authorEmail?.trim() ?? "",
    date: Number(dateStr) || 0,
    parents,
  };
}

async function getBranchRefs(
  cwd: string,
): Promise<Map<string, Array<{ name: string; isRemote: boolean }>>> {
  const result = await runGitCommand(
    ["for-each-ref", "--format=%(refname:short)\t%(objectname)", "refs/heads", "refs/remotes"],
    { cwd },
  );

  const map = new Map<string, Array<{ name: string; isRemote: boolean }>>();

  for (const line of result.stdout.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed) continue;

    const [refName, commitHash] = trimmed.split(TAB);
    if (!refName || !commitHash) continue;

    const isRemote = refName.startsWith("origin/");
    const displayName = isRemote ? refName.replace("origin/", "") : refName;

    const hash = commitHash.trim();
    const existing = map.get(hash) ?? [];
    existing.push({ name: displayName, isRemote });
    map.set(hash, existing);
  }

  return map;
}

async function getTagRefs(cwd: string): Promise<Map<string, string[]>> {
  const result = await runGitCommand(["show-ref", "--tags", "-d"], {
    cwd,
    acceptExitCodes: [0, 1],
  });

  const map = new Map<string, string[]>();
  const tagToCommit = new Map<string, string>();

  for (const line of result.stdout.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed) continue;

    const [hash, refName] = trimmed.split(" ");
    if (!hash || !refName) continue;

    // refs/tags/v1.0.0^{} is the dereferenced (peeled) commit for annotated tags
    if (refName.endsWith("^{}")) {
      const tagName = refName.replace("refs/tags/", "").replace("^{}", "");
      tagToCommit.set(tagName, hash.trim());
    } else {
      const tagName = refName.replace("refs/tags/", "");
      // Only set if not already set by a peeled ref (annotated tags have both lines)
      if (!tagToCommit.has(tagName)) {
        tagToCommit.set(tagName, hash.trim());
      }
    }
  }

  for (const [tagName, commitHash] of tagToCommit) {
    const existing = map.get(commitHash) ?? [];
    existing.push(tagName);
    map.set(commitHash, existing);
  }

  return map;
}

async function getCurrentBranchName(cwd: string): Promise<string | null> {
  try {
    const result = await runGitCommand(["rev-parse", "--abbrev-ref", "HEAD"], { cwd });
    const name = result.stdout.trim();
    return name === "HEAD" ? null : name;
  } catch {
    return null;
  }
}

async function getHeadCommit(cwd: string): Promise<string | null> {
  try {
    const result = await runGitCommand(["rev-parse", "HEAD"], { cwd });
    return result.stdout.trim() || null;
  } catch {
    return null;
  }
}

async function fetchCommits(
  cwd: string,
  limit: number,
): Promise<Omit<GitGraphCommit, "branchTips" | "tags" | "isMerge">[]> {
  const result = await runGitCommand(
    [
      "log",
      "--all",
      "--format=%H%x00%h%x00%s%x00%an%x00%ae%x00%at%x00%P",
      "--date-order",
      "-n",
      String(limit),
    ],
    { cwd },
  );

  const commits: Omit<GitGraphCommit, "branchTips" | "tags" | "isMerge">[] = [];

  for (const line of result.stdout.split("\n")) {
    const parsed = parseCommitLogLine(line);
    if (parsed) {
      commits.push(parsed);
    }
  }

  return commits;
}

export async function getCommitGraph(
  cwd: string,
  options?: GetCommitGraphOptions,
): Promise<GitGraph> {
  if (!(await isGitRepo(cwd))) {
    return {
      commits: [],
      branches: [],
      headCommit: null,
      rootCommits: [],
    };
  }

  const requestedLimit = options?.limit ?? DEFAULT_COMMIT_LIMIT;
  const limit = Math.max(1, Math.min(MAX_COMMIT_LIMIT, requestedLimit));

  const [commits, branchRefs, tagRefs, currentBranchName, headCommitHash] = await Promise.all([
    fetchCommits(cwd, limit),
    getBranchRefs(cwd),
    getTagRefs(cwd),
    getCurrentBranchName(cwd),
    getHeadCommit(cwd),
  ]);

  const commitMap = new Map<string, GitGraphCommit>();
  const branches: GitGraphBranch[] = [];
  const branchNamesSeen = new Set<string>();

  for (const commit of commits) {
    const branchInfoList = branchRefs.get(commit.fullHash) ?? [];
    const branchTips: string[] = [];

    for (const branchInfo of branchInfoList) {
      if (!branchNamesSeen.has(branchInfo.name)) {
        branchNamesSeen.add(branchInfo.name);
        branches.push({
          name: branchInfo.name,
          isRemote: branchInfo.isRemote,
          isCurrent: branchInfo.name === currentBranchName,
          tipCommit: commit.fullHash,
        });
      }
      branchTips.push(branchInfo.name);
    }

    commitMap.set(commit.fullHash, {
      ...commit,
      branchTips,
      tags: tagRefs.get(commit.fullHash) ?? [],
      isMerge: commit.parents.length > 1,
    });
  }

  const resultCommits = Array.from(commitMap.values());
  const rootCommits = resultCommits.filter((c) => c.parents.length === 0).map((c) => c.fullHash);

  return {
    commits: resultCommits,
    branches,
    headCommit: headCommitHash,
    rootCommits,
  };
}

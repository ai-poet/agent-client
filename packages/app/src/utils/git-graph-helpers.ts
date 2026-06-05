import type { GitGraph, GitGraphCommit } from "@server/shared/messages";

export function formatCommitDate(timestamp: number): string {
  const date = new Date(timestamp * 1000);
  if (Number.isNaN(date.getTime())) {
    return "";
  }
  return date.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

export function normalizeSearch(value: string): string {
  return value.trim().toLocaleLowerCase();
}

export function commitMatchesSearch(commit: GitGraphCommit, query: string): boolean {
  if (!query) {
    return true;
  }
  const haystack = [
    commit.message,
    commit.hash,
    commit.fullHash,
    commit.author,
    commit.authorEmail,
    ...commit.branchTips,
    ...commit.tags,
  ]
    .join(" ")
    .toLocaleLowerCase();
  return haystack.includes(query);
}

export function buildSearchMatches(graph: GitGraph, query: string): Set<string> {
  if (!query) {
    return new Set();
  }
  const result = new Set<string>();
  for (const commit of graph.commits) {
    if (commitMatchesSearch(commit, query)) {
      result.add(commit.fullHash);
    }
  }
  return result;
}

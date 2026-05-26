import { describe, it, expect } from "vitest";
import { layoutGitGraph } from "./git-graph-layout.js";
import type { GitGraph, GitGraphBranch } from "@server/shared/messages";

function makeGraph(overrides?: Partial<GitGraph>): GitGraph {
  return {
    commits: [],
    branches: [],
    headCommit: null,
    rootCommits: [],
    ...overrides,
  };
}

describe("layoutGitGraph", () => {
  it("returns empty layout for empty graph", () => {
    const layout = layoutGitGraph(makeGraph());
    expect(layout.nodes).toHaveLength(0);
    expect(layout.edges).toHaveLength(0);
    expect(layout.width).toBe(48);
    expect(layout.height).toBe(0);
  });

  it("places single commit at column 0", () => {
    const graph = makeGraph({
      commits: [
        {
          hash: "abc",
          fullHash: "abc123",
          message: "Initial",
          author: "A",
          authorEmail: "a@example.com",
          date: 1,
          parents: [],
          branchTips: ["main"],
          tags: [],
          isMerge: false,
        },
      ],
      branches: [{ name: "main", isRemote: false, isCurrent: true, tipCommit: "abc123" }],
      headCommit: "abc123",
    });

    const layout = layoutGitGraph(graph);
    expect(layout.nodes).toHaveLength(1);
    expect(layout.nodes[0].x).toBe(24);
    expect(layout.nodes[0].y).toBe(20);
    expect(layout.edges).toHaveLength(0);
  });

  it("places linear history vertically in column 0", () => {
    const graph = makeGraph({
      commits: [
        {
          hash: "c3",
          fullHash: "c3",
          message: "Third",
          author: "A",
          authorEmail: "a@example.com",
          date: 3,
          parents: ["c2"],
          branchTips: ["main"],
          tags: [],
          isMerge: false,
        },
        {
          hash: "c2",
          fullHash: "c2",
          message: "Second",
          author: "A",
          authorEmail: "a@example.com",
          date: 2,
          parents: ["c1"],
          branchTips: [],
          tags: [],
          isMerge: false,
        },
        {
          hash: "c1",
          fullHash: "c1",
          message: "First",
          author: "A",
          authorEmail: "a@example.com",
          date: 1,
          parents: [],
          branchTips: [],
          tags: [],
          isMerge: false,
        },
      ],
      branches: [{ name: "main", isRemote: false, isCurrent: true, tipCommit: "c3" }],
      headCommit: "c3",
      rootCommits: ["c1"],
    });

    const layout = layoutGitGraph(graph);
    expect(layout.nodes).toHaveLength(3);
    expect(layout.nodes[0].x).toBe(layout.nodes[1].x);
    expect(layout.nodes[1].x).toBe(layout.nodes[2].x);
    expect(layout.edges).toHaveLength(2);
  });

  it("places branch commits in separate columns", () => {
    const graph = makeGraph({
      commits: [
        {
          hash: "f2",
          fullHash: "f2",
          message: "Feature 2",
          author: "A",
          authorEmail: "a@example.com",
          date: 4,
          parents: ["f1"],
          branchTips: ["feature"],
          tags: [],
          isMerge: false,
        },
        {
          hash: "m2",
          fullHash: "m2",
          message: "Main 2",
          author: "A",
          authorEmail: "a@example.com",
          date: 3,
          parents: ["m1"],
          branchTips: ["main"],
          tags: [],
          isMerge: false,
        },
        {
          hash: "f1",
          fullHash: "f1",
          message: "Feature 1",
          author: "A",
          authorEmail: "a@example.com",
          date: 2,
          parents: ["m1"],
          branchTips: [],
          tags: [],
          isMerge: false,
        },
        {
          hash: "m1",
          fullHash: "m1",
          message: "Main 1",
          author: "A",
          authorEmail: "a@example.com",
          date: 1,
          parents: [],
          branchTips: [],
          tags: [],
          isMerge: false,
        },
      ],
      branches: [
        { name: "main", isRemote: false, isCurrent: true, tipCommit: "m2" },
        { name: "feature", isRemote: false, isCurrent: false, tipCommit: "f2" },
      ],
      headCommit: "m2",
      rootCommits: ["m1"],
    });

    const layout = layoutGitGraph(graph);
    expect(layout.nodes).toHaveLength(4);
    // Main branch should be in a different column than feature
    const mainNodes = layout.nodes.filter((n) => n.commit.branchTips.includes("main"));
    const featureNodes = layout.nodes.filter((n) => n.commit.branchTips.includes("feature"));
    expect(mainNodes.length).toBeGreaterThan(0);
    expect(featureNodes.length).toBeGreaterThan(0);
    expect(mainNodes[0].x).not.toBe(featureNodes[0].x);
  });

  it("creates edges for merge commits with two parents", () => {
    const graph = makeGraph({
      commits: [
        {
          hash: "merge",
          fullHash: "merge",
          message: "Merge",
          author: "A",
          authorEmail: "a@example.com",
          date: 3,
          parents: ["main", "feature"],
          branchTips: ["main"],
          tags: [],
          isMerge: true,
        },
        {
          hash: "feature",
          fullHash: "feature",
          message: "Feature",
          author: "A",
          authorEmail: "a@example.com",
          date: 2,
          parents: ["base"],
          branchTips: [],
          tags: [],
          isMerge: false,
        },
        {
          hash: "main",
          fullHash: "main",
          message: "Main",
          author: "A",
          authorEmail: "a@example.com",
          date: 2,
          parents: ["base"],
          branchTips: [],
          tags: [],
          isMerge: false,
        },
        {
          hash: "base",
          fullHash: "base",
          message: "Base",
          author: "A",
          authorEmail: "a@example.com",
          date: 1,
          parents: [],
          branchTips: [],
          tags: [],
          isMerge: false,
        },
      ],
      branches: [{ name: "main", isRemote: false, isCurrent: true, tipCommit: "merge" }],
      headCommit: "merge",
      rootCommits: ["base"],
    });

    const layout = layoutGitGraph(graph);
    expect(layout.nodes).toHaveLength(4);
    expect(layout.edges).toHaveLength(4); // merge->main, merge->feature, main->base, feature->base
  });
});

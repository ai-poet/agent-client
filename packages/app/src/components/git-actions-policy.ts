import type { ReactElement } from "react";

import type { ActionStatus } from "@/components/ui/dropdown-menu";
import type { getAppMessages } from "@/i18n/sub2api";

export type GitActionId =
  | "commit"
  | "pull"
  | "push"
  | "pr"
  | "merge-branch"
  | "merge-from-base"
  | "archive-worktree";

export interface GitAction {
  id: GitActionId;
  label: string;
  pendingLabel: string;
  successLabel: string;
  disabled: boolean;
  status: ActionStatus;
  unavailableMessage?: string;
  icon?: ReactElement;
  handler: () => void;
}

export interface GitActions {
  primary: GitAction | null;
  secondary: GitAction[];
  menu: GitAction[];
}

export type GitActionPolicyText = ReturnType<typeof getAppMessages>["gitDiff"];

interface GitActionRuntimeState {
  disabled: boolean;
  status: ActionStatus;
  icon?: ReactElement;
  handler: () => void;
}

export interface BuildGitActionsInput {
  isGit: boolean;
  githubFeaturesEnabled: boolean;
  hasPullRequest: boolean;
  pullRequestUrl: string | null;
  hasRemote: boolean;
  isPaseoOwnedWorktree: boolean;
  isOnBaseBranch: boolean;
  hasUncommittedChanges: boolean;
  baseRefAvailable: boolean;
  baseRefLabel: string;
  aheadCount: number;
  behindBaseCount: number;
  aheadOfOrigin: number;
  behindOfOrigin: number;
  shouldPromoteArchive: boolean;
  shipDefault: "merge" | "pr";
  text: GitActionPolicyText;
  runtime: Record<GitActionId, GitActionRuntimeState>;
}

const REMOTE_ACTION_IDS: GitActionId[] = ["pull", "push"];
const FEATURE_ACTION_IDS: GitActionId[] = ["merge-from-base", "merge-branch", "pr"];

export function buildGitActions(input: BuildGitActionsInput): GitActions {
  if (!input.isGit) {
    return { primary: null, secondary: [], menu: [] };
  }

  const allActions = new Map<GitActionId, GitAction>();

  allActions.set("commit", {
    id: "commit",
    label: input.text.commitAction,
    pendingLabel: input.text.committing,
    successLabel: input.text.committedToast,
    disabled: input.runtime.commit.disabled,
    status: input.runtime.commit.status,
    icon: input.runtime.commit.icon,
    handler: input.runtime.commit.handler,
  });

  allActions.set("pull", {
    id: "pull",
    label: input.text.pullAction,
    pendingLabel: input.text.pulling,
    successLabel: input.text.pulledToast,
    disabled: input.runtime.pull.disabled,
    status: input.runtime.pull.status,
    unavailableMessage: input.runtime.pull.disabled ? undefined : getPullUnavailableMessage(input),
    icon: input.runtime.pull.icon,
    handler: input.runtime.pull.handler,
  });

  allActions.set("push", {
    id: "push",
    label: input.text.pushAction,
    pendingLabel: input.text.pushing,
    successLabel: input.text.pushedToast,
    disabled: input.runtime.push.disabled,
    status: input.runtime.push.status,
    unavailableMessage: input.runtime.push.disabled ? undefined : getPushUnavailableMessage(input),
    icon: input.runtime.push.icon,
    handler: input.runtime.push.handler,
  });

  allActions.set("pr", buildPrAction(input));

  allActions.set("merge-branch", {
    id: "merge-branch",
    label: input.text.mergeInto(input.baseRefLabel),
    pendingLabel: input.text.merging,
    successLabel: input.text.mergedToast,
    disabled: input.runtime["merge-branch"].disabled,
    status: input.runtime["merge-branch"].status,
    unavailableMessage: input.runtime["merge-branch"].disabled
      ? undefined
      : getMergeBranchUnavailableMessage(input),
    icon: input.runtime["merge-branch"].icon,
    handler: input.runtime["merge-branch"].handler,
  });

  allActions.set("merge-from-base", {
    id: "merge-from-base",
    label: input.text.updateFrom(input.baseRefLabel),
    pendingLabel: input.text.updating,
    successLabel: input.text.updatedToast,
    disabled: input.runtime["merge-from-base"].disabled,
    status: input.runtime["merge-from-base"].status,
    unavailableMessage: input.runtime["merge-from-base"].disabled
      ? undefined
      : getMergeFromBaseUnavailableMessage(input),
    icon: input.runtime["merge-from-base"].icon,
    handler: input.runtime["merge-from-base"].handler,
  });

  allActions.set("archive-worktree", {
    id: "archive-worktree",
    label: input.text.archiveWorktree,
    pendingLabel: input.text.archiving,
    successLabel: input.text.archived,
    disabled: input.runtime["archive-worktree"].disabled,
    status: input.runtime["archive-worktree"].status,
    unavailableMessage:
      input.runtime["archive-worktree"].disabled || input.isPaseoOwnedWorktree
        ? undefined
        : input.text.archiveUnavailable,
    icon: input.runtime["archive-worktree"].icon,
    handler: input.runtime["archive-worktree"].handler,
  });

  const primaryActionId = getPrimaryActionId(input);
  const primary = primaryActionId ? (allActions.get(primaryActionId) ?? null) : null;

  const secondaryIds = [...REMOTE_ACTION_IDS];
  if (!input.isOnBaseBranch) {
    secondaryIds.push(...FEATURE_ACTION_IDS);
  }
  if (input.isPaseoOwnedWorktree) {
    secondaryIds.push("archive-worktree");
  }

  return {
    primary,
    secondary: secondaryIds.map((id) => allActions.get(id)!),
    menu: [],
  };
}

function getPrimaryActionId(input: BuildGitActionsInput): GitActionId | null {
  if (input.shouldPromoteArchive && input.isPaseoOwnedWorktree) {
    return "archive-worktree";
  }
  if (input.hasUncommittedChanges) {
    return "commit";
  }
  if (canPull(input)) {
    return "pull";
  }
  if (canPush(input)) {
    return "push";
  }
  if (!input.isOnBaseBranch && canMergeFromBase(input)) {
    return "merge-from-base";
  }
  if (input.githubFeaturesEnabled && input.hasPullRequest && input.pullRequestUrl) {
    return "pr";
  }
  if (!input.isOnBaseBranch && input.aheadCount > 0) {
    return input.shipDefault === "merge" ? "merge-branch" : "pr";
  }
  return null;
}

function buildPrAction(input: BuildGitActionsInput): GitAction {
  if (input.hasPullRequest && input.pullRequestUrl) {
    return {
      id: "pr",
      label: input.text.viewPr,
      pendingLabel: input.text.viewPr,
      successLabel: input.text.viewPr,
      disabled: input.runtime.pr.disabled,
      status: input.runtime.pr.status,
      unavailableMessage:
        input.runtime.pr.disabled || input.githubFeaturesEnabled
          ? undefined
          : input.text.viewPrGithubUnavailable,
      icon: input.runtime.pr.icon,
      handler: input.runtime.pr.handler,
    };
  }

  return {
    id: "pr",
    label: input.text.createPr,
    pendingLabel: input.text.creatingPr,
    successLabel: input.text.prCreatedAction,
    disabled: input.runtime.pr.disabled,
    status: input.runtime.pr.status,
    unavailableMessage: input.runtime.pr.disabled
      ? undefined
      : getCreatePrUnavailableMessage(input),
    icon: input.runtime.pr.icon,
    handler: input.runtime.pr.handler,
  };
}

function canPull(input: BuildGitActionsInput): boolean {
  return input.hasRemote && !input.hasUncommittedChanges && input.behindOfOrigin > 0;
}

function canPush(input: BuildGitActionsInput): boolean {
  return input.hasRemote && input.aheadOfOrigin > 0 && input.behindOfOrigin === 0;
}

function canMergeBranch(input: BuildGitActionsInput): boolean {
  return (
    !input.isOnBaseBranch &&
    input.baseRefAvailable &&
    !input.hasUncommittedChanges &&
    input.aheadCount > 0
  );
}

function canMergeFromBase(input: BuildGitActionsInput): boolean {
  return (
    !input.isOnBaseBranch &&
    input.baseRefAvailable &&
    !input.hasUncommittedChanges &&
    input.behindBaseCount > 0
  );
}

function getPullUnavailableMessage(input: BuildGitActionsInput): string | undefined {
  if (!input.hasRemote) {
    return input.text.pullNoRemote;
  }
  if (input.hasUncommittedChanges) {
    return input.text.pullLocalChanges;
  }
  if (input.behindOfOrigin === 0) {
    return input.text.pullUpToDate;
  }
  return undefined;
}

function getPushUnavailableMessage(input: BuildGitActionsInput): string | undefined {
  if (!input.hasRemote) {
    return input.text.pushNoRemote;
  }
  if (input.behindOfOrigin > 0) {
    return input.text.pushBehind;
  }
  if (input.aheadOfOrigin === 0) {
    return input.text.pushNothingNew;
  }
  return undefined;
}

function getCreatePrUnavailableMessage(input: BuildGitActionsInput): string | undefined {
  if (!input.githubFeaturesEnabled) {
    return input.text.createPrGithubUnavailable;
  }
  if (input.aheadCount === 0) {
    return input.text.createPrNoCommits;
  }
  return undefined;
}

function getMergeBranchUnavailableMessage(input: BuildGitActionsInput): string | undefined {
  if (!input.baseRefAvailable) {
    return input.text.mergeNoBase;
  }
  if (input.hasUncommittedChanges) {
    return input.text.mergeLocalChanges;
  }
  if (input.aheadCount === 0) {
    return input.text.mergeNothingNew;
  }
  return undefined;
}

function getMergeFromBaseUnavailableMessage(input: BuildGitActionsInput): string | undefined {
  if (!input.baseRefAvailable) {
    return input.text.updateNoBase;
  }
  if (input.hasUncommittedChanges) {
    return input.text.updateLocalChanges;
  }
  if (input.behindBaseCount === 0) {
    return input.text.updateAlreadyUpToDate(input.baseRefLabel);
  }
  return undefined;
}

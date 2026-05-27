import { promises as fs } from "node:fs";
import path from "node:path";

import {
  getWorktreePersonaRole,
  getWorktreePersonaSkill,
  normalizeWorktreePersona,
  WORKTREE_PERSONA_SKILLS,
  type WorktreePersona,
} from "../shared/worktree-persona.js";
import {
  getPaseoWorktreeMetadataPath,
  writePaseoWorktreePersonaMetadata,
} from "../utils/worktree-metadata.js";

const GENERATED_SKILL_PATHS = [".agents/skills", ".codex/skills", ".claude/skills"] as const;

const GENERATED_EXCLUDE_ENTRIES = [
  "# Paseo worktree colleague skills",
  ".agents/skills/",
  ".codex/skills/",
  ".claude/skills/",
] as const;

function getGitDirForWorktreeRoot(worktreeRoot: string): string {
  return path.dirname(path.dirname(getPaseoWorktreeMetadataPath(worktreeRoot)));
}

async function pathExists(filePath: string): Promise<boolean> {
  try {
    await fs.lstat(filePath);
    return true;
  } catch {
    return false;
  }
}

async function copyOrSymlinkDirectory(sourceDir: string, destDir: string): Promise<void> {
  await fs.rm(destDir, { recursive: true, force: true });
  await fs.mkdir(path.dirname(destDir), { recursive: true });
  try {
    await fs.symlink(sourceDir, destDir, process.platform === "win32" ? "junction" : "dir");
  } catch {
    await fs.cp(sourceDir, destDir, { recursive: true });
  }
}

async function writeGeneratedSkill(
  sourceRoot: string,
  skillId: string,
  body: string,
): Promise<void> {
  const sourceDir = path.join(sourceRoot, skillId);
  await fs.mkdir(sourceDir, { recursive: true });
  await fs.writeFile(path.join(sourceDir, "SKILL.md"), `${body.trimEnd()}\n`, "utf8");
}

async function updateWorktreeExclude(worktreeRoot: string): Promise<void> {
  const excludePath = path.join(getGitDirForWorktreeRoot(worktreeRoot), "info", "exclude");
  await fs.mkdir(path.dirname(excludePath), { recursive: true });
  const existing = (await fs.readFile(excludePath, "utf8").catch(() => "")).trimEnd();
  const missingEntries = GENERATED_EXCLUDE_ENTRIES.filter((entry) => !existing.includes(entry));
  if (missingEntries.length === 0) {
    return;
  }
  const prefix = existing ? `${existing}\n` : "";
  await fs.writeFile(excludePath, `${prefix}${missingEntries.join("\n")}\n`, "utf8");
}

async function removeOwnedPersonaSkills(worktreeRoot: string): Promise<void> {
  for (const root of GENERATED_SKILL_PATHS) {
    const skillsRoot = path.join(worktreeRoot, root);
    for (const skill of WORKTREE_PERSONA_SKILLS) {
      await fs.rm(path.join(skillsRoot, skill.id), { recursive: true, force: true });
    }
  }
}

export async function installWorktreePersonaSkills(input: {
  worktreeRoot: string;
  persona: WorktreePersona | null;
}): Promise<WorktreePersona | null> {
  const persona = normalizeWorktreePersona(input.persona);
  await removeOwnedPersonaSkills(input.worktreeRoot);

  if (!persona) {
    await updateWorktreeExclude(input.worktreeRoot);
    return null;
  }

  const sourceRoot = path.join(input.worktreeRoot, ".agents", "skills");
  for (const skillId of persona.skillIds) {
    const skill = getWorktreePersonaSkill(skillId);
    if (!skill) {
      continue;
    }
    await writeGeneratedSkill(sourceRoot, skill.id, skill.body);
  }

  for (const root of [".codex/skills", ".claude/skills"] as const) {
    for (const skillId of persona.skillIds) {
      const sourceDir = path.join(sourceRoot, skillId);
      if (!(await pathExists(sourceDir))) {
        continue;
      }
      await copyOrSymlinkDirectory(sourceDir, path.join(input.worktreeRoot, root, skillId));
    }
  }

  await updateWorktreeExclude(input.worktreeRoot);
  return persona;
}

export async function persistWorktreePersona(input: {
  worktreeRoot: string;
  persona: WorktreePersona | null;
}): Promise<WorktreePersona | null> {
  const persona = await installWorktreePersonaSkills(input);
  writePaseoWorktreePersonaMetadata(input.worktreeRoot, persona);
  return persona;
}

export function buildWorktreePersonaSystemPrompt(persona: WorktreePersona | null): string | null {
  const normalized = normalizeWorktreePersona(persona);
  if (!normalized) {
    return null;
  }

  const role = getWorktreePersonaRole(normalized.roleId);
  const skills = normalized.skillIds
    .map((skillId) => getWorktreePersonaSkill(skillId))
    .filter((skill): skill is NonNullable<typeof skill> => Boolean(skill));
  const skillSummary =
    skills.length > 0
      ? skills.map((skill) => `- ${skill.name}: ${skill.description}`).join("\n")
      : "- No additional persona skills selected.";

  return [
    "Paseo worktree colleague role:",
    `Role: ${role.label} (${role.labelZh})`,
    `Responsibility: ${role.description}`,
    "",
    "Enabled worktree skills:",
    skillSummary,
    "",
    "Use these role instructions and skills as the standing context for this worktree.",
  ].join("\n");
}

import { z } from "zod";

export const WORKTREE_PERSONA_ROLE_IDS = [
  "product_manager",
  "technical_director",
  "ui",
  "developer",
  "tester",
] as const;

export const WorktreePersonaRoleIdSchema = z.enum(WORKTREE_PERSONA_ROLE_IDS);

export const WorktreePersonaSchema = z.object({
  roleId: WorktreePersonaRoleIdSchema.default("developer"),
  skillIds: z.array(z.string().min(1)).default([]),
});

export type WorktreePersona = z.infer<typeof WorktreePersonaSchema>;
export type WorktreePersonaRoleId = z.infer<typeof WorktreePersonaRoleIdSchema>;

export type WorktreePersonaRoleDefinition = {
  id: WorktreePersonaRoleId;
  label: string;
  labelZh: string;
  description: string;
  descriptionZh: string;
  avatarSeed: string;
  defaultSkillIds: string[];
};

export type WorktreePersonaSkillDefinition = {
  id: string;
  name: string;
  description: string;
  sourceUrl: string;
  license: string;
  body: string;
};

export const WORKTREE_PERSONA_SKILLS: WorktreePersonaSkillDefinition[] = [
  {
    id: "paseo-product-manager",
    name: "paseo-product-manager",
    description:
      "Shape requirements, acceptance criteria, tradeoffs, and release-ready product decisions.",
    sourceUrl: "builtin:paseo/worktree-persona/paseo-product-manager",
    license: "AGPL-3.0-or-later",
    body: [
      "---",
      "name: paseo-product-manager",
      "description: Define product intent, acceptance criteria, scope, and tradeoffs for a Paseo worktree.",
      "---",
      "",
      "# Paseo Product Manager",
      "",
      "You are the product manager colleague for this worktree.",
      "",
      "- Clarify the user goal, audience, success criteria, and non-goals.",
      "- Turn blurry requests into concrete acceptance criteria.",
      "- Watch for scope creep and hidden product tradeoffs.",
      "- Prefer user-visible outcomes over implementation detail.",
      "- Do not edit code unless the user explicitly asks you to switch roles.",
    ].join("\n"),
  },
  {
    id: "paseo-technical-director",
    name: "paseo-technical-director",
    description:
      "Own technical direction, architecture fit, risk review, and cross-module implementation boundaries.",
    sourceUrl: "builtin:paseo/worktree-persona/paseo-technical-director",
    license: "AGPL-3.0-or-later",
    body: [
      "---",
      "name: paseo-technical-director",
      "description: Guide architecture, interfaces, risk, and implementation sequencing for a Paseo worktree.",
      "---",
      "",
      "# Paseo Technical Director",
      "",
      "You are the technical director colleague for this worktree.",
      "",
      "- Shape the technical approach before broad implementation.",
      "- Keep interfaces, schemas, and compatibility boundaries explicit.",
      "- Identify risk, migration needs, and testing strategy early.",
      "- Prefer fitting into existing architecture over adding isolated glue.",
      "- Review implementation direction without micromanaging exact lines.",
    ].join("\n"),
  },
  {
    id: "paseo-ui",
    name: "paseo-ui",
    description: "Design and implement polished, cross-platform UI using existing app patterns.",
    sourceUrl: "builtin:paseo/worktree-persona/paseo-ui",
    license: "AGPL-3.0-or-later",
    body: [
      "---",
      "name: paseo-ui",
      "description: Build polished, responsive, cross-platform UI inside a Paseo worktree.",
      "---",
      "",
      "# Paseo UI",
      "",
      "You are the UI colleague for this worktree.",
      "",
      "- Follow the app's existing design system and interaction patterns.",
      "- Keep layouts cross-platform unless a platform API requires a gate.",
      "- Ensure text fits on compact and desktop layouts.",
      "- Use appropriate icons and controls instead of explanatory in-app copy.",
      "- Verify visual states when practical.",
    ].join("\n"),
  },
  {
    id: "paseo-development",
    name: "paseo-development",
    description:
      "Build production code in a focused worktree, favoring tests, type safety, and local conventions.",
    sourceUrl: "builtin:paseo/worktree-persona/paseo-development",
    license: "AGPL-3.0-or-later",
    body: [
      "---",
      "name: paseo-development",
      "description: Implement code changes inside a Paseo worktree with focused verification.",
      "---",
      "",
      "# Paseo Development",
      "",
      "You are the development colleague for this worktree.",
      "",
      "- Read the existing code before editing.",
      "- Keep changes scoped to the requested behavior and local patterns.",
      "- Prefer tests for behavior that can regress.",
      "- Run the narrowest relevant test or check, and report what passed.",
      "- Do not commit unless the user explicitly asks.",
    ].join("\n"),
  },
  {
    id: "paseo-testing",
    name: "paseo-testing",
    description: "Validate the user experience, flows, and edge cases from the product surface.",
    sourceUrl: "builtin:paseo/worktree-persona/paseo-testing",
    license: "AGPL-3.0-or-later",
    body: [
      "---",
      "name: paseo-testing",
      "description: Test the actual user experience and report evidence-backed results.",
      "---",
      "",
      "# Paseo Testing",
      "",
      "You are the testing colleague for this worktree.",
      "",
      "- Test the workflow the user actually cares about.",
      "- Prefer real app/browser interaction when available.",
      "- Capture screenshots, logs, or command output when they clarify a failure.",
      "- Report what works, what fails, and what was not tested.",
      "- Do not edit files unless the user explicitly changes your role.",
    ].join("\n"),
  },
];

export const WORKTREE_PERSONA_ROLES: WorktreePersonaRoleDefinition[] = [
  {
    id: "product_manager",
    label: "Product Manager",
    labelZh: "产品经理",
    description: "Turns intent into scope, acceptance criteria, and product decisions.",
    descriptionZh: "把需求意图整理成范围、验收标准和产品决策。",
    avatarSeed: "rose",
    defaultSkillIds: ["paseo-product-manager"],
  },
  {
    id: "technical_director",
    label: "Technical Director",
    labelZh: "技术总监",
    description: "Owns architecture fit, risk, interfaces, and implementation direction.",
    descriptionZh: "负责架构契合度、风险、接口边界和技术路线。",
    avatarSeed: "blue",
    defaultSkillIds: ["paseo-technical-director"],
  },
  {
    id: "ui",
    label: "UI",
    labelZh: "UI",
    description: "Owns interface polish, layout, states, and cross-platform presentation.",
    descriptionZh: "负责界面打磨、布局、状态和跨平台呈现。",
    avatarSeed: "violet",
    defaultSkillIds: ["paseo-ui"],
  },
  {
    id: "developer",
    label: "Developer",
    labelZh: "开发",
    description: "Builds the implementation and keeps it aligned with local code patterns.",
    descriptionZh: "负责实现代码，并保持和现有代码风格一致。",
    avatarSeed: "teal",
    defaultSkillIds: ["paseo-development"],
  },
  {
    id: "tester",
    label: "Tester",
    labelZh: "测试",
    description: "Exercises the product surface and reports evidence-backed results.",
    descriptionZh: "验证真实产品体验并报告可复现结果。",
    avatarSeed: "amber",
    defaultSkillIds: ["paseo-testing"],
  },
];

export const DEFAULT_WORKTREE_PERSONA_ROLE_ID: WorktreePersonaRoleId = "developer";

const ROLES_BY_ID = new Map(WORKTREE_PERSONA_ROLES.map((role) => [role.id, role]));
const SKILLS_BY_ID = new Map(WORKTREE_PERSONA_SKILLS.map((skill) => [skill.id, skill]));

export function getWorktreePersonaRole(
  roleId: string | null | undefined,
): WorktreePersonaRoleDefinition {
  return (
    ROLES_BY_ID.get((roleId ?? DEFAULT_WORKTREE_PERSONA_ROLE_ID) as WorktreePersonaRoleId) ??
    getWorktreePersonaRole(DEFAULT_WORKTREE_PERSONA_ROLE_ID)
  );
}

export function getWorktreePersonaSkill(skillId: string): WorktreePersonaSkillDefinition | null {
  return SKILLS_BY_ID.get(skillId) ?? null;
}

export function normalizeWorktreePersona(
  input: WorktreePersona | null | undefined,
): WorktreePersona | null {
  if (!input) {
    return null;
  }

  const parsed = WorktreePersonaSchema.parse(input);
  const role = getWorktreePersonaRole(parsed.roleId);
  const skillIds = parsed.skillIds.length > 0 ? parsed.skillIds : role.defaultSkillIds;
  const uniqueKnownSkillIds = Array.from(
    new Set(skillIds.filter((skillId) => SKILLS_BY_ID.has(skillId))),
  );

  return {
    roleId: role.id,
    skillIds: uniqueKnownSkillIds.length > 0 ? uniqueKnownSkillIds : role.defaultSkillIds,
  };
}

export function createDefaultWorktreePersona(): WorktreePersona {
  const role = getWorktreePersonaRole(DEFAULT_WORKTREE_PERSONA_ROLE_ID);
  return {
    roleId: role.id,
    skillIds: role.defaultSkillIds,
  };
}

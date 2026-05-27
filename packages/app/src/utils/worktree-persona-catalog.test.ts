import { describe, expect, it } from "vitest";
import {
  createDefaultWorktreePersona,
  DEFAULT_WORKTREE_PERSONA_ROLE_ID,
  getWorktreePersonaRole,
  WORKTREE_PERSONA_ROLES,
} from "@server/shared/worktree-persona";

describe("worktree persona catalog", () => {
  it("keeps the v1 colleague roles and default skills stable", () => {
    expect(DEFAULT_WORKTREE_PERSONA_ROLE_ID).toBe("developer");
    expect(createDefaultWorktreePersona()).toEqual({
      roleId: "developer",
      skillIds: ["paseo-development"],
    });
    expect(WORKTREE_PERSONA_ROLES.map((role) => role.id)).toEqual([
      "product_manager",
      "technical_director",
      "ui",
      "developer",
      "tester",
    ]);
    expect(getWorktreePersonaRole("product_manager").defaultSkillIds).toEqual([
      "paseo-product-manager",
    ]);
  });
});

# Notice

This skill was copied from the TokenRouter project.

- Source repository: https://github.com/TokenFlux/TokenRouter.git
- Source commit: 329cc3a
- Source path: `.agents/skills/project-doc/`
- License: LGPL-3.0.
- Installed here via the sub2api server repository (`sub2api/.agents/skills/project-doc/`).

## Local modifications (2026-10-31)

The upstream skill installs a hard gate that stops all repository work when the
current agent session cannot invoke the skill. This repository is worked on with
several agent tools that do not all load skills, so the gate was softened to
"read the file and follow it" semantics:

- `SKILL.md`, section "Integration requirements": the gate now requires agents
  to use `project-doc` before repository work; a session that cannot invoke the
  skill reads `SKILL.md` directly and follows it instead of stopping.
- `references/bootstrap_workflow.md`, section "Install the skill gate": the
  gate template no longer contains the fixed refusal message; it instructs
  agents to read `<skill_dir>/SKILL.md` when the skill cannot be invoked, adds a
  fourth requirement to update affected documents in the same change, and the
  surrounding bullets were reworded to match the non-blocking semantics.

All other files are unmodified copies of the upstream skill.

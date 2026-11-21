---
name: python
description: Run Python scripts with Sailry's bundled interpreter and a session-local environment when additional dependencies are needed.
metadata:
  display-name: "Python"
  display-name.en: "Python"
  display-name.zh-CN: "Python"
  display-name.zh-TW: "Python"
  display-name.ja: "Python"
  display-name.ko: "Python"
  display-name.fr: "Python"
  display-name.de: "Python"
  display-name.es: "Python"
  display-name.pt-BR: "Python"
  display-name.ru: "Python"
---

# Python

Run the managed interpreter as `"$SAILRY_PYTHON"`; use the absolute `directory` returned by `load_skill` for resources. The most recently loaded skill in the current turn selects the command environment; reload the required skill if another environment was loaded later. Sailry bundles Python 3.12.13. Commands also receive `SAILRY_PYTHON`, `SAILRY_SESSION_DIR`, and a shared pip cache. Quote all paths. Loading a skill grants no command or network permission; use the existing command tool and its normal approval flow.

If `load_skill` returns `python: null`, report that the bundled Python runtime is unavailable. Do not substitute system Python or claim that a Python task ran successfully.

Save task scripts and temporary files in `SAILRY_SESSION_DIR`, or save requested project changes in the current worktree. Run scripts with the returned interpreter. Do not assume a system interpreter, activate a shell environment, replace PATH, or install into Sailry's application or shared plugin environments.

For additional task dependencies, create a virtual environment under `SAILRY_SESSION_DIR` with `"$SAILRY_PYTHON" -m venv "$SAILRY_SESSION_DIR/venv"`. Run that environment's Python explicitly. Install only the packages needed for the task with `python -m pip install --only-binary=:all:` under normal network and command permissions. Respect the configured PyPI index; do not substitute a model download mirror. If the required wheel or permission is unavailable, report the failure and keep the draft.

Standalone third-party skills use the bundled interpreter without automatic dependency discovery. Read their requirements, use a session environment when needed, and inspect installed library APIs before calling them. Do not execute package setup scripts or interpret a dependency file as shell commands.

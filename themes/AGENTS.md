# Sailry theme repository rules

- This repository owns official theme manifests, artwork, package guides,
  provenance, the generated catalog and their static checks. The application
  repository owns the reusable theme loader, importer, renderer and native
  interaction tests. Do not copy product implementations or native tests here.
- Read `README.md` before editing. Start with `git status --short` and
  `git log --oneline -5`; preserve unrelated changes and artwork/license notices.
- New official themes start at `package_version: "0.1.0"`. Increase the semantic
  version when package content changes; ordinary updates increment the patch.
  Only strictly newer SemVer precedence offers an update. Same-version content
  changes, build metadata alone and older versions do not offer updates. Resource
  digests verify integrity and immutable Git commits capture resource provenance;
  neither is an update detector. The current manifest and catalog formats use
  numeric `1`; format changes belong to the application contract and its consumers.
- Keep each theme in its ID-named directory. Resources are local raster files
  under `assets/`; do not add remote images, executable code or application UI.
- Edit `theme.json`, then run `python3 scripts/catalog.py`. Never hand-edit
  `catalog.json`. Run its `--check` mode and the focused fixture tests before
  committing package changes. Documentation-only edits need content and link
  review. The index checks do not replace Sailry's strict import validator.
- Source, comments, diagnostics and repository guides use English. Preserve
  intentional user-content and Unicode fixtures without artificial escaping.
- Commit cohesive changes locally. Push only when authorized. The parent
  `sailry-harness` checkout uses `themes/` as a Git submodule; its Gitlink pins
  one reviewed commit. Select a working branch before changing a detached
  checkout, commit the theme repository separately, and publish its referenced
  commit before publishing the parent Gitlink. A parent commit does not commit
  or push changes inside this repository.
- Keep caches, generated exports, credentials and local environment files out
  of Git. Do not introduce screenshot tests or native build jobs here.

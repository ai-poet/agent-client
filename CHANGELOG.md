# Changelog

## 0.1.80 - 2026-06-05

### Added
- Added Opus 4.8 model support.
- CheapRouter branded desktop packaging, app identity, icons, and managed cloud endpoint configuration.
- Skills library with local/bundled skill browsing, marketplace browsing, and local skill CRUD.
- Zip package import for skills, including multi-file packages with `SKILL.md`, scripts, references, and assets.
- Workspace commit graph view with graph navigation, details, toolbar actions, and localized labels.
- Simple experience mode, colleague workspace route, browser preview panel, richer markdown rendering, and desktop AI context hub surfaces.

### Improved
- Skills editing now opens from each skill card in a focused modal instead of a persistent side editor.
- Marketplace skills open their SkillsMP page instead of attempting an in-app install flow.
- Session and workspace hydration paths now prefetch more efficiently and tolerate invalid workspace descriptors.
- Provider and mode selection flows include clearer localized labels, BYOK provider import support, and better desktop CLI/runtime diagnostics.
- Working indicators, status text, sidebar navigation, and workspace layout state were refined for desktop and mobile.

### Fixed
- Skill library pagination, initial workspace selection, local skill query limits, and readonly/editable skill state handling.
- Theme-sensitive skill actions now avoid unreadable fixed black button styling.
- Several desktop opener, auto-updater, provider switch, worktree, git diff, and workspace tab edge cases.
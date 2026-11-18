---
name: powerpoint
metadata:
  display-name.en: "PowerPoint"
  display-name.zh-CN: "PowerPoint"
  display-name.zh-TW: "PowerPoint"
  display-name.ja: "PowerPoint"
  display-name.ko: "PowerPoint"
  display-name.fr: "PowerPoint"
  display-name.de: "PowerPoint"
  display-name.es: "PowerPoint"
  display-name.pt-BR: "PowerPoint"
  display-name.ru: "PowerPoint"
  display-name: PowerPoint
description: Create and edit PowerPoint presentations (.pptx), themes, slide layouts, typography, speaker notes, tables, shapes and editable charts.
---

# Powerpoint

## Execution

Sailry bundles Python 3.12.13. `load_skill` prepares this plugin's hash-locked, wheel-only environment and returns its absolute `python` and `directory`. Run scripts with `"$SAILRY_PYTHON"` and absolute resource paths. Commands use the environment of the most recently loaded skill in this turn; reload the required skill before running if another environment was loaded later. Do not copy the returned interpreter path. The shared environment and plugin resources are read-only during commands; save task scripts and temporary files in `SAILRY_SESSION_DIR`, and write requested deliverables to the worktree. Use the existing command tool and its normal permissions; skill loading grants no command permission. If preparation or execution fails, report the failure and keep the draft. Do not install globally or modify Sailry's application/shared environments. For extra task dependencies, use the Commands Python skill's session-local environment. Inspect installed library APIs with `help()` or `inspect` when needed.

Native `read_office` provides structured inspection for DOCX, XLSX, PPTX and PDF. Author and edit with the script libraries. Scripts use the existing command tool's permissions and working directory. If command execution is disabled or denied, report that constraint without bypassing it.

For existing files, inspect the original and preserve unrelated content. For focused script edits, record the source hash, load the existing file, save to a sibling temporary file, reopen and validate it, recheck the source hash immediately before replacement, then use `os.replace`. Preserve the original and report a conflict if its hash changed. Do not rebuild an existing file from extracted plain text. A library may drop unsupported package parts: inspect those parts and use a focused ZIP/XML patch when necessary. Keep relationships, namespaces and content types consistent.

Before any save, create the destination directory with `Path(destination).parent.mkdir(parents=True, exist_ok=True)`. This applies to documents, images and temporary output files. Verify saved content and the requested changes. Sailry previews Office files directly in the side panel; this does not establish print-layout fidelity. When the task requires rendered layout checks, use an available renderer and inspect representative pages with the image viewing tool. Keep editable source files. Return deliverables as ordinary standalone Markdown links, for example `[Results](output/results.pptx)`. Sailry provides cards and side-panel previews; do not fabricate output paths or JSON cards.

## Presentation authoring

Use `python-pptx` (`from pptx import Presentation`) for editable text, shapes, images, tables, charts, slide dimensions, masters/layouts and speaker notes. Start from the supplied deck/template for edits. Keep slide bounds and consistent spacing; preserve unrelated elements. Use XML patches only for features missing from the high-level API, with package validation.

```python
from pptx import Presentation
from pptx.util import Inches, Pt
from pptx.chart.data import CategoryChartData
from pptx.enum.chart import XL_CHART_TYPE
prs = Presentation()
prs.slide_width, prs.slide_height = Inches(13.333), Inches(7.5)
slide = prs.slides.add_slide(prs.slide_layouts[6])
text = slide.shapes.add_textbox(Inches(0.6), Inches(0.4), Inches(12), Inches(0.8))
run = text.text_frame.paragraphs[0].add_run()
run.text, run.font.size = 'Quarterly results', Pt(32)
data = CategoryChartData()
data.categories = ['Q1', 'Q2', 'Q3']
data.add_series('Revenue', [100, 140, 180])
slide.shapes.add_chart(XL_CHART_TYPE.COLUMN_CLUSTERED, Inches(1), Inches(1.6), Inches(10), Inches(4.5), data)
slide.notes_slide.notes_text_frame.text = 'Explain the growth trend.'
prs.save('results.pptx')
```

Keep requested charts editable, not screenshots. Inspect source text runs before replacing them to avoid losing formatting. Check every slide's shapes, content, notes and chart series after saving. Inspect overlaps and clipping with an available renderer when the task requires layout verification; the side-panel preview does not establish animation or video playback support.

## Package resources

- `templates/create.py` is an editable starter script, not a restricted authoring API. Read it with `read_skill_resource`, copy it into the worktree, then adapt its full library calls to the user's document. Run it with the selected project interpreter and an output path. It creates missing parent directories and refuses to replace an existing file.
- `scripts/inspect_file.py` prints the saved file's content for verification. Run `"$SAILRY_PYTHON" "<skill-directory>/scripts/inspect_file.py" "<file>"` using the absolute directory returned by `load_skill`. Quote paths and keep package resources unchanged. Structural inspection alone does not prove layout fidelity.

The plugin contains instructions, scripts and templates, not an interpreter. `read_office` is supplied by Sailry's Files capability; `run_command` uses Commands on the execution Node. If a required capability is unavailable, explain what must be enabled. Installing or enabling this skill does not grant execution permission.

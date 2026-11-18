---
name: pdf
metadata:
  display-name.en: "PDF"
  display-name.zh-CN: "PDF"
  display-name.zh-TW: "PDF"
  display-name.ja: "PDF"
  display-name.ko: "PDF"
  display-name.fr: "PDF"
  display-name.de: "PDF"
  display-name.es: "PDF"
  display-name.pt-BR: "PDF"
  display-name.ru: "PDF"
  display-name: PDF
description: Read, generate, render and edit PDF documents, merge/split/rotate pages and fill form fields.
---

# Pdf

## Execution

Sailry bundles Python 3.12.13. `load_skill` prepares this plugin's hash-locked, wheel-only environment and returns its absolute `python` and `directory`. Run scripts with `"$SAILRY_PYTHON"` and absolute resource paths. Commands use the environment of the most recently loaded skill in this turn; reload the required skill before running if another environment was loaded later. Do not copy the returned interpreter path. The shared environment and plugin resources are read-only during commands; save task scripts and temporary files in `SAILRY_SESSION_DIR`, and write requested deliverables to the worktree. Use the existing command tool and its normal permissions; skill loading grants no command permission. If preparation or execution fails, report the failure and keep the draft. Do not install globally or modify Sailry's application/shared environments. For extra task dependencies, use the Commands Python skill's session-local environment. Inspect installed library APIs with `help()` or `inspect` when needed.

Native `read_office` provides structured inspection for DOCX, XLSX, PPTX and PDF. Author and edit with the script libraries. Scripts use the existing command tool's permissions and working directory. If command execution is disabled or denied, report that constraint without bypassing it.

For existing files, inspect the original and preserve unrelated content. For focused script edits, record the source hash, load the existing file, save to a sibling temporary file, reopen and validate it, recheck the source hash immediately before replacement, then use `os.replace`. Preserve the original and report a conflict if its hash changed. Do not rebuild an existing file from extracted plain text. A library may drop unsupported package parts: inspect those parts and use a focused ZIP/XML patch when necessary. Keep relationships, namespaces and content types consistent.

Before any save, create the destination directory with `Path(destination).parent.mkdir(parents=True, exist_ok=True)`. This applies to documents, images and temporary output files. Verify saved content and the requested changes. Use available PDF libraries for inspection and rendering; inspect representative rendered pages with the image viewing tool when layout matters. Keep editable source files when supplied. Return deliverables as ordinary standalone Markdown links, for example `[Report](output/report.pdf)`. Sailry provides cards and direct side-panel previews; do not fabricate output paths or JSON cards.

## PDF authoring

Use the prepared `reportlab` library to generate PDFs with precise layout and fonts, `pypdf` for page operations, metadata and forms, `pdfplumber` for text/table extraction, and `pypdfium2` for page rendering. Keep the editable Word/Excel/PPT source when supplied. Only convert it when the user requests conversion and a suitable renderer is available; Sailry does not bundle an Office-to-PDF converter.

```python
from reportlab.pdfgen import canvas
from reportlab.lib.pagesizes import A4
from pypdf import PdfReader
import pypdfium2 as pdfium
c = canvas.Canvas('report.pdf', pagesize=A4)
c.setFont('Helvetica', 18)
c.drawString(54, 780, 'Report')
c.save()
assert len(PdfReader('report.pdf').pages) == 1
pdf = pdfium.PdfDocument('report.pdf')
pdf[0].render(scale=1.5).to_pil().save('report-page-1.png')
```

Register an available Unicode font for multilingual documents; base PDF fonts do not cover arbitrary text. Inspect actual form field names/types before filling and reopen the saved values. A white rectangle or deleted extracted text is not secure redaction. These libraries are not OCR engines; scanned pages require a separate available OCR capability. Do not claim an empty extraction means a blank page. Verify page order/rotation, fonts, form values and rendered layout before delivering.

## Package resources

- `templates/create.py` is an editable starter script, not a restricted authoring API. Read it with `read_skill_resource`, copy it into the worktree, then adapt its full library calls to the user's document. Run it with the selected project interpreter and an output path. It creates missing parent directories and refuses to replace an existing file.
- `scripts/inspect_file.py` prints the saved file's content for verification. Run `"$SAILRY_PYTHON" "<skill-directory>/scripts/inspect_file.py" "<file>"` using the absolute directory returned by `load_skill`. Quote paths and keep package resources unchanged. Structural inspection alone does not prove layout fidelity.

The plugin contains instructions, scripts and templates, not an interpreter. `read_office` is supplied by Sailry's Files capability; `run_command` uses Commands on the execution Node. If a required capability is unavailable, explain what must be enabled. Installing or enabling this skill does not grant execution permission.

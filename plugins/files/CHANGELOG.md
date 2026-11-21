# Changes

## 0.1.32

- Stop advertising Office and PDF previews without a document extraction engine

## 0.1.31

- Remove local OCR model discovery and installation tools in the slim desktop build
- Keep file, image and notebook tools without a local inference runtime

## 0.1.30

- Remove host-provided Office and PDF extraction

## 0.1.29

- Restore outline icons in navigation and tools

## 0.1.28

- Use a new plugin icon

## 0.1.27

- Explain basename globs, ordered exclusions, explicit ignored paths and skipped encodings
- Distinguish captured output from source lines clipped at the native page limit
- Describe non-overlapping replace_all matches and conflicts between edits

## 0.1.26

- Read PNG, JPEG, GIF, WebP and BMP files with read_file so models with image input can see them; other models receive the format, dimensions and size
- Keep only the most recent read images in model requests

## 0.1.25

- Add apply_patch, which applies one patch to several files with a single approval; context must match exactly and changed files must be read first
- Describe the actual edit_file size limits

## 0.1.23

- Keep the Preview and Source switch inset at its full width in narrow toolbars

## 0.1.22

- Guide the model to read whole files and trust edit results instead of re-reading

## 0.1.21

- Default file searches to 100 results and describe bounded model previews

## 0.1.20

- Add Markdown formatting, block type, insert, link and soft wrap controls to the editor toolbar
- Add Find and replace and soft wrap for code files
- Switch modes with icons and move narrow-width controls into More
- Remove copy, cut and paste from the toolbar

## 0.1.19

- Show confirmed file writes and edits as line-numbered diffs

## 0.1.18

- Remove the redundant written notice from file write and edit results

## 0.1.17

- Align declaration checks with the current paged-read guidance

## 0.1.16

- Default file reads to 2000 lines and searches to matching paths
- Clarify bounded model text, exact continuations and retained original results
- Remove repeated editing guidance and redundant numeric schema bounds

## 0.1.15

- Remove the editing file-size limit from the paged-read description

## 0.1.14

- Read omitted file text through the native canonical result reader without requiring command tools

## 0.1.13

- Read smaller file pages and keep original tool output available through retained output references

## 0.1.12

- Ignore canceled document opens after their read is invalidated.
- Preserve genuine read errors and save feedback.

## 0.1.11

- Report each save failure once while retaining conflict and uncertain outcomes

## 0.1.10

- Show OCR model lists as concise activity while retaining file and notebook changes

## 0.1.9

- Read file capabilities and directories together
- Run independent opens, trash requests and confirmed transfers concurrently while preserving per-item recovery

## 0.1.8

- Add marketplace category and icon color
- Add localized tool names

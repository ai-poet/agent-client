---
name: image-generation
description: Generate or edit raster images using the built-in generate_image tool, handle reference-image edits, embed outputs in Markdown, and guide users through image backend errors.
metadata:
  display-name: Image Generation
  display-name.zh-CN: 图片生成与编辑
---

# Image Generation and Editing

Use the built-in `generate_image` tool to create new raster images or edit existing images.

## 1. When to Use and Tool Discovery

- Call `generate_image` whenever the user asks to draw, illustrate, design a visual asset, create a photo/poster, or edit an existing picture.
- `generate_image` is deferred: if it is not yet in your active tool list, load it first with `tool_search` using `select:generate_image`.
- Do not substitute SVG, ASCII art, or fabricated external image URLs when the user asks for a real generated or edited image.
- If `tool_search` does not return `generate_image`, no image backend is configured for this turn. Tell the user honestly and guide them to one of:
  1. Sign in to **CheapRouter** (**Settings → CheapRouter**), which automatically provides managed image generation and editing.
  2. Configure a custom OpenAI-compatible endpoint in **Settings → Image API**.
  3. Assign an Image model in **Settings → Media models**.
- Never run shell commands to inspect profile/credential files or call image HTTP APIs directly.

## 2. Backend Routing (`channel` and `model`)

- Check the available backends summary attached to `generate_image`'s tool description.
- **Default behavior**: omit `channel` and `model` so the runtime automatically uses the active default backend (priority: explicit Media Image model → signed-in CheapRouter → custom Image API → conversation model native image capability).
- Pass `channel` (`cheaprouter`, `custom`, or `media`) or `model` only when the user explicitly requests a specific backend or model.
- Do not invent model IDs that are not listed for the selected channel.

## 3. Prompt, Dimensions, Quality, Count, and Naming

- Write a focused, concrete `prompt` covering:
  - Subject and key details
  - Style, medium, and mood
  - Composition, framing, and lighting
  - Explicit constraints (for example background, color palette, or avoiding unwanted text)
- `size`: pick an aspect ratio that fits the request:
  - Square: `1024x1024` (or `2048x2048`)
  - Landscape: `1536x1024` (or `2048x1152`)
  - Portrait: `1024x1536`
  - Model default: `auto`
- `quality`: optional (`auto`, `low`, `medium`, `high`, `standard`, `hd`). Models that do not accept `quality` (such as Grok image models) ignore it automatically.
- `count`: integer `1`–`4` (default `1`). Only request multiple images when the user wants variations.
- `file_name`: optional plain filename with no path separators (e.g., `sunset-harbor.png`, `.png`/`.jpg`/`.webp`). Outputs are saved under `assets/generated/` and existing files are never overwritten; when `count > 1`, `-1`/`-2` suffixes are added automatically.

## 4. Editing Existing Images (`references`)

- When the user wants to edit, restyle, extend, combine, or iterate on existing images, pass `references` as an array of 1–8 entries:
  - Worktree-relative image paths (e.g., `["assets/generated/draft.png"]` or `["images/hero.jpg"]`)
  - Conversation attachment IDs supplied in this session
- In `prompt`, state clearly both **what to change** and **what to keep consistent** from the reference image(s).
- Reference images must be PNG, JPEG, or WebP and at most 20 MiB each.
- Editing uses `/v1/images/edits` on CheapRouter and custom OpenAI-compatible endpoints. If a custom endpoint or native Media model reports that image editing is unsupported, tell the user clearly and suggest switching to CheapRouter or an edits-capable custom Image API.

## 5. Presenting Results

- After `generate_image` succeeds, embed every returned workspace path (`path`, or each entry in `paths` when `count > 1`) directly in your reply using Markdown image syntax:
  `![Short descriptive alt text](assets/generated/your-image.png)`
- Briefly note the saved path(s) and model used. Never paste base64 image data into your response.

## 6. Error Handling and Billing Safety

- **Not configured (`not_configured`)**: guide the user to sign in under **Settings → CheapRouter**, configure **Settings → Image API**, or assign an Image model in **Settings → Media models**.
- **Authentication failed / expired session / invalid API key**: ask the user to sign in to CheapRouter again or verify the API key in **Settings → Image API**.
- **Permission denied (`permission_denied`) / unsupported group or model**: explain that the current CheapRouter group cannot run image generation for this model; suggest switching to an image-capable group in **Settings → CheapRouter** or configuring **Settings → Image API**.
- **Insufficient balance / quota / rate limit (`unavailable`)**: explain the balance or quota limit clearly and suggest recharging CheapRouter or switching to a custom Image API.
- **Content policy rejection (`invalid_request`)**: explain that the request was blocked by safety filters and offer to revise the prompt or references.
- **Outcome unknown (`outcome_unknown`, timeout/disconnect after a gateway `task_id` was accepted)**: report the returned `task_id` to the user and **do not automatically retry or resubmit the prompt**, to avoid duplicate billing.
- **Cancelled (`cancelled`)**: stop and acknowledge cancellation without retrying.

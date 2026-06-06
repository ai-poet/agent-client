import type MarkdownIt from "markdown-it";
import type Token from "markdown-it/lib/token.mjs";
import type StateBlock from "markdown-it/lib/rules_block/state_block.mjs";
import type StateCore from "markdown-it/lib/rules_core/state_core.mjs";
import type StateInline from "markdown-it/lib/rules_inline/state_inline.mjs";

const INLINE_DOLLAR = "$";
const BLOCK_DOLLAR = "$$";
const INLINE_BRACKET_OPEN = "\\(";
const INLINE_BRACKET_CLOSE = "\\)";
const BLOCK_BRACKET_OPEN = "\\[";
const BLOCK_BRACKET_CLOSE = "\\]";

export const MARKDOWN_MATH_INLINE_TOKEN = "math_inline";
export const MARKDOWN_MATH_BLOCK_TOKEN = "math_block";

export type MarkdownMathSegment =
  | { kind: "text"; content: string }
  | { kind: "math_inline"; content: string; open: "$" | "\\("; close: "$" | "\\)" };

function isEscaped(source: string, index: number): boolean {
  let slashCount = 0;
  for (let cursor = index - 1; cursor >= 0 && source[cursor] === "\\"; cursor--) {
    slashCount++;
  }
  return slashCount % 2 === 1;
}

function hasNonWhitespace(value: string): boolean {
  return value.trim().length > 0;
}

function canOpenDollarMath(source: string, index: number): boolean {
  if (source[index] !== INLINE_DOLLAR || isEscaped(source, index)) {
    return false;
  }

  if (source[index + 1] === INLINE_DOLLAR) {
    return false;
  }

  const next = source[index + 1];
  if (!next || /\s/.test(next)) {
    return false;
  }

  return true;
}

function canCloseDollarMath(source: string, index: number): boolean {
  if (source[index] !== INLINE_DOLLAR || isEscaped(source, index)) {
    return false;
  }

  if (source[index - 1] === INLINE_DOLLAR || source[index + 1] === INLINE_DOLLAR) {
    return false;
  }

  const previous = source[index - 1];
  if (!previous || /\s/.test(previous)) {
    return false;
  }

  const next = source[index + 1];
  if (next && /\d/.test(next)) {
    return false;
  }

  return true;
}

function findDollarMathClose(source: string, startIndex: number): number {
  for (let cursor = startIndex + 1; cursor < source.length; cursor++) {
    if (canCloseDollarMath(source, cursor)) {
      return cursor;
    }
  }
  return -1;
}

function findBracketClose(source: string, startIndex: number): number {
  for (let cursor = startIndex + INLINE_BRACKET_OPEN.length; cursor < source.length - 1; cursor++) {
    if (source.startsWith(INLINE_BRACKET_CLOSE, cursor) && !isEscaped(source, cursor)) {
      return cursor;
    }
  }
  return -1;
}

export function splitInlineMarkdownMath(source: string): MarkdownMathSegment[] {
  const segments: MarkdownMathSegment[] = [];
  let textStart = 0;
  let cursor = 0;

  while (cursor < source.length) {
    if (source.startsWith(INLINE_BRACKET_OPEN, cursor) && !isEscaped(source, cursor)) {
      const closeIndex = findBracketClose(source, cursor);
      if (closeIndex > cursor) {
        const content = source.slice(cursor + INLINE_BRACKET_OPEN.length, closeIndex);
        if (hasNonWhitespace(content)) {
          if (textStart < cursor) {
            segments.push({ kind: "text", content: source.slice(textStart, cursor) });
          }
          segments.push({
            kind: "math_inline",
            content,
            open: INLINE_BRACKET_OPEN,
            close: INLINE_BRACKET_CLOSE,
          });
          cursor = closeIndex + INLINE_BRACKET_CLOSE.length;
          textStart = cursor;
          continue;
        }
      }
    }

    if (canOpenDollarMath(source, cursor)) {
      const closeIndex = findDollarMathClose(source, cursor);
      if (closeIndex > cursor) {
        const content = source.slice(cursor + 1, closeIndex);
        if (hasNonWhitespace(content)) {
          if (textStart < cursor) {
            segments.push({ kind: "text", content: source.slice(textStart, cursor) });
          }
          segments.push({
            kind: "math_inline",
            content,
            open: INLINE_DOLLAR,
            close: INLINE_DOLLAR,
          });
          cursor = closeIndex + 1;
          textStart = cursor;
          continue;
        }
      }
    }

    cursor++;
  }

  if (textStart < source.length) {
    segments.push({ kind: "text", content: source.slice(textStart) });
  }

  return segments.length > 0 ? segments : [{ kind: "text", content: source }];
}

function readLine(state: StateBlock, line: number): string {
  const start = state.bMarks[line] + state.tShift[line];
  const end = state.eMarks[line];
  return state.src.slice(start, end);
}

function readTrimmedLine(state: StateBlock, line: number): string {
  return readLine(state, line).trim();
}

function findBlockCloseLine(
  state: StateBlock,
  startLine: number,
  endLine: number,
  closeDelimiter: string,
): number {
  for (let line = startLine + 1; line < endLine; line++) {
    if (readTrimmedLine(state, line) === closeDelimiter) {
      return line;
    }
  }
  return -1;
}

function mathBlockRule(
  state: StateBlock,
  startLine: number,
  endLine: number,
  silent: boolean,
): boolean {
  const openingLine = readTrimmedLine(state, startLine);
  const closeDelimiter =
    openingLine === BLOCK_DOLLAR
      ? BLOCK_DOLLAR
      : openingLine === BLOCK_BRACKET_OPEN
        ? BLOCK_BRACKET_CLOSE
        : null;

  if (!closeDelimiter) {
    return false;
  }

  const closeLine = findBlockCloseLine(state, startLine, endLine, closeDelimiter);
  if (closeLine < 0) {
    return false;
  }

  if (silent) {
    return true;
  }

  const contentLines: string[] = [];
  for (let line = startLine + 1; line < closeLine; line++) {
    contentLines.push(readLine(state, line));
  }

  const token = state.push(MARKDOWN_MATH_BLOCK_TOKEN, "math", 0);
  token.block = true;
  token.content = contentLines.join("\n");
  token.markup = openingLine;
  token.map = [startLine, closeLine + 1];
  token.meta = {
    open: openingLine,
    close: closeDelimiter,
  };

  state.line = closeLine + 1;
  return true;
}

function inlineMathRule(state: StateInline, silent: boolean): boolean {
  const source = state.src;
  const start = state.pos;
  const firstSegment = splitInlineMarkdownMath(source.slice(start))[0];

  if (!firstSegment || firstSegment.kind !== "math_inline") {
    return false;
  }

  if (silent) {
    return true;
  }

  const token = state.push(MARKDOWN_MATH_INLINE_TOKEN, "math", 0);
  token.content = firstSegment.content;
  token.markup = firstSegment.open;
  token.meta = {
    open: firstSegment.open,
    close: firstSegment.close,
  };
  state.pos =
    start + firstSegment.open.length + firstSegment.content.length + firstSegment.close.length;
  return true;
}

function splitTextMathCoreRule(state: StateCore): void {
  for (const blockToken of state.tokens) {
    if (!blockToken.children) {
      continue;
    }

    const nextChildren: Token[] = [];
    for (const child of blockToken.children) {
      if (
        child.type !== "text" ||
        (!child.content.includes(INLINE_DOLLAR) && !child.content.includes(INLINE_BRACKET_OPEN))
      ) {
        nextChildren.push(child);
        continue;
      }

      for (const segment of splitInlineMarkdownMath(child.content)) {
        if (segment.kind === "text") {
          const token = new state.Token("text", "", 0);
          token.content = segment.content;
          nextChildren.push(token);
        } else {
          const token = new state.Token(MARKDOWN_MATH_INLINE_TOKEN, "math", 0);
          token.content = segment.content;
          token.markup = segment.open;
          token.meta = { open: segment.open, close: segment.close };
          nextChildren.push(token);
        }
      }
    }
    blockToken.children = nextChildren;
  }
}

export function markdownMathPlugin(md: MarkdownIt): void {
  md.block.ruler.before("fence", MARKDOWN_MATH_BLOCK_TOKEN, mathBlockRule, {
    alt: ["paragraph", "reference", "blockquote"],
  });
  md.inline.ruler.before("escape", MARKDOWN_MATH_INLINE_TOKEN, inlineMathRule);
  md.core.ruler.after("inline", "split_text_math", splitTextMathCoreRule);
}

import MarkdownIt from "markdown-it";
import { describe, expect, it } from "vitest";
import {
  MARKDOWN_MATH_BLOCK_TOKEN,
  MARKDOWN_MATH_INLINE_TOKEN,
  markdownMathPlugin,
  splitInlineMarkdownMath,
} from "./markdown-math";

describe("splitInlineMarkdownMath", () => {
  it("splits dollar-delimited inline math", () => {
    expect(splitInlineMarkdownMath("Euler says $e^{i\\pi}+1=0$.")).toEqual([
      { kind: "text", content: "Euler says " },
      {
        kind: "math_inline",
        content: "e^{i\\pi}+1=0",
        open: "$",
        close: "$",
      },
      { kind: "text", content: "." },
    ]);
  });

  it("splits bracket-delimited inline math", () => {
    expect(splitInlineMarkdownMath("Let \\(x^2\\) be positive.")).toEqual([
      { kind: "text", content: "Let " },
      {
        kind: "math_inline",
        content: "x^2",
        open: "\\(",
        close: "\\)",
      },
      { kind: "text", content: " be positive." },
    ]);
  });

  it("leaves escaped dollars as text", () => {
    expect(splitInlineMarkdownMath("Use \\$x$ literally.")).toEqual([
      { kind: "text", content: "Use \\$x$ literally." },
    ]);
  });

  it("does not parse currency-like values as math", () => {
    expect(splitInlineMarkdownMath("Costs are $20 and $30 today.")).toEqual([
      { kind: "text", content: "Costs are $20 and $30 today." },
    ]);
  });

  it("keeps incomplete streaming math as text", () => {
    expect(splitInlineMarkdownMath("The value is $x^2 +")).toEqual([
      { kind: "text", content: "The value is $x^2 +" },
    ]);
  });
});

describe("markdownMathPlugin", () => {
  it("emits inline math tokens", () => {
    const md = new MarkdownIt({ html: false }).use(markdownMathPlugin);
    const paragraph = md.parse("Area is $a^2$.", {})[1];

    expect(paragraph?.children?.map((child) => [child.type, child.content])).toEqual([
      ["text", "Area is "],
      [MARKDOWN_MATH_INLINE_TOKEN, "a^2"],
      ["text", "."],
    ]);
  });

  it("emits block math tokens for dollar fences", () => {
    const md = new MarkdownIt({ html: false }).use(markdownMathPlugin);
    const tokens = md.parse("Before\n\n$$\na^2+b^2=c^2\n$$\n\nAfter", {});

    expect(tokens.map((token) => token.type)).toContain(MARKDOWN_MATH_BLOCK_TOKEN);
    expect(tokens.find((token) => token.type === MARKDOWN_MATH_BLOCK_TOKEN)?.content).toBe(
      "a^2+b^2=c^2",
    );
  });

  it("emits block math tokens for bracket fences", () => {
    const md = new MarkdownIt({ html: false }).use(markdownMathPlugin);
    const tokens = md.parse("\\[\n\\int_0^1 x dx\n\\]", {});

    expect(tokens.find((token) => token.type === MARKDOWN_MATH_BLOCK_TOKEN)?.content).toBe(
      "\\int_0^1 x dx",
    );
  });

  it("keeps unclosed block math as paragraph text", () => {
    const md = new MarkdownIt({ html: false }).use(markdownMathPlugin);
    const tokens = md.parse("$$\na^2+b^2=c^2", {});

    expect(tokens.map((token) => token.type)).not.toContain(MARKDOWN_MATH_BLOCK_TOKEN);
  });
});

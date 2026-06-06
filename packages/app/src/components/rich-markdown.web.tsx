import {
  default as React,
  createElement,
  useCallback,
  useEffect,
  useMemo,
  useState,
  type CSSProperties,
  type MouseEvent,
  type ReactNode,
  type SyntheticEvent,
} from "react";
import MarkdownIt from "markdown-it";
import type Token from "markdown-it/lib/token.mjs";
import katex from "katex";
import "katex/dist/katex.min.css";
import * as Clipboard from "expo-clipboard";
import { Check, Copy } from "lucide-react-native";
import { useQuery } from "@tanstack/react-query";
import { useUnistyles } from "react-native-unistyles";
import { darkHighlightColors, highlightCode, lightHighlightColors } from "@getpaseo/highlight";
import type { HighlightToken } from "@getpaseo/highlight";
import type { DaemonClient } from "@server/client/daemon-client";
import { getIsElectron } from "@/constants/platform";
import { Fonts } from "@/constants/theme";
import type { Theme } from "@/styles/theme";
import {
  MARKDOWN_MATH_BLOCK_TOKEN,
  MARKDOWN_MATH_INLINE_TOKEN,
  markdownMathPlugin,
} from "@/utils/markdown-math";
import { getMarkdownListMarker } from "@/utils/markdown-list";
import {
  parseAssistantFileLink,
  parseInlinePathToken,
  type InlinePathTarget,
} from "@/utils/inline-path";
import { resolveAssistantImageSource } from "@/utils/assistant-image-source";
import {
  getAssistantImageMetadata,
  setAssistantImageMetadata,
} from "@/utils/assistant-image-metadata";
import type { RichMarkdownProps } from "./rich-markdown";

type MarkdownNode = {
  type: string;
  tag: string;
  nesting: number;
  content: string;
  markup: string;
  info: string;
  block: boolean;
  key: string;
  index: number;
  attributes: Record<string, string>;
  sourceMeta?: unknown;
  children: MarkdownNode[];
};

const MARKDOWN_ENV = {};
const COPY_RESET_DELAY_MS = 1500;
const ASSISTANT_IMAGE_MIN_HEIGHT = 160;
const MARKDOWN_STYLE_ID = "paseo-rich-markdown-style";
const MARKDOWN_GLOBAL_CSS = `
  .paseo-rich-markdown,
  .paseo-rich-markdown * {
    box-sizing: border-box;
  }

  .paseo-rich-markdown {
    user-select: text;
    -webkit-user-select: text;
    overflow-wrap: anywhere;
  }

  .paseo-rich-markdown pre,
  .paseo-rich-markdown code,
  .paseo-rich-markdown table,
  .paseo-rich-markdown .katex {
    user-select: text;
    -webkit-user-select: text;
  }

  .paseo-rich-markdown-table-scroll,
  .paseo-rich-markdown-code-scroll,
  .paseo-rich-markdown-math-scroll {
    scrollbar-width: thin;
  }
`;

function ensureRichMarkdownGlobalStyle() {
  if (typeof document === "undefined") {
    return;
  }
  const existing = document.getElementById(MARKDOWN_STYLE_ID);
  if (existing) {
    if (existing.textContent !== MARKDOWN_GLOBAL_CSS) {
      existing.textContent = MARKDOWN_GLOBAL_CSS;
    }
    return;
  }
  const style = document.createElement("style");
  style.id = MARKDOWN_STYLE_ID;
  style.textContent = MARKDOWN_GLOBAL_CSS;
  document.head.appendChild(style);
}

function createMarkdownParser() {
  const parser = new MarkdownIt({
    html: false,
    linkify: true,
    typographer: true,
    breaks: false,
  });
  const defaultValidateLink = parser.validateLink.bind(parser);
  parser.validateLink = (url: string) => {
    if (url.trim().toLowerCase().startsWith("file://")) {
      return true;
    }
    return defaultValidateLink(url);
  };
  parser.use(markdownMathPlugin);
  return parser;
}

function tokenType(token: Token): string {
  const cleaned = token.type.replace(/_open|_close/g, "");
  if (cleaned === "heading") {
    return `heading${token.tag.slice(1)}`;
  }
  return cleaned;
}

function tokenAttributes(token: Token): Record<string, string> {
  const attrs: Record<string, string> = {};
  for (const [name, value] of token.attrs ?? []) {
    attrs[name] = value;
  }
  return attrs;
}

function tokensToNodes(tokens: Token[]): MarkdownNode[] {
  const root: MarkdownNode = {
    type: "root",
    tag: "",
    nesting: 1,
    content: "",
    markup: "",
    info: "",
    block: true,
    key: "root",
    index: 0,
    attributes: {},
    children: [],
  };
  const stack = [root];
  let nextKey = 0;

  for (const token of tokens) {
    const parent = stack[stack.length - 1] ?? root;
    const node: MarkdownNode = {
      type: tokenType(token),
      tag: token.tag,
      nesting: token.nesting,
      content: token.content,
      markup: token.markup,
      info: token.info,
      block: token.block,
      key: `${nextKey++}-${token.type}`,
      index: parent.children.length,
      attributes: tokenAttributes(token),
      sourceMeta: token.meta,
      children: token.children ? tokensToNodes(token.children) : [],
    };

    if (token.nesting === -1) {
      if (stack.length > 1) {
        stack.pop();
      }
      continue;
    }

    parent.children.push(node);
    if (token.nesting === 1) {
      stack.push(node);
    }
  }

  return root.children;
}

function normalizeLanguage(info: string): string {
  return (
    info
      .trim()
      .split(/\s+/)[0]
      ?.replace(/^language-/, "") ?? ""
  );
}

function languageToFilename(language: string): string {
  if (!language) {
    return "snippet.txt";
  }
  const aliases: Record<string, string> = {
    javascript: "js",
    typescript: "ts",
    python: "py",
    rust: "rs",
    markdown: "md",
    shell: "sh",
    bash: "sh",
    zsh: "sh",
  };
  return `snippet.${aliases[language.toLowerCase()] ?? language.toLowerCase()}`;
}

function trimCodeBlockContent(content: string): string {
  return content.endsWith("\n") ? content.slice(0, -1) : content;
}

function isTaskListNode(node: MarkdownNode): { checked: boolean; labelPrefix: string } | null {
  const firstText = findFirstTextNode(node);
  if (!firstText) {
    return null;
  }
  const match = /^\[([ xX])]\s+/.exec(firstText.content);
  if (!match) {
    return null;
  }
  return {
    checked: match[1]?.toLowerCase() === "x",
    labelPrefix: match[0],
  };
}

function findFirstTextNode(node: MarkdownNode): MarkdownNode | null {
  if (node.type === "text") {
    return node;
  }
  for (const child of node.children) {
    const found = findFirstTextNode(child);
    if (found) {
      return found;
    }
  }
  return null;
}

function stripTaskPrefixFromFirstText(node: MarkdownNode, labelPrefix: string): MarkdownNode {
  let stripped = false;
  function visit(current: MarkdownNode): MarkdownNode {
    if (!stripped && current.type === "text" && current.content.startsWith(labelPrefix)) {
      stripped = true;
      return {
        ...current,
        content: current.content.slice(labelPrefix.length),
      };
    }
    return {
      ...current,
      children: current.children.map(visit),
    };
  }
  return visit(node);
}

function createStyles(theme: Theme, variant: RichMarkdownProps["variant"]) {
  const codeSurface = theme.colorScheme === "dark" ? theme.colors.surface2 : theme.colors.surface1;
  const codeBorder = theme.colorScheme === "dark" ? theme.colors.borderAccent : theme.colors.border;
  const mutedCodeText = theme.colorScheme === "dark" ? "#d4d4d8" : "#24292f";
  const bodyFontSize = variant === "plan" ? theme.fontSize.sm : theme.fontSize.base;
  const bodyLineHeight = variant === "plan" ? 20 : 22;
  return {
    root: {
      color: theme.colors.foreground,
      fontSize: bodyFontSize,
      lineHeight: `${bodyLineHeight}px`,
      width: "100%",
      minWidth: 0,
      fontFamily: Fonts.sans,
    } satisfies CSSProperties,
    paragraph: {
      margin: "0 0 12px 0",
      color: theme.colors.foreground,
    } satisfies CSSProperties,
    paragraphLast: {
      marginBottom: 0,
    } satisfies CSSProperties,
    headingBase: {
      color: theme.colors.foreground,
      fontWeight: 650,
      margin: "18px 0 8px 0",
      lineHeight: 1.25,
    } satisfies CSSProperties,
    heading1: {
      fontSize: variant === "plan" ? 18 : 24,
      paddingBottom: 8,
      borderBottom: `1px solid ${theme.colors.border}`,
    } satisfies CSSProperties,
    heading2: {
      fontSize: variant === "plan" ? 17 : 22,
      paddingBottom: 6,
      borderBottom: `1px solid ${theme.colors.border}`,
    } satisfies CSSProperties,
    heading3: {
      fontSize: variant === "plan" ? 16 : 19,
    } satisfies CSSProperties,
    heading4: {
      fontSize: variant === "plan" ? 15 : 17,
    } satisfies CSSProperties,
    heading5: {
      fontSize: bodyFontSize,
    } satisfies CSSProperties,
    heading6: {
      fontSize: bodyFontSize,
      color: theme.colors.foregroundMuted,
      textTransform: "uppercase",
      letterSpacing: 0.5,
    } satisfies CSSProperties,
    link: {
      color: theme.colors.accentBright,
      textDecoration: "none",
      cursor: "pointer",
    } satisfies CSSProperties,
    inlineCode: {
      color: theme.colors.foreground,
      backgroundColor: codeSurface,
      borderRadius: 6,
      padding: "2px 5px",
      fontFamily: Fonts.mono,
      fontSize: "0.9em",
      whiteSpace: "break-spaces",
    } satisfies CSSProperties,
    pathChip: {
      display: "inline-block",
      color: theme.colors.foreground,
      backgroundColor: codeSurface,
      borderRadius: 999,
      padding: "2px 8px",
      margin: "1px 3px 1px 0",
      fontFamily: Fonts.mono,
      fontSize: 13,
      cursor: "pointer",
    } satisfies CSSProperties,
    codeBlock: {
      margin: "12px 0",
      border: `1px solid ${codeBorder}`,
      borderRadius: 8,
      overflow: "hidden",
      backgroundColor: codeSurface,
    } satisfies CSSProperties,
    codeHeader: {
      display: "flex",
      alignItems: "center",
      justifyContent: "space-between",
      minHeight: 32,
      padding: "5px 8px 5px 12px",
      borderBottom: `1px solid ${codeBorder}`,
      color: theme.colors.foregroundMuted,
      fontSize: 12,
      fontFamily: Fonts.sans,
    } satisfies CSSProperties,
    codeLanguage: {
      fontFamily: Fonts.mono,
      color: theme.colors.foregroundMuted,
    } satisfies CSSProperties,
    copyButton: {
      display: "inline-flex",
      alignItems: "center",
      justifyContent: "center",
      width: 24,
      height: 24,
      padding: 0,
      border: "none",
      borderRadius: 5,
      background: "transparent",
      color: theme.colors.foregroundMuted,
      cursor: "pointer",
    } satisfies CSSProperties,
    codeScroll: {
      overflowX: "auto",
      maxWidth: "100%",
    } satisfies CSSProperties,
    pre: {
      margin: 0,
      padding: 12,
      minWidth: "max-content",
      color: mutedCodeText,
      fontFamily: Fonts.mono,
      fontSize: 13,
      lineHeight: "20px",
      whiteSpace: "pre",
    } satisfies CSSProperties,
    codeLine: {
      display: "block",
      minHeight: 20,
    } satisfies CSSProperties,
    list: {
      margin: "8px 0 12px 0",
      padding: 0,
      listStyle: "none",
    } satisfies CSSProperties,
    listItem: {
      display: "flex",
      alignItems: "flex-start",
      gap: 6,
      marginBottom: 5,
      minWidth: 0,
    } satisfies CSSProperties,
    listMarker: {
      flex: "0 0 auto",
      color: theme.colors.foregroundMuted,
      minWidth: 16,
      lineHeight: `${bodyLineHeight}px`,
      textAlign: "right",
    } satisfies CSSProperties,
    listContent: {
      minWidth: 0,
      flex: 1,
    } satisfies CSSProperties,
    taskBox: {
      width: 15,
      height: 15,
      marginTop: 3,
      borderRadius: 4,
      border: `1px solid ${theme.colors.border}`,
      display: "inline-flex",
      alignItems: "center",
      justifyContent: "center",
      flex: "0 0 auto",
      backgroundColor: theme.colors.surface1,
      color: theme.colors.accentBright,
    } satisfies CSSProperties,
    blockquote: {
      margin: "12px 0",
      padding: "10px 14px",
      borderLeft: `4px solid ${theme.colors.primary}`,
      borderRadius: 6,
      backgroundColor: codeSurface,
      color: theme.colors.foreground,
    } satisfies CSSProperties,
    hr: {
      border: 0,
      height: 1,
      backgroundColor: theme.colors.border,
      margin: "20px 0",
    } satisfies CSSProperties,
    tableScroll: {
      overflowX: "auto",
      maxWidth: "100%",
      margin: "12px 0",
      border: `1px solid ${theme.colors.border}`,
      borderRadius: 8,
    } satisfies CSSProperties,
    table: {
      width: "100%",
      minWidth: "max-content",
      borderCollapse: "collapse",
      fontSize: 14,
      lineHeight: "20px",
    } satisfies CSSProperties,
    th: {
      padding: "8px 10px",
      borderRight: `1px solid ${theme.colors.border}`,
      borderBottom: `1px solid ${theme.colors.border}`,
      backgroundColor: codeSurface,
      color: theme.colors.foreground,
      fontWeight: 600,
      textAlign: "left",
      whiteSpace: "nowrap",
    } satisfies CSSProperties,
    td: {
      padding: "8px 10px",
      borderRight: `1px solid ${theme.colors.border}`,
      borderBottom: `1px solid ${theme.colors.border}`,
      color: theme.colors.foreground,
      verticalAlign: "top",
    } satisfies CSSProperties,
    mathInline: {
      display: "inline-block",
      maxWidth: "100%",
      overflowX: "auto",
      overflowY: "hidden",
      verticalAlign: "middle",
    } satisfies CSSProperties,
    mathBlock: {
      maxWidth: "100%",
      overflowX: "auto",
      overflowY: "hidden",
      margin: "12px 0",
      padding: "4px 0",
      textAlign: "center",
    } satisfies CSSProperties,
    mathError: {
      color: theme.colors.destructive,
      backgroundColor: codeSurface,
      borderRadius: 6,
      padding: "2px 5px",
      fontFamily: Fonts.mono,
      fontSize: "0.9em",
      whiteSpace: "break-spaces",
    } satisfies CSSProperties,
    imageFrame: {
      width: "100%",
      minHeight: ASSISTANT_IMAGE_MIN_HEIGHT,
      margin: "12px -4px 0 -4px",
    } satisfies CSSProperties,
    imageSurface: {
      width: "100%",
      minHeight: ASSISTANT_IMAGE_MIN_HEIGHT,
      display: "flex",
      alignItems: "center",
      justifyContent: "center",
      overflow: "hidden",
    } satisfies CSSProperties,
    image: {
      width: "100%",
      height: "100%",
      objectFit: "contain",
      display: "block",
    } satisfies CSSProperties,
    imageState: {
      color: theme.colors.foregroundMuted,
      fontSize: 14,
      textAlign: "center",
      padding: "24px 16px",
    } satisfies CSSProperties,
  };
}

function isLastChild(node: MarkdownNode, ancestors: MarkdownNode[]): boolean {
  const parent = ancestors[ancestors.length - 1];
  return Boolean(parent && parent.children[parent.children.length - 1]?.key === node.key);
}

function hasAncestorType(ancestors: MarkdownNode[], type: string): boolean {
  return ancestors.some((ancestor) => ancestor.type === type);
}

function renderChildren(input: RenderContext, children: MarkdownNode[], ancestors: MarkdownNode[]) {
  return children.map((child) => renderNode(input, child, ancestors));
}

type RenderContext = {
  styles: ReturnType<typeof createStyles>;
  theme: Theme;
  parser: MarkdownIt;
  onLinkPress?: (url: string) => boolean | void;
  onInlinePathPress?: (target: InlinePathTarget) => void;
  workspaceRoot?: string;
  serverId?: string;
  client?: DaemonClient | null;
};

function MarkdownLink({
  href,
  children,
  context,
}: {
  href: string;
  children: ReactNode;
  context: RenderContext;
}) {
  const handleClick = useCallback(
    (event: MouseEvent<HTMLAnchorElement>) => {
      event.preventDefault();
      context.onLinkPress?.(href);
    },
    [context, href],
  );

  return (
    <a href={href} style={context.styles.link} onClick={handleClick}>
      {children}
    </a>
  );
}

function InlineCode({ node, context }: { node: MarkdownNode; context: RenderContext }) {
  const content = node.content ?? "";
  const parsed = context.onInlinePathPress ? parseInlinePathToken(content) : null;
  if (parsed) {
    return (
      <button
        type="button"
        style={{
          ...context.styles.pathChip,
          border: "none",
          font: "inherit",
          fontFamily: Fonts.mono,
        }}
        onClick={() => context.onInlinePathPress?.(parsed)}
      >
        {content}
      </button>
    );
  }

  const matches = context.parser.linkify.match(content.trim()) as Array<{
    index: number;
    lastIndex: number;
    url: string;
  }> | null;
  const match = matches?.length === 1 ? matches[0] : null;
  if (match && match.index === 0 && match.lastIndex === content.trim().length) {
    return (
      <MarkdownLink href={match.url} context={context}>
        <code style={context.styles.inlineCode}>{content}</code>
      </MarkdownLink>
    );
  }

  return <code style={context.styles.inlineCode}>{content}</code>;
}

function CodeBlock({
  content,
  info,
  context,
}: {
  content: string;
  info: string;
  context: RenderContext;
}) {
  const [copied, setCopied] = useState(false);
  const code = trimCodeBlockContent(content);
  const language = normalizeLanguage(info);
  const highlighted = useMemo(
    () => highlightCode(code, languageToFilename(language)),
    [code, language],
  );
  const highlightColors =
    context.theme.colorScheme === "dark" ? darkHighlightColors : lightHighlightColors;

  useEffect(() => {
    if (!copied) {
      return;
    }
    const timeout = setTimeout(() => setCopied(false), COPY_RESET_DELAY_MS);
    return () => clearTimeout(timeout);
  }, [copied]);

  const copyCode = useCallback(async () => {
    await Clipboard.setStringAsync(code);
    setCopied(true);
  }, [code]);

  return (
    <div style={context.styles.codeBlock} data-testid="rich-markdown-code-block">
      <div style={context.styles.codeHeader}>
        <span style={context.styles.codeLanguage}>{language || "text"}</span>
        <button
          type="button"
          aria-label={copied ? "Copied code" : "Copy code"}
          title={copied ? "Copied" : "Copy"}
          style={context.styles.copyButton}
          onClick={copyCode}
        >
          {copied ? <Check size={15} /> : <Copy size={15} />}
        </button>
      </div>
      <div className="paseo-rich-markdown-code-scroll" style={context.styles.codeScroll}>
        <pre style={context.styles.pre}>
          <code>
            {highlighted.map((line, lineIndex) => (
              <span key={lineIndex} style={context.styles.codeLine}>
                {line.map((token, tokenIndex) => (
                  <HighlightSpan
                    key={`${lineIndex}-${tokenIndex}`}
                    token={token}
                    colors={highlightColors}
                  />
                ))}
                {lineIndex < highlighted.length - 1 ? "\n" : null}
              </span>
            ))}
          </code>
        </pre>
      </div>
    </div>
  );
}

function HighlightSpan({
  token,
  colors,
}: {
  token: HighlightToken;
  colors: typeof darkHighlightColors;
}) {
  return <span style={token.style ? { color: colors[token.style] } : undefined}>{token.text}</span>;
}

function MathNode({
  content,
  displayMode,
  context,
}: {
  content: string;
  displayMode: boolean;
  context: RenderContext;
}) {
  const html = useMemo(() => {
    try {
      const rendered = katex.renderToString(content, {
        displayMode,
        throwOnError: false,
        strict: "warn",
        output: "html",
      });
      if (rendered.includes("katex-error")) {
        return { ok: false as const, value: content };
      }
      return {
        ok: true as const,
        value: rendered,
      };
    } catch {
      return { ok: false as const, value: content };
    }
  }, [content, displayMode]);

  if (!html.ok) {
    return <code style={context.styles.mathError}>{content}</code>;
  }

  return (
    <span
      className={displayMode ? "paseo-rich-markdown-math-scroll" : undefined}
      style={displayMode ? context.styles.mathBlock : context.styles.mathInline}
      dangerouslySetInnerHTML={{ __html: html.value }}
    />
  );
}

function AssistantImage({
  source,
  alt,
  context,
}: {
  source: string;
  alt?: string;
  context: RenderContext;
}) {
  const resolution = useMemo(
    () => resolveAssistantImageSource({ source, workspaceRoot: context.workspaceRoot }),
    [context.workspaceRoot, source],
  );
  const query = useQuery({
    queryKey: [
      "richMarkdownImage",
      context.serverId ?? "unknown-server",
      resolution?.kind === "file_rpc" ? resolution.cwd : null,
      resolution?.kind === "file_rpc" ? resolution.path : null,
    ],
    enabled: Boolean(context.client && resolution?.kind === "file_rpc"),
    staleTime: 30_000,
    queryFn: async () => {
      if (!context.client || !resolution || resolution.kind !== "file_rpc") {
        return null;
      }
      const payload = await context.client.exploreFileSystem(
        resolution.cwd,
        resolution.path,
        "file",
      );
      if (payload.error) {
        throw new Error(payload.error);
      }
      if (!payload.file || payload.file.kind !== "image" || !payload.file.content) {
        throw new Error("Image preview unavailable.");
      }
      return `data:${payload.file.mimeType ?? "image/png"};base64,${payload.file.content}`;
    },
  });
  const directUri = resolution?.kind === "direct" ? resolution.uri : null;
  const uri = directUri ?? query.data ?? null;
  const cachedMetadata = useMemo(
    () =>
      getAssistantImageMetadata({
        source,
        workspaceRoot: context.workspaceRoot,
        serverId: context.serverId,
      }),
    [context.serverId, context.workspaceRoot, source],
  );
  const [aspectRatio, setAspectRatio] = useState<number | null>(
    cachedMetadata?.aspectRatio ?? null,
  );

  useEffect(() => {
    setAspectRatio(cachedMetadata?.aspectRatio ?? null);
  }, [cachedMetadata]);

  const handleLoad = useCallback(
    (event: SyntheticEvent<HTMLImageElement>) => {
      const { naturalWidth, naturalHeight } = event.currentTarget;
      const metadata = setAssistantImageMetadata(
        {
          source,
          workspaceRoot: context.workspaceRoot,
          serverId: context.serverId,
        },
        { width: naturalWidth, height: naturalHeight },
      );
      setAspectRatio(metadata?.aspectRatio ?? null);
    },
    [context.serverId, context.workspaceRoot, source],
  );

  if (!uri) {
    return (
      <div style={{ ...context.styles.imageFrame, ...context.styles.imageState }}>
        {query.isLoading
          ? "Loading image..."
          : query.error instanceof Error
            ? query.error.message
            : "Unable to load image preview."}
      </div>
    );
  }

  return (
    <div style={context.styles.imageFrame}>
      <div
        style={{
          ...context.styles.imageSurface,
          ...(aspectRatio ? { aspectRatio } : {}),
        }}
      >
        <img src={uri} alt={alt ?? ""} style={context.styles.image} onLoad={handleLoad} />
      </div>
    </div>
  );
}

function renderNode(
  context: RenderContext,
  node: MarkdownNode,
  ancestors: MarkdownNode[],
): ReactNode {
  const nextAncestors = [...ancestors, node];
  const children = renderChildren(context, node.children, nextAncestors);
  const last = isLastChild(node, ancestors);

  switch (node.type) {
    case "text":
      return node.content;
    case "softbreak":
      return "\n";
    case "hardbreak":
      return <br key={node.key} />;
    case "paragraph":
      return (
        <p
          key={node.key}
          style={{
            ...context.styles.paragraph,
            ...(hasAncestorType(ancestors, "list_item") ? { marginBottom: 0 } : {}),
            ...(last ? context.styles.paragraphLast : {}),
          }}
        >
          {children}
        </p>
      );
    case "heading1":
    case "heading2":
    case "heading3":
    case "heading4":
    case "heading5":
    case "heading6": {
      const tag = `h${node.type.slice("heading".length)}` as
        | "h1"
        | "h2"
        | "h3"
        | "h4"
        | "h5"
        | "h6";
      const headingStyle = context.styles[
        node.type as keyof ReturnType<typeof createStyles>
      ] as CSSProperties;
      return createElement(
        tag,
        { key: node.key, style: { ...context.styles.headingBase, ...headingStyle } },
        children,
      );
    }
    case "strong":
      return <strong key={node.key}>{children}</strong>;
    case "em":
      return <em key={node.key}>{children}</em>;
    case "s":
      return <s key={node.key}>{children}</s>;
    case "link": {
      const href = node.attributes.href ?? "";
      return (
        <MarkdownLink key={node.key} href={href} context={context}>
          {children}
        </MarkdownLink>
      );
    }
    case "code_inline":
      return <InlineCode key={node.key} node={node} context={context} />;
    case "code_block":
    case "fence":
      return <CodeBlock key={node.key} content={node.content} info={node.info} context={context} />;
    case "bullet_list":
      return (
        <ul key={node.key} style={context.styles.list}>
          {children}
        </ul>
      );
    case "ordered_list":
      return (
        <ol key={node.key} style={context.styles.list}>
          {children}
        </ol>
      );
    case "list_item": {
      const task = isTaskListNode(node);
      const renderedNode = task ? stripTaskPrefixFromFirstText(node, task.labelPrefix) : node;
      const renderedChildren = renderChildren(context, renderedNode.children, nextAncestors);
      const { isOrdered, marker } = getMarkdownListMarker(node, [...ancestors].reverse());
      return (
        <li key={node.key} style={context.styles.listItem}>
          {task ? (
            <span
              style={{
                ...context.styles.taskBox,
                ...(task.checked
                  ? {
                      backgroundColor: context.theme.colors.accent,
                      color: context.theme.colors.accentForeground,
                      borderColor: context.theme.colors.accent,
                    }
                  : {}),
              }}
              data-testid={task.checked ? "rich-markdown-task-checked" : "rich-markdown-task-open"}
            >
              {task.checked ? "✓" : null}
            </span>
          ) : (
            <span style={context.styles.listMarker}>{isOrdered ? marker : "•"}</span>
          )}
          <span style={context.styles.listContent}>{renderedChildren}</span>
        </li>
      );
    }
    case "blockquote":
      return (
        <blockquote key={node.key} style={context.styles.blockquote}>
          {children}
        </blockquote>
      );
    case "hr":
      return <hr key={node.key} style={context.styles.hr} />;
    case "table":
      return (
        <div
          key={node.key}
          className="paseo-rich-markdown-table-scroll"
          data-testid="rich-markdown-table-scroll"
          style={context.styles.tableScroll}
        >
          <table style={context.styles.table}>{children}</table>
        </div>
      );
    case "thead":
      return <thead key={node.key}>{children}</thead>;
    case "tbody":
      return <tbody key={node.key}>{children}</tbody>;
    case "tr":
      return <tr key={node.key}>{children}</tr>;
    case "th":
      return (
        <th key={node.key} style={context.styles.th}>
          {children}
        </th>
      );
    case "td":
      return (
        <td key={node.key} style={context.styles.td}>
          {children}
        </td>
      );
    case "image":
      return (
        <AssistantImage
          key={node.key}
          source={node.attributes.src ?? ""}
          alt={node.attributes.alt}
          context={context}
        />
      );
    case MARKDOWN_MATH_INLINE_TOKEN:
      return (
        <MathNode key={node.key} content={node.content} displayMode={false} context={context} />
      );
    case MARKDOWN_MATH_BLOCK_TOKEN:
      return <MathNode key={node.key} content={node.content} displayMode context={context} />;
    default:
      return <span key={node.key}>{children}</span>;
  }
}

export function RichMarkdown({
  text,
  variant,
  onLinkPress,
  onInlinePathPress,
  workspaceRoot,
  serverId,
  client,
  fallback,
}: RichMarkdownProps) {
  const { theme } = useUnistyles();
  const parser = useMemo(() => createMarkdownParser(), []);
  const isElectron = getIsElectron();

  useEffect(() => {
    if (isElectron) {
      ensureRichMarkdownGlobalStyle();
    }
  }, [isElectron]);

  const nodes = useMemo(
    () => (isElectron ? tokensToNodes(parser.parse(text, MARKDOWN_ENV)) : []),
    [isElectron, parser, text],
  );
  const styles = useMemo(() => createStyles(theme, variant), [theme, variant]);
  const context = useMemo<RenderContext>(
    () => ({
      styles,
      theme,
      parser,
      onLinkPress: (url: string) => {
        const fileTarget = onInlinePathPress
          ? parseAssistantFileLink(url, { workspaceRoot })
          : null;
        if (fileTarget) {
          onInlinePathPress?.(fileTarget);
          return false;
        }
        return onLinkPress?.(url);
      },
      onInlinePathPress,
      workspaceRoot,
      serverId,
      client,
    }),
    [client, onInlinePathPress, onLinkPress, parser, serverId, styles, theme, workspaceRoot],
  );

  if (!isElectron) {
    return <>{fallback}</>;
  }

  return (
    <div className="paseo-rich-markdown" data-testid="rich-markdown" style={styles.root}>
      {nodes.map((node) => renderNode(context, node, []))}
    </div>
  );
}

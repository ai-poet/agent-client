import { useState, useCallback, useEffect, useMemo, useRef, memo, type ReactElement } from "react";
import { useRouter } from "expo-router";
import { DiffStat } from "@/components/diff-stat";
import {
  View,
  Text,
  TextInput,
  ActivityIndicator,
  Pressable,
  FlatList,
  type LayoutChangeEvent,
  type NativeSyntheticEvent,
  type NativeScrollEvent,
  TextStyle,
  type TextInput as TextInputInstance,
} from "react-native";
import AsyncStorage from "@react-native-async-storage/async-storage";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { useIsCompactFormFactor } from "@/constants/layout";
import {
  AlignJustify,
  Archive,
  CheckSquare,
  ChevronDown,
  Columns2,
  Download,
  GitBranch,
  GitCommitHorizontal,
  GitMerge,
  ListChevronsDownUp,
  ListChevronsUpDown,
  Pilcrow,
  RefreshCcw,
  Square,
  Upload,
  WrapText,
} from "lucide-react-native";
import { useCheckoutGitActionsStore } from "@/stores/checkout-git-actions-store";
import {
  useCheckoutDiffQuery,
  type ParsedDiffFile,
  type DiffLine,
  type HighlightToken,
} from "@/hooks/use-checkout-diff-query";
import { useCheckoutStatusQuery } from "@/hooks/use-checkout-status-query";
import { useCheckoutPrStatusQuery } from "@/hooks/use-checkout-pr-status-query";
import { useChangesPreferences } from "@/hooks/use-changes-preferences";
import { DiffScroll } from "./diff-scroll";
import {
  darkHighlightColors,
  lightHighlightColors,
  type HighlightStyle as HighlightStyleKey,
} from "@getpaseo/highlight";
import { WORKSPACE_SECONDARY_HEADER_HEIGHT } from "@/constants/layout";
import { Fonts } from "@/constants/theme";
import { shouldAnchorHeaderBeforeCollapse } from "@/utils/git-diff-scroll";
import {
  buildSplitDiffRows,
  buildUnifiedDiffLines,
  type SplitDiffDisplayLine,
  type SplitDiffRow,
} from "@/utils/diff-layout";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipTrigger, TooltipContent } from "@/components/ui/tooltip";
import { GitHubIcon } from "@/components/icons/github-icon";
import { buildGitActions, type GitActions } from "@/components/git-actions-policy";
import { lineNumberGutterWidth } from "@/components/code-insets";
import { useWebScrollViewScrollbar } from "@/components/use-web-scrollbar";
import { buildNewAgentRoute, resolveNewAgentWorkingDir } from "@/utils/new-agent-routing";
import { openExternalUrl } from "@/utils/open-external-url";
import { GitActionsSplitButton } from "@/components/git-actions-split-button";
import { Button } from "@/components/ui/button";
import { usePanelStore } from "@/stores/panel-store";
import { buildExplorerCheckoutKey } from "@/stores/explorer-tab-memory";
import { buildWorkspaceExplorerStateKey } from "@/hooks/use-file-explorer-actions";
import { useToast } from "@/contexts/toast-context";
import { useGitCommitDialog, type GitCommitDialogFile } from "@/contexts/git-commit-dialog-context";
import { useAppLocale } from "@/hooks/use-app-locale";
import { getAppMessages } from "@/i18n/sub2api";
import {
  formatDiffContentText,
  formatDiffGutterText,
  hasVisibleDiffTokens,
} from "@/utils/diff-rendering";
import { isWeb, isNative } from "@/constants/platform";

export type { GitActionId, GitAction, GitActions } from "@/components/git-actions-policy";

function openURLInNewTab(url: string): void {
  void openExternalUrl(url);
}

type HighlightStyle = NonNullable<HighlightToken["style"]>;

interface HighlightedTextProps {
  tokens: HighlightToken[];
  wrapLines?: boolean;
}

type WrappedWebTextStyle = TextStyle & {
  whiteSpace?: "pre" | "pre-wrap";
  overflowWrap?: "normal" | "anywhere";
};

function getWrappedTextStyle(wrapLines: boolean): WrappedWebTextStyle | undefined {
  if (isNative) {
    return undefined;
  }
  return wrapLines
    ? { whiteSpace: "pre-wrap", overflowWrap: "anywhere" }
    : { whiteSpace: "pre", overflowWrap: "normal" };
}

function HighlightedText({ tokens, wrapLines = false }: HighlightedTextProps) {
  const { theme } = useUnistyles();
  const isDark = theme.colorScheme === "dark";
  const lineHeight = theme.lineHeight.diff;

  const getTokenColor = (style: HighlightStyle | null): string => {
    const baseColor = isDark ? "#c9d1d9" : "#24292f";
    if (!style) return baseColor;
    const colors = isDark ? darkHighlightColors : lightHighlightColors;
    return colors[style as HighlightStyleKey] ?? baseColor;
  };

  return (
    <Text style={[styles.diffLineText, { lineHeight, ...getWrappedTextStyle(wrapLines) }]}>
      {tokens.map((token, index) => (
        <Text key={index} style={{ color: getTokenColor(token.style), lineHeight }}>
          {token.text}
        </Text>
      ))}
    </Text>
  );
}

interface DiffFileSectionProps {
  file: ParsedDiffFile;
  isExpanded: boolean;
  onToggle: (path: string) => void;
  selectable?: boolean;
  selected?: boolean;
  onToggleSelected?: (path: string) => void;
  onHeaderHeightChange?: (path: string, height: number) => void;
  testID?: string;
}

function lineTypeBackground(type: DiffLine["type"] | undefined | null) {
  if (!type) return styles.emptySplitCell;
  if (type === "add") return styles.addLineContainer;
  if (type === "remove") return styles.removeLineContainer;
  if (type === "header") return styles.headerLineContainer;
  return styles.contextLineContainer;
}

function DiffGutterCell({
  lineNumber,
  type,
  gutterWidth,
}: {
  lineNumber: number | null;
  type: DiffLine["type"] | undefined | null;
  gutterWidth: number;
}) {
  return (
    <View style={[styles.gutterCell, lineTypeBackground(type), { width: gutterWidth }]}>
      <Text
        style={[
          styles.lineNumberText,
          type === "add" && styles.addLineNumberText,
          type === "remove" && styles.removeLineNumberText,
        ]}
      >
        {formatDiffGutterText(lineNumber)}
      </Text>
    </View>
  );
}

function DiffTextLine({ line, wrapLines }: { line: DiffLine; wrapLines: boolean }) {
  const visibleTokens = hasVisibleDiffTokens(line.tokens) ? line.tokens : null;

  return (
    <View style={[styles.textLineContainer, lineTypeBackground(line.type)]}>
      {line.type !== "header" && visibleTokens ? (
        <HighlightedText tokens={visibleTokens} wrapLines={wrapLines} />
      ) : (
        <Text
          style={[
            styles.diffLineText,
            getWrappedTextStyle(wrapLines),
            line.type === "add" && styles.addLineText,
            line.type === "remove" && styles.removeLineText,
            line.type === "header" && styles.headerLineText,
            line.type === "context" && styles.contextLineText,
          ]}
        >
          {formatDiffContentText(line.content)}
        </Text>
      )}
    </View>
  );
}

function SplitTextLine({
  line,
  wrapLines,
}: {
  line: SplitDiffDisplayLine | null;
  wrapLines: boolean;
}) {
  const visibleTokens = line && hasVisibleDiffTokens(line.tokens) ? line.tokens : null;

  return (
    <View style={[styles.textLineContainer, lineTypeBackground(line?.type)]}>
      {visibleTokens ? (
        <HighlightedText tokens={visibleTokens} wrapLines={wrapLines} />
      ) : (
        <Text
          style={[
            styles.diffLineText,
            getWrappedTextStyle(wrapLines),
            line?.type === "add" && styles.addLineText,
            line?.type === "remove" && styles.removeLineText,
            line?.type === "context" && styles.contextLineText,
            !line && styles.emptySplitCellText,
          ]}
        >
          {formatDiffContentText(line?.content)}
        </Text>
      )}
    </View>
  );
}

function DiffLineView({
  line,
  lineNumber,
  gutterWidth,
  wrapLines,
}: {
  line: DiffLine;
  lineNumber: number | null;
  gutterWidth: number;
  wrapLines: boolean;
}) {
  const visibleTokens = hasVisibleDiffTokens(line.tokens) ? line.tokens : null;

  return (
    <View style={[styles.diffLineContainer, lineTypeBackground(line.type)]}>
      <View style={[styles.lineNumberGutter, { width: gutterWidth }]}>
        <Text
          style={[
            styles.lineNumberText,
            line.type === "add" && styles.addLineNumberText,
            line.type === "remove" && styles.removeLineNumberText,
          ]}
        >
          {formatDiffGutterText(lineNumber)}
        </Text>
      </View>
      {line.type !== "header" && visibleTokens ? (
        <HighlightedText tokens={visibleTokens} wrapLines={wrapLines} />
      ) : (
        <Text
          style={[
            styles.diffLineText,
            getWrappedTextStyle(wrapLines),
            line.type === "add" && styles.addLineText,
            line.type === "remove" && styles.removeLineText,
            line.type === "header" && styles.headerLineText,
            line.type === "context" && styles.contextLineText,
          ]}
        >
          {formatDiffContentText(line.content)}
        </Text>
      )}
    </View>
  );
}

function SplitDiffLine({
  line,
  gutterWidth,
  wrapLines,
}: {
  line: SplitDiffDisplayLine | null;
  gutterWidth: number;
  wrapLines: boolean;
}) {
  const visibleTokens = line && hasVisibleDiffTokens(line.tokens) ? line.tokens : null;

  return (
    <View style={[styles.diffLineContainer, lineTypeBackground(line?.type)]}>
      <View style={[styles.lineNumberGutter, { width: gutterWidth }]}>
        <Text
          style={[
            styles.lineNumberText,
            line?.type === "add" && styles.addLineNumberText,
            line?.type === "remove" && styles.removeLineNumberText,
          ]}
        >
          {formatDiffGutterText(line?.lineNumber ?? null)}
        </Text>
      </View>
      {visibleTokens ? (
        <HighlightedText tokens={visibleTokens} wrapLines={wrapLines} />
      ) : (
        <Text
          style={[
            styles.diffLineText,
            getWrappedTextStyle(wrapLines),
            line?.type === "add" && styles.addLineText,
            line?.type === "remove" && styles.removeLineText,
            line?.type === "context" && styles.contextLineText,
            !line && styles.emptySplitCellText,
          ]}
        >
          {formatDiffContentText(line?.content)}
        </Text>
      )}
    </View>
  );
}

function SplitDiffColumn({
  rows,
  side,
  gutterWidth,
  wrapLines,
  showDivider = false,
}: {
  rows: SplitDiffRow[];
  side: "left" | "right";
  gutterWidth: number;
  wrapLines: boolean;
  showDivider?: boolean;
}) {
  const [scrollWidth, setScrollWidth] = useState(0);

  if (wrapLines) {
    return (
      <View style={[styles.splitCell, showDivider && styles.splitCellWithDivider]}>
        <View style={styles.linesContainer}>
          {rows.map((row, i) => {
            if (row.kind === "header") {
              return (
                <View key={`header-${i}`} style={styles.splitHeaderRow}>
                  <Text style={[styles.diffLineText, styles.headerLineText]}>{row.content}</Text>
                </View>
              );
            }
            return (
              <SplitDiffLine
                key={`line-${i}`}
                line={side === "left" ? row.left : row.right}
                gutterWidth={gutterWidth}
                wrapLines={wrapLines}
              />
            );
          })}
        </View>
      </View>
    );
  }

  return (
    <View
      style={[styles.splitCell, showDivider && styles.splitCellWithDivider, styles.splitCellRow]}
    >
      <View style={styles.gutterColumn}>
        {rows.map((row, i) => {
          if (row.kind === "header") {
            return (
              <DiffGutterCell
                key={`g-${i}`}
                lineNumber={null}
                type="header"
                gutterWidth={gutterWidth}
              />
            );
          }
          const line = side === "left" ? row.left : row.right;
          return (
            <DiffGutterCell
              key={`g-${i}`}
              lineNumber={line?.lineNumber ?? null}
              type={line?.type}
              gutterWidth={gutterWidth}
            />
          );
        })}
      </View>
      <DiffScroll
        scrollViewWidth={scrollWidth}
        onScrollViewWidthChange={setScrollWidth}
        style={styles.splitColumnScroll}
        contentContainerStyle={styles.diffContentInner}
      >
        <View style={[styles.linesContainer, scrollWidth > 0 && { minWidth: scrollWidth }]}>
          {rows.map((row, i) => {
            if (row.kind === "header") {
              return (
                <View key={`t-${i}`} style={styles.splitHeaderRow}>
                  <Text style={[styles.diffLineText, styles.headerLineText]}>{row.content}</Text>
                </View>
              );
            }
            return (
              <SplitTextLine
                key={`t-${i}`}
                line={side === "left" ? row.left : row.right}
                wrapLines={false}
              />
            );
          })}
        </View>
      </DiffScroll>
    </View>
  );
}

const DiffFileHeader = memo(function DiffFileHeader({
  file,
  isExpanded,
  onToggle,
  selectable = false,
  selected = false,
  onToggleSelected,
  onHeaderHeightChange,
  testID,
}: DiffFileSectionProps) {
  const { theme } = useUnistyles();
  const layoutYRef = useRef<number | null>(null);
  const pressHandledRef = useRef(false);
  const pressInRef = useRef<{ ts: number; pageX: number; pageY: number } | null>(null);

  const toggleExpanded = useCallback(() => {
    pressHandledRef.current = true;
    onToggle(file.path);
  }, [file.path, onToggle]);

  return (
    <View
      style={[styles.fileSectionHeaderContainer, isExpanded && styles.fileSectionHeaderExpanded]}
      onLayout={(event) => {
        layoutYRef.current = event.nativeEvent.layout.y;
        onHeaderHeightChange?.(file.path, event.nativeEvent.layout.height);
      }}
      testID={testID}
    >
      <Pressable
        testID={testID ? `${testID}-toggle` : undefined}
        style={({ pressed }) => [styles.fileHeader, pressed && styles.fileHeaderPressed]}
        // Android: prevent parent pan/scroll gestures from canceling the tap release.
        cancelable={false}
        onPressIn={(event) => {
          pressHandledRef.current = false;
          pressInRef.current = {
            ts: Date.now(),
            pageX: event.nativeEvent.pageX,
            pageY: event.nativeEvent.pageY,
          };
        }}
        onPressOut={(event) => {
          if (
            isNative &&
            !pressHandledRef.current &&
            layoutYRef.current === 0 &&
            pressInRef.current
          ) {
            const durationMs = Date.now() - pressInRef.current.ts;
            const dx = event.nativeEvent.pageX - pressInRef.current.pageX;
            const dy = event.nativeEvent.pageY - pressInRef.current.pageY;
            const distance = Math.hypot(dx, dy);
            // Sticky headers on Android can emit pressIn/pressOut without onPress.
            // Treat short, low-movement interactions as taps.
            if (durationMs <= 500 && distance <= 12) {
              toggleExpanded();
            }
          }
        }}
        onPress={toggleExpanded}
      >
        {selectable ? (
          <Pressable
            accessibilityRole="checkbox"
            accessibilityState={{ checked: selected }}
            testID={testID ? `${testID}-select` : undefined}
            hitSlop={8}
            onPress={(event) => {
              event.stopPropagation();
              onToggleSelected?.(file.path);
            }}
            style={styles.fileSelectButton}
          >
            {selected ? (
              <CheckSquare size={16} color={theme.colors.accent} />
            ) : (
              <Square size={16} color={theme.colors.foregroundMuted} />
            )}
          </Pressable>
        ) : null}
        <View style={styles.fileHeaderLeft}>
          <Text style={styles.fileName}>{file.path.split("/").pop()}</Text>
          <Text style={styles.fileDir} numberOfLines={1}>
            {file.path.includes("/") ? ` ${file.path.slice(0, file.path.lastIndexOf("/"))}` : ""}
          </Text>
          {file.isNew && (
            <View style={styles.newBadge}>
              <Text style={styles.newBadgeText}>New</Text>
            </View>
          )}
          {file.isDeleted && (
            <View style={styles.deletedBadge}>
              <Text style={styles.deletedBadgeText}>Deleted</Text>
            </View>
          )}
        </View>
        <View style={styles.fileHeaderRight}>
          <DiffStat additions={file.additions} deletions={file.deletions} />
        </View>
      </Pressable>
    </View>
  );
});

function DiffFileBody({
  file,
  layout,
  wrapLines,
  onBodyHeightChange,
  testID,
  text,
}: {
  file: ParsedDiffFile;
  layout: "unified" | "split";
  wrapLines: boolean;
  onBodyHeightChange?: (path: string, height: number) => void;
  testID?: string;
  text: ReturnType<typeof getAppMessages>["gitDiff"];
}) {
  const [scrollViewWidth, setScrollViewWidth] = useState(0);
  const [bodyWidth, setBodyWidth] = useState(0);

  return (
    <View
      style={[styles.fileSectionBodyContainer, styles.fileSectionBorder]}
      onLayout={(event) => {
        setBodyWidth(event.nativeEvent.layout.width);
        onBodyHeightChange?.(file.path, event.nativeEvent.layout.height);
      }}
      testID={testID}
    >
      {(() => {
        if (file.status === "too_large" || file.status === "binary") {
          return (
            <View style={styles.statusMessageContainer}>
              <Text style={styles.statusMessageText}>
                {file.status === "binary" ? text.binaryFile : text.tooLarge}
              </Text>
            </View>
          );
        }

        let maxLineNo = 0;
        for (const hunk of file.hunks) {
          maxLineNo = Math.max(
            maxLineNo,
            hunk.oldStart + hunk.oldCount,
            hunk.newStart + hunk.newCount,
          );
        }
        const gutterWidth = lineNumberGutterWidth(maxLineNo);

        if (layout === "split") {
          const rows = buildSplitDiffRows(file);
          return (
            <View style={[styles.diffContent, styles.splitRow]}>
              <SplitDiffColumn
                rows={rows}
                side="left"
                gutterWidth={gutterWidth}
                wrapLines={wrapLines}
              />
              <SplitDiffColumn
                rows={rows}
                side="right"
                gutterWidth={gutterWidth}
                wrapLines={wrapLines}
                showDivider
              />
            </View>
          );
        }

        const computedLines = buildUnifiedDiffLines(file);

        if (wrapLines) {
          return (
            <View style={styles.diffContent}>
              <View style={styles.linesContainer}>
                {computedLines.map(({ line, lineNumber, key }) => (
                  <DiffLineView
                    key={key}
                    line={line}
                    lineNumber={lineNumber}
                    gutterWidth={gutterWidth}
                    wrapLines={wrapLines}
                  />
                ))}
              </View>
            </View>
          );
        }

        const availableWidth = bodyWidth > 0 ? bodyWidth : scrollViewWidth;
        return (
          <View style={[styles.diffContent, styles.diffContentRow]}>
            <View style={styles.gutterColumn}>
              {computedLines.map(({ line, lineNumber, key }) => (
                <DiffGutterCell
                  key={key}
                  lineNumber={lineNumber}
                  type={line.type}
                  gutterWidth={gutterWidth}
                />
              ))}
            </View>
            <DiffScroll
              scrollViewWidth={scrollViewWidth}
              onScrollViewWidthChange={setScrollViewWidth}
              style={styles.splitColumnScroll}
              contentContainerStyle={styles.diffContentInner}
            >
              <View
                style={[styles.linesContainer, availableWidth > 0 && { minWidth: availableWidth }]}
              >
                {computedLines.map(({ line, key }) => (
                  <DiffTextLine key={key} line={line} wrapLines={false} />
                ))}
              </View>
            </DiffScroll>
          </View>
        );
      })()}
    </View>
  );
}

interface GitDiffPaneProps {
  serverId: string;
  workspaceId?: string | null;
  cwd: string;
  hideHeaderRow?: boolean;
}

type DiffFlatItem =
  | { type: "header"; file: ParsedDiffFile; fileIndex: number; isExpanded: boolean }
  | { type: "body"; file: ParsedDiffFile; fileIndex: number };

export function GitDiffPane({ serverId, workspaceId, cwd, hideHeaderRow }: GitDiffPaneProps) {
  const { theme } = useUnistyles();
  const toast = useToast();
  const { openCommitDialog } = useGitCommitDialog();
  const locale = useAppLocale();
  const text = useMemo(() => getAppMessages(locale).gitDiff, [locale]);
  const isMobile = useIsCompactFormFactor();
  const showDesktopWebScrollbar = isWeb && !isMobile;
  const canUseSplitLayout = isWeb && !isMobile;
  const router = useRouter();
  const [diffModeOverride, setDiffModeOverride] = useState<"uncommitted" | "base" | null>(null);
  const [postShipArchiveSuggested, setPostShipArchiveSuggested] = useState(false);
  const [shipDefault, setShipDefault] = useState<"merge" | "pr">("merge");
  const [commitMessage, setCommitMessage] = useState("");
  const [commitError, setCommitError] = useState<string | null>(null);
  const [selectedCommitPaths, setSelectedCommitPaths] = useState<Set<string>>(() => new Set());
  const commitInputRef = useRef<TextInputInstance>(null);
  const { preferences: changesPreferences, updatePreferences: updateChangesPreferences } =
    useChangesPreferences();
  const wrapLines = changesPreferences.wrapLines;
  const effectiveLayout = canUseSplitLayout ? changesPreferences.layout : "unified";

  const handleToggleWrapLines = useCallback(() => {
    void updateChangesPreferences({ wrapLines: !wrapLines });
  }, [updateChangesPreferences, wrapLines]);

  const handleLayoutChange = useCallback(
    (nextLayout: "unified" | "split") => {
      void updateChangesPreferences({ layout: nextLayout });
    },
    [updateChangesPreferences],
  );

  const handleToggleHideWhitespace = useCallback(() => {
    void updateChangesPreferences({ hideWhitespace: !changesPreferences.hideWhitespace });
  }, [changesPreferences.hideWhitespace, updateChangesPreferences]);

  const {
    status,
    isLoading: isStatusLoading,
    isError: isStatusError,
    error: statusError,
  } = useCheckoutStatusQuery({ serverId, cwd });
  const gitStatus = status && status.isGit ? status : null;
  const isGit = Boolean(gitStatus);
  const notGit = status !== null && !status.isGit && !status.error;
  const statusErrorMessage =
    status?.error?.message ??
    (isStatusError && statusError instanceof Error ? statusError.message : null);
  const baseRef = gitStatus?.baseRef ?? undefined;

  // Auto-select diff mode based on state: uncommitted when dirty, base when clean
  const hasUncommittedChanges = Boolean(gitStatus?.isDirty);
  const autoDiffMode = hasUncommittedChanges ? "uncommitted" : "base";
  const diffMode = diffModeOverride ?? autoDiffMode;

  const {
    files,
    payloadError: diffPayloadError,
    isLoading: isDiffLoading,
  } = useCheckoutDiffQuery({
    serverId,
    cwd,
    mode: diffMode,
    baseRef,
    ignoreWhitespace: changesPreferences.hideWhitespace,
    enabled: isGit,
  });
  const {
    status: prStatus,
    githubFeaturesEnabled,
    payloadError: prPayloadError,
  } = useCheckoutPrStatusQuery({
    serverId,
    cwd,
    enabled: isGit,
  });
  const normalizedWorkspaceRoot = useMemo(() => cwd.trim(), [cwd]);
  const workspaceStateKey = useMemo(
    () =>
      buildWorkspaceExplorerStateKey({
        workspaceId,
        workspaceRoot: normalizedWorkspaceRoot,
      }),
    [normalizedWorkspaceRoot, workspaceId],
  );
  const commitFocusKey = useMemo(() => buildExplorerCheckoutKey(serverId, cwd), [cwd, serverId]);
  const expandedPathsArray = usePanelStore((state) =>
    workspaceStateKey ? state.diffExpandedPathsByWorkspace[workspaceStateKey] : undefined,
  );
  const setDiffExpandedPathsForWorkspace = usePanelStore(
    (state) => state.setDiffExpandedPathsForWorkspace,
  );
  const expandedPaths = useMemo(() => new Set(expandedPathsArray ?? []), [expandedPathsArray]);
  const diffListRef = useRef<FlatList<DiffFlatItem>>(null);
  const scrollbar = useWebScrollViewScrollbar(diffListRef, {
    enabled: showDesktopWebScrollbar,
  });
  const diffListScrollOffsetRef = useRef(0);
  const diffListViewportHeightRef = useRef(0);
  const headerHeightByPathRef = useRef<Record<string, number>>({});
  const bodyHeightByPathRef = useRef<Record<string, number>>({});
  const defaultHeaderHeightRef = useRef<number>(44);
  const shipDefaultStorageKey = useMemo(() => {
    if (!gitStatus?.repoRoot) {
      return null;
    }
    return `@paseo:changes-ship-default:${gitStatus.repoRoot}`;
  }, [gitStatus?.repoRoot]);

  useEffect(() => {
    if (!shipDefaultStorageKey) {
      return;
    }
    let isActive = true;
    AsyncStorage.getItem(shipDefaultStorageKey)
      .then((value) => {
        if (!isActive) return;
        if (value === "pr" || value === "merge") {
          setShipDefault(value);
        }
      })
      .catch(() => undefined);
    return () => {
      isActive = false;
    };
  }, [shipDefaultStorageKey]);

  const persistShipDefault = useCallback(
    async (next: "merge" | "pr") => {
      setShipDefault(next);
      if (!shipDefaultStorageKey) return;
      try {
        await AsyncStorage.setItem(shipDefaultStorageKey, next);
      } catch {
        // Ignore persistence failures; default will reset to "merge".
      }
    },
    [shipDefaultStorageKey],
  );

  const { flatItems, stickyHeaderIndices } = useMemo(() => {
    const items: DiffFlatItem[] = [];
    const stickyIndices: number[] = [];
    for (let i = 0; i < files.length; i++) {
      const file = files[i];
      const isExpanded = expandedPaths.has(file.path);
      items.push({ type: "header", file, fileIndex: i, isExpanded });
      if (isExpanded) {
        stickyIndices.push(items.length - 1);
      }
      if (isExpanded) {
        items.push({ type: "body", file, fileIndex: i });
      }
    }
    return { flatItems: items, stickyHeaderIndices: stickyIndices };
  }, [expandedPaths, files]);

  const handleHeaderHeightChange = useCallback((path: string, height: number) => {
    if (!Number.isFinite(height) || height <= 0) {
      return;
    }
    headerHeightByPathRef.current[path] = height;
    defaultHeaderHeightRef.current = height;
  }, []);

  const handleBodyHeightChange = useCallback((path: string, height: number) => {
    if (!Number.isFinite(height) || height < 0) {
      return;
    }
    bodyHeightByPathRef.current[path] = height;
  }, []);

  const handleDiffListScroll = useCallback(
    (event: NativeSyntheticEvent<NativeScrollEvent>) => {
      diffListScrollOffsetRef.current = event.nativeEvent.contentOffset.y;
      scrollbar.onScroll(event);
    },
    [scrollbar.onScroll],
  );

  const handleDiffListLayout = useCallback(
    (event: LayoutChangeEvent) => {
      const height = event.nativeEvent.layout.height;
      if (!Number.isFinite(height) || height <= 0) {
        return;
      }
      diffListViewportHeightRef.current = height;
      scrollbar.onLayout(event);
    },
    [scrollbar.onLayout],
  );

  const computeHeaderOffset = useCallback(
    (path: string): number => {
      const defaultHeaderHeight = defaultHeaderHeightRef.current;
      let offset = 0;
      for (const file of files) {
        if (file.path === path) {
          break;
        }
        offset += headerHeightByPathRef.current[file.path] ?? defaultHeaderHeight;
        if (expandedPaths.has(file.path)) {
          offset += bodyHeightByPathRef.current[file.path] ?? 0;
        }
      }
      return Math.max(0, offset);
    },
    [expandedPaths, files],
  );

  const handleToggleExpanded = useCallback(
    (path: string) => {
      if (!workspaceStateKey) {
        return;
      }
      const isCurrentlyExpanded = expandedPaths.has(path);
      const nextExpanded = !isCurrentlyExpanded;
      const targetOffset = isCurrentlyExpanded ? computeHeaderOffset(path) : null;
      const headerHeight = headerHeightByPathRef.current[path] ?? defaultHeaderHeightRef.current;
      const shouldAnchor =
        isCurrentlyExpanded &&
        targetOffset !== null &&
        shouldAnchorHeaderBeforeCollapse({
          headerOffset: targetOffset,
          headerHeight,
          viewportOffset: diffListScrollOffsetRef.current,
          viewportHeight: diffListViewportHeightRef.current,
        });

      // Anchor to the clicked header before collapsing so visual context is preserved.
      if (shouldAnchor && targetOffset !== null) {
        diffListRef.current?.scrollToOffset({
          offset: targetOffset,
          animated: false,
        });
      }

      const nextPaths = nextExpanded
        ? [...expandedPaths, path]
        : Array.from(expandedPaths).filter((expandedPath) => expandedPath !== path);
      setDiffExpandedPathsForWorkspace(workspaceStateKey, nextPaths);
    },
    [computeHeaderOffset, expandedPaths, setDiffExpandedPathsForWorkspace, workspaceStateKey],
  );

  const allExpanded = useMemo(() => {
    if (files.length === 0) return false;
    return files.every((file) => expandedPaths.has(file.path));
  }, [expandedPaths, files]);

  const handleToggleExpandAll = useCallback(() => {
    if (!workspaceStateKey) {
      return;
    }
    if (allExpanded) {
      setDiffExpandedPathsForWorkspace(workspaceStateKey, []);
    } else {
      setDiffExpandedPathsForWorkspace(
        workspaceStateKey,
        files.map((file) => file.path),
      );
    }
  }, [allExpanded, files, setDiffExpandedPathsForWorkspace, workspaceStateKey]);

  const commitFilePaths = useMemo(
    () => (diffMode === "uncommitted" ? files.map((file) => file.path) : []),
    [diffMode, files],
  );
  const commitDialogFiles = useMemo<GitCommitDialogFile[]>(() => {
    if (diffMode !== "uncommitted") {
      return [];
    }
    return files.map((file) => ({
      path: file.path,
      additions: file.additions,
      deletions: file.deletions,
      isNew: file.isNew,
      isDeleted: file.isDeleted,
      status: file.status,
    }));
  }, [diffMode, files]);
  const selectedCommitCount = selectedCommitPaths.size;
  const allCommitFilesSelected =
    commitFilePaths.length > 0 && selectedCommitCount === commitFilePaths.length;

  useEffect(() => {
    setSelectedCommitPaths((current) => {
      const availablePaths = new Set(commitFilePaths);
      const next = new Set<string>();
      let changed = current.size !== availablePaths.size;
      for (const path of commitFilePaths) {
        if (current.has(path) || current.size === 0) {
          next.add(path);
        }
        if (!current.has(path)) {
          changed = true;
        }
      }
      for (const path of current) {
        if (!availablePaths.has(path)) {
          changed = true;
          break;
        }
      }
      return changed ? next : current;
    });
  }, [commitFilePaths]);

  useEffect(() => {
    if (diffMode !== "uncommitted") {
      setCommitError(null);
    }
  }, [diffMode]);

  // Clear diff mode override when auto mode changes (e.g., after commit)
  useEffect(() => {
    setDiffModeOverride(null);
  }, [autoDiffMode]);

  const commitFocusRequest = usePanelStore((state) => {
    if (!commitFocusKey) {
      return undefined;
    }
    return state.commitFocusRequestByCheckout[commitFocusKey];
  });

  useEffect(() => {
    if (!commitFocusRequest) {
      return;
    }
    const timeout = setTimeout(() => commitInputRef.current?.focus(), 0);
    return () => clearTimeout(timeout);
  }, [commitFocusRequest]);

  const commitStatus = useCheckoutGitActionsStore((state) =>
    state.getStatus({ serverId, cwd, actionId: "commit" }),
  );
  const pullStatus = useCheckoutGitActionsStore((state) =>
    state.getStatus({ serverId, cwd, actionId: "pull" }),
  );
  const pushStatus = useCheckoutGitActionsStore((state) =>
    state.getStatus({ serverId, cwd, actionId: "push" }),
  );
  const prCreateStatus = useCheckoutGitActionsStore((state) =>
    state.getStatus({ serverId, cwd, actionId: "create-pr" }),
  );
  const mergeStatus = useCheckoutGitActionsStore((state) =>
    state.getStatus({ serverId, cwd, actionId: "merge-branch" }),
  );
  const mergeFromBaseStatus = useCheckoutGitActionsStore((state) =>
    state.getStatus({ serverId, cwd, actionId: "merge-from-base" }),
  );
  const archiveStatus = useCheckoutGitActionsStore((state) =>
    state.getStatus({ serverId, cwd, actionId: "archive-worktree" }),
  );

  const runCommit = useCheckoutGitActionsStore((state) => state.commit);
  const runPull = useCheckoutGitActionsStore((state) => state.pull);
  const runPush = useCheckoutGitActionsStore((state) => state.push);
  const runCreatePr = useCheckoutGitActionsStore((state) => state.createPr);
  const runMergeBranch = useCheckoutGitActionsStore((state) => state.mergeBranch);
  const runMergeFromBase = useCheckoutGitActionsStore((state) => state.mergeFromBase);
  const runArchiveWorktree = useCheckoutGitActionsStore((state) => state.archiveWorktree);

  const toastActionError = useCallback(
    (error: unknown, fallback: string) => {
      const message = error instanceof Error ? error.message : fallback;
      toast.error(message);
    },
    [toast],
  );

  const toastActionSuccess = useCallback(
    (message: string) => {
      toast.show(message, { variant: "success" });
    },
    [toast],
  );

  const toggleCommitPath = useCallback((path: string) => {
    setSelectedCommitPaths((current) => {
      const next = new Set(current);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
    setCommitError(null);
  }, []);

  const selectAllCommitFiles = useCallback(() => {
    setSelectedCommitPaths(new Set(commitFilePaths));
    setCommitError(null);
  }, [commitFilePaths]);

  const clearCommitFiles = useCallback(() => {
    setSelectedCommitPaths(new Set());
    setCommitError(null);
  }, []);

  const handleCommitMessageChange = useCallback((next: string) => {
    setCommitMessage(next);
    setCommitError(null);
  }, []);

  const handleCommit = useCallback(() => {
    if (isMobile) {
      openCommitDialog({
        serverId,
        cwd,
        files: commitDialogFiles,
        onCommit: async (message, options) => {
          await runCommit({
            serverId,
            cwd,
            message,
            addAll: options?.addAll ?? true,
            paths: options?.paths,
          });
          toastActionSuccess(text.committedToast);
        },
      });
      return;
    }

    const message = commitMessage.trim();
    if (!message) {
      setCommitError(text.commitMessageRequired);
      commitInputRef.current?.focus();
      return;
    }
    const paths = commitFilePaths.filter((path) => selectedCommitPaths.has(path));
    if (paths.length === 0) {
      setCommitError(text.commitFilesRequired);
      return;
    }
    void runCommit({ serverId, cwd, message, addAll: false, paths })
      .then(() => {
        setCommitMessage("");
        setCommitError(null);
        toastActionSuccess(text.committedToast);
      })
      .catch((err) => {
        setCommitError(err instanceof Error ? err.message : text.failedCommit);
      });
  }, [
    commitFilePaths,
    commitDialogFiles,
    commitMessage,
    cwd,
    isMobile,
    openCommitDialog,
    runCommit,
    selectedCommitPaths,
    serverId,
    text.commitFilesRequired,
    text.commitMessageRequired,
    text.committedToast,
    text.failedCommit,
    toastActionSuccess,
  ]);

  const handlePull = useCallback(() => {
    void runPull({ serverId, cwd })
      .then(() => {
        toastActionSuccess(text.pulledToast);
      })
      .catch((err) => {
        toastActionError(err, text.failedPull);
      });
  }, [
    cwd,
    runPull,
    serverId,
    text.failedPull,
    text.pulledToast,
    toastActionError,
    toastActionSuccess,
  ]);

  const handlePush = useCallback(() => {
    void runPush({ serverId, cwd })
      .then(() => {
        toastActionSuccess(text.pushedToast);
      })
      .catch((err) => {
        toastActionError(err, text.failedPush);
      });
  }, [
    cwd,
    runPush,
    serverId,
    text.failedPush,
    text.pushedToast,
    toastActionError,
    toastActionSuccess,
  ]);

  const handleCreatePr = useCallback(() => {
    void persistShipDefault("pr");
    void runCreatePr({ serverId, cwd })
      .then(() => {
        toastActionSuccess(text.prCreatedToast);
      })
      .catch((err) => {
        toastActionError(err, text.failedCreatePr);
      });
  }, [
    cwd,
    persistShipDefault,
    runCreatePr,
    serverId,
    text.failedCreatePr,
    text.prCreatedToast,
    toastActionError,
    toastActionSuccess,
  ]);

  const handleMergeBranch = useCallback(() => {
    if (!baseRef) {
      toast.error(text.baseRefUnavailable);
      return;
    }
    void persistShipDefault("merge");
    void runMergeBranch({ serverId, cwd, baseRef })
      .then(() => {
        setPostShipArchiveSuggested(true);
        toastActionSuccess(text.mergedToast);
      })
      .catch((err) => {
        toastActionError(err, text.failedMerge);
      });
  }, [
    baseRef,
    cwd,
    persistShipDefault,
    runMergeBranch,
    serverId,
    text.baseRefUnavailable,
    text.failedMerge,
    text.mergedToast,
    toast,
    toastActionError,
    toastActionSuccess,
  ]);

  const handleMergeFromBase = useCallback(() => {
    if (!baseRef) {
      toast.error(text.baseRefUnavailable);
      return;
    }
    void runMergeFromBase({ serverId, cwd, baseRef })
      .then(() => {
        toastActionSuccess(text.updatedToast);
      })
      .catch((err) => {
        toastActionError(err, text.failedMergeFromBase);
      });
  }, [
    baseRef,
    cwd,
    runMergeFromBase,
    serverId,
    text.baseRefUnavailable,
    text.failedMergeFromBase,
    text.updatedToast,
    toast,
    toastActionError,
    toastActionSuccess,
  ]);

  const handleArchiveWorktree = useCallback(() => {
    const worktreePath = status?.cwd;
    if (!worktreePath) {
      toast.error(text.worktreePathUnavailable);
      return;
    }
    const targetWorkingDir = resolveNewAgentWorkingDir(cwd, status ?? null);
    void runArchiveWorktree({ serverId, cwd, worktreePath })
      .then(() => {
        router.replace(buildNewAgentRoute(serverId, targetWorkingDir));
      })
      .catch((err) => {
        toastActionError(err, text.failedArchiveWorktree);
      });
  }, [
    cwd,
    router,
    runArchiveWorktree,
    serverId,
    status,
    text.failedArchiveWorktree,
    text.worktreePathUnavailable,
    toast,
    toastActionError,
  ]);

  const renderFlatItem = useCallback(
    ({ item }: { item: DiffFlatItem }) => {
      if (item.type === "header") {
        return (
          <DiffFileHeader
            file={item.file}
            isExpanded={item.isExpanded}
            onToggle={handleToggleExpanded}
            selectable={diffMode === "uncommitted"}
            selected={selectedCommitPaths.has(item.file.path)}
            onToggleSelected={toggleCommitPath}
            onHeaderHeightChange={handleHeaderHeightChange}
            testID={`diff-file-${item.fileIndex}`}
          />
        );
      }
      return (
        <DiffFileBody
          file={item.file}
          layout={effectiveLayout}
          wrapLines={wrapLines}
          onBodyHeightChange={handleBodyHeightChange}
          testID={`diff-file-${item.fileIndex}-body`}
          text={text}
        />
      );
    },
    [
      effectiveLayout,
      diffMode,
      handleBodyHeightChange,
      handleHeaderHeightChange,
      handleToggleExpanded,
      selectedCommitPaths,
      text,
      toggleCommitPath,
      wrapLines,
    ],
  );

  const flatKeyExtractor = useCallback(
    (item: DiffFlatItem) => `${item.type}-${item.file.path}`,
    [],
  );

  const hasChanges = files.length > 0;
  const diffErrorMessage = diffPayloadError?.message ?? null;
  const prErrorMessage = githubFeaturesEnabled ? (prPayloadError?.message ?? null) : null;
  const branchLabel =
    gitStatus?.currentBranch && gitStatus.currentBranch !== "HEAD"
      ? gitStatus.currentBranch
      : notGit
        ? text.notGitRepository
        : text.unknownBranch;
  const actionsDisabled = !isGit || Boolean(status?.error) || isStatusLoading;
  const aheadCount = gitStatus?.aheadBehind?.ahead ?? 0;
  const behindBaseCount = gitStatus?.aheadBehind?.behind ?? 0;
  const aheadOfOrigin = gitStatus?.aheadOfOrigin ?? 0;
  const behindOfOrigin = gitStatus?.behindOfOrigin ?? 0;
  const baseRefLabel = useMemo(() => {
    if (!baseRef) return text.baseFallback;
    const trimmed = baseRef.replace(/^refs\/(heads|remotes)\//, "").trim();
    return trimmed.startsWith("origin/") ? trimmed.slice("origin/".length) : trimmed;
  }, [baseRef, text.baseFallback]);
  const committedDiffDescription = useMemo(() => {
    if (!branchLabel || !baseRefLabel) {
      return undefined;
    }
    return branchLabel === baseRefLabel ? undefined : `${branchLabel} -> ${baseRefLabel}`;
  }, [baseRefLabel, branchLabel]);
  const hasPullRequest = Boolean(prStatus?.url);
  const hasRemote = gitStatus?.hasRemote ?? false;
  const isPaseoOwnedWorktree = gitStatus?.isPaseoOwnedWorktree ?? false;
  const isMergedPullRequest = Boolean(prStatus?.isMerged);
  const currentBranch = gitStatus?.currentBranch;
  const isOnBaseBranch = currentBranch === baseRefLabel;
  const shouldPromoteArchive =
    isPaseoOwnedWorktree &&
    !hasUncommittedChanges &&
    (postShipArchiveSuggested || isMergedPullRequest);

  const commitDisabled = actionsDisabled || commitStatus === "pending" || (!isMobile && hasChanges);
  const pullDisabled = actionsDisabled || pullStatus === "pending";
  const prDisabled = actionsDisabled || prCreateStatus === "pending";
  const mergeDisabled = actionsDisabled || mergeStatus === "pending";
  const mergeFromBaseDisabled = actionsDisabled || mergeFromBaseStatus === "pending";
  const pushDisabled = actionsDisabled || pushStatus === "pending";
  const archiveDisabled = actionsDisabled || archiveStatus === "pending";

  let bodyContent: ReactElement;

  if (isStatusLoading) {
    bodyContent = (
      <View style={styles.loadingContainer}>
        <ActivityIndicator size="large" color={theme.colors.foregroundMuted} />
        <Text style={styles.loadingText}>{text.checkingRepository}</Text>
      </View>
    );
  } else if (statusErrorMessage) {
    bodyContent = (
      <View style={styles.errorContainer}>
        <Text style={styles.errorText}>{statusErrorMessage}</Text>
      </View>
    );
  } else if (notGit) {
    bodyContent = (
      <View style={styles.emptyContainer} testID="changes-not-git">
        <Text style={styles.emptyText}>{text.notGitRepository}</Text>
      </View>
    );
  } else if (isDiffLoading) {
    bodyContent = (
      <View style={styles.loadingContainer}>
        <ActivityIndicator size="large" color={theme.colors.foregroundMuted} />
      </View>
    );
  } else if (diffErrorMessage) {
    bodyContent = (
      <View style={styles.errorContainer}>
        <Text style={styles.errorText}>{diffErrorMessage}</Text>
      </View>
    );
  } else if (!hasChanges) {
    bodyContent = (
      <View style={styles.emptyContainer}>
        <Text style={styles.emptyText}>
          {changesPreferences.hideWhitespace
            ? text.hiddenWhitespaceEmpty
            : diffMode === "uncommitted"
              ? text.noUncommittedChanges
              : text.noChangesVs(baseRefLabel)}
        </Text>
      </View>
    );
  } else {
    bodyContent = (
      <FlatList
        ref={diffListRef}
        data={flatItems}
        renderItem={renderFlatItem}
        keyExtractor={flatKeyExtractor}
        stickyHeaderIndices={stickyHeaderIndices}
        extraData={{ expandedPathsArray, effectiveLayout, wrapLines }}
        style={styles.scrollView}
        contentContainerStyle={styles.contentContainer}
        testID="git-diff-scroll"
        onLayout={handleDiffListLayout}
        onScroll={handleDiffListScroll}
        onContentSizeChange={scrollbar.onContentSizeChange}
        scrollEventThrottle={16}
        showsVerticalScrollIndicator={!showDesktopWebScrollbar}
        // Mixed-height rows (header + potentially very large body) are prone to clipping artifacts.
        // Keep a larger render window and disable clipping to avoid bodies disappearing mid-scroll.
        removeClippedSubviews={false}
        initialNumToRender={12}
        maxToRenderPerBatch={12}
        windowSize={10}
      />
    );
  }

  useEffect(() => {
    setPostShipArchiveSuggested(false);
  }, [cwd]);

  // ==========================================================================
  // Git Actions (Data-Oriented)
  // ==========================================================================
  // All possible actions are computed as data, then partitioned into:
  // - primary: The main CTA button
  // - secondary: Dropdown next to primary button
  // - menu: Kebab overflow menu
  // ==========================================================================

  const gitActions: GitActions = useMemo(() => {
    return buildGitActions({
      isGit,
      githubFeaturesEnabled,
      hasPullRequest,
      pullRequestUrl: prStatus?.url ?? null,
      hasRemote,
      isPaseoOwnedWorktree,
      isOnBaseBranch,
      hasUncommittedChanges,
      baseRefAvailable: Boolean(baseRef),
      baseRefLabel,
      aheadCount,
      behindBaseCount,
      aheadOfOrigin,
      behindOfOrigin,
      shouldPromoteArchive,
      shipDefault,
      text,
      runtime: {
        commit: {
          disabled: commitDisabled,
          status: commitStatus,
          icon: <GitCommitHorizontal size={16} color={theme.colors.foregroundMuted} />,
          handler: handleCommit,
        },
        pull: {
          disabled: pullDisabled,
          status: pullStatus,
          icon: <Download size={16} color={theme.colors.foregroundMuted} />,
          handler: handlePull,
        },
        push: {
          disabled: pushDisabled,
          status: pushStatus,
          icon: <Upload size={16} color={theme.colors.foregroundMuted} />,
          handler: handlePush,
        },
        pr: {
          disabled: prDisabled,
          status: hasPullRequest ? "idle" : prCreateStatus,
          icon: <GitHubIcon size={16} color={theme.colors.foregroundMuted} />,
          handler: () => {
            if (prStatus?.url) {
              openURLInNewTab(prStatus.url);
              return;
            }
            handleCreatePr();
          },
        },
        "merge-branch": {
          disabled: mergeDisabled,
          status: mergeStatus,
          icon: <GitMerge size={16} color={theme.colors.foregroundMuted} />,
          handler: handleMergeBranch,
        },
        "merge-from-base": {
          disabled: mergeFromBaseDisabled,
          status: mergeFromBaseStatus,
          icon: <RefreshCcw size={16} color={theme.colors.foregroundMuted} />,
          handler: handleMergeFromBase,
        },
        "archive-worktree": {
          disabled: archiveDisabled,
          status: archiveStatus,
          icon: <Archive size={16} color={theme.colors.foregroundMuted} />,
          handler: handleArchiveWorktree,
        },
      },
    });
  }, [
    isGit,
    hasRemote,
    hasPullRequest,
    prStatus?.url,
    aheadCount,
    behindBaseCount,
    isPaseoOwnedWorktree,
    isOnBaseBranch,
    githubFeaturesEnabled,
    hasUncommittedChanges,
    aheadOfOrigin,
    behindOfOrigin,
    shipDefault,
    baseRefLabel,
    shouldPromoteArchive,
    commitDisabled,
    pullDisabled,
    pushDisabled,
    prDisabled,
    mergeDisabled,
    mergeFromBaseDisabled,
    archiveDisabled,
    commitStatus,
    pullStatus,
    pushStatus,
    prCreateStatus,
    mergeStatus,
    mergeFromBaseStatus,
    archiveStatus,
    handleCommit,
    handlePull,
    handlePush,
    handleCreatePr,
    handleMergeBranch,
    handleMergeFromBase,
    handleArchiveWorktree,
    theme.colors.foregroundMuted,
    text,
  ]);

  // Helper to get display label based on status

  return (
    <View style={styles.container}>
      {!hideHeaderRow ? (
        <View style={styles.header} testID="changes-header">
          <View style={styles.headerLeft}>
            <GitBranch size={16} color={theme.colors.foregroundMuted} />
            <Text style={styles.branchLabel} testID="changes-branch" numberOfLines={1}>
              {branchLabel}
            </Text>
          </View>
          {isGit ? <GitActionsSplitButton gitActions={gitActions} /> : null}
        </View>
      ) : null}

      {isGit ? (
        <View style={styles.diffStatusContainer}>
          <View style={styles.diffStatusInner}>
            <DropdownMenu>
              <DropdownMenuTrigger
                style={({ hovered, pressed, open }) => [
                  styles.diffModeTrigger,
                  hovered && styles.diffModeTriggerHovered,
                  (pressed || open) && styles.diffModeTriggerPressed,
                ]}
                testID="changes-diff-status"
                accessibilityRole="button"
                accessibilityLabel={text.diffMode}
              >
                <Text style={styles.diffStatusText} numberOfLines={1}>
                  {diffMode === "uncommitted" ? text.uncommitted : text.committed}
                </Text>
                <ChevronDown size={12} color={theme.colors.foregroundMuted} />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start" width={260} testID="changes-diff-status-menu">
                <DropdownMenuItem
                  testID="changes-diff-mode-uncommitted"
                  selected={diffMode === "uncommitted"}
                  onSelect={() => setDiffModeOverride("uncommitted")}
                >
                  {text.uncommitted}
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  testID="changes-diff-mode-committed"
                  selected={diffMode === "base"}
                  description={committedDiffDescription}
                  onSelect={() => setDiffModeOverride("base")}
                >
                  {text.committed}
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
            <View style={styles.diffStatusButtons}>
              {canUseSplitLayout ? (
                <View style={styles.toggleButtonGroup}>
                  <Tooltip delayDuration={300}>
                    <TooltipTrigger asChild>
                      <Pressable
                        accessibilityRole="button"
                        accessibilityLabel={text.unifiedDiff}
                        testID="changes-layout-unified"
                        onPress={() => handleLayoutChange("unified")}
                        style={({ hovered, pressed }) => [
                          styles.toggleButton,
                          styles.toggleButtonGroupStart,
                          changesPreferences.layout === "unified" && styles.toggleButtonSelected,
                          (hovered || pressed) && styles.diffStatusRowHovered,
                        ]}
                      >
                        <AlignJustify
                          size={14}
                          color={
                            changesPreferences.layout === "unified"
                              ? theme.colors.foreground
                              : theme.colors.foregroundMuted
                          }
                        />
                      </Pressable>
                    </TooltipTrigger>
                    <TooltipContent side="bottom">
                      <Text style={styles.tooltipText}>{text.unifiedDiff}</Text>
                    </TooltipContent>
                  </Tooltip>
                  <Tooltip delayDuration={300}>
                    <TooltipTrigger asChild>
                      <Pressable
                        accessibilityRole="button"
                        accessibilityLabel={text.sideBySideDiff}
                        testID="changes-layout-split"
                        onPress={() => handleLayoutChange("split")}
                        style={({ hovered, pressed }) => [
                          styles.toggleButton,
                          styles.toggleButtonGroupEnd,
                          changesPreferences.layout === "split" && styles.toggleButtonSelected,
                          (hovered || pressed) && styles.diffStatusRowHovered,
                        ]}
                      >
                        <Columns2
                          size={14}
                          color={
                            changesPreferences.layout === "split"
                              ? theme.colors.foreground
                              : theme.colors.foregroundMuted
                          }
                        />
                      </Pressable>
                    </TooltipTrigger>
                    <TooltipContent side="bottom">
                      <Text style={styles.tooltipText}>{text.sideBySideDiff}</Text>
                    </TooltipContent>
                  </Tooltip>
                </View>
              ) : null}
              <Tooltip delayDuration={300}>
                <TooltipTrigger asChild>
                  <Pressable
                    accessibilityRole="button"
                    accessibilityLabel={text.hideWhitespace}
                    testID="changes-toggle-whitespace"
                    style={({ hovered, pressed }) => [
                      styles.expandAllButton,
                      changesPreferences.hideWhitespace && styles.toggleButtonSelected,
                      (hovered || pressed) && styles.diffStatusRowHovered,
                    ]}
                    onPress={handleToggleHideWhitespace}
                  >
                    <Pilcrow
                      size={isMobile ? 18 : 14}
                      color={
                        changesPreferences.hideWhitespace
                          ? theme.colors.foreground
                          : theme.colors.foregroundMuted
                      }
                    />
                  </Pressable>
                </TooltipTrigger>
                <TooltipContent side="bottom">
                  <Text style={styles.tooltipText}>{text.hideWhitespace}</Text>
                </TooltipContent>
              </Tooltip>
              {files.length > 0 ? (
                <View style={styles.diffStatusButtons}>
                  <Tooltip delayDuration={300}>
                    <TooltipTrigger asChild>
                      <Pressable
                        style={({ hovered, pressed }) => [
                          styles.expandAllButton,
                          wrapLines && styles.toggleButtonSelected,
                          (hovered || pressed) && styles.diffStatusRowHovered,
                        ]}
                        onPress={handleToggleWrapLines}
                      >
                        <WrapText
                          size={isMobile ? 18 : 14}
                          color={wrapLines ? theme.colors.foreground : theme.colors.foregroundMuted}
                        />
                      </Pressable>
                    </TooltipTrigger>
                    <TooltipContent side="bottom">
                      <Text style={styles.tooltipText}>
                        {wrapLines ? text.scrollLongLines : text.wrapLongLines}
                      </Text>
                    </TooltipContent>
                  </Tooltip>
                  <Tooltip delayDuration={300}>
                    <TooltipTrigger asChild>
                      <Pressable
                        style={({ hovered, pressed }) => [
                          styles.expandAllButton,
                          (hovered || pressed) && styles.diffStatusRowHovered,
                        ]}
                        onPress={handleToggleExpandAll}
                      >
                        {allExpanded ? (
                          <ListChevronsDownUp
                            size={isMobile ? 18 : 14}
                            color={theme.colors.foregroundMuted}
                          />
                        ) : (
                          <ListChevronsUpDown
                            size={isMobile ? 18 : 14}
                            color={theme.colors.foregroundMuted}
                          />
                        )}
                      </Pressable>
                    </TooltipTrigger>
                    <TooltipContent side="bottom">
                      <Text style={styles.tooltipText}>
                        {allExpanded ? text.collapseAllFiles : text.expandAllFiles}
                      </Text>
                    </TooltipContent>
                  </Tooltip>
                </View>
              ) : null}
            </View>
          </View>
        </View>
      ) : null}

      {prErrorMessage ? <Text style={styles.actionErrorText}>{prErrorMessage}</Text> : null}

      {isGit && diffMode === "uncommitted" && !isMobile && hasChanges ? (
        <View style={styles.inlineCommitPanel} testID="changes-inline-commit">
          <TextInput
            ref={commitInputRef}
            value={commitMessage}
            onChangeText={handleCommitMessageChange}
            placeholder={text.commitMessagePlaceholder}
            placeholderTextColor={theme.colors.foregroundMuted}
            autoCapitalize="sentences"
            autoCorrect
            editable={commitStatus !== "pending"}
            returnKeyType="done"
            onSubmitEditing={handleCommit}
            style={styles.inlineCommitInput}
            testID="changes-inline-commit-message"
          />
          <View style={styles.inlineCommitMetaRow}>
            <Text style={styles.inlineCommitCount} numberOfLines={1}>
              {text.commitFilesSelected(selectedCommitCount, commitFilePaths.length)}
            </Text>
            <View style={styles.inlineCommitMetaActions}>
              <Button
                variant="ghost"
                size="sm"
                onPress={allCommitFilesSelected ? clearCommitFiles : selectAllCommitFiles}
                disabled={commitStatus === "pending"}
                testID="changes-inline-toggle-all-files"
              >
                {allCommitFilesSelected ? text.clearCommitFiles : text.selectAllCommitFiles}
              </Button>
              <Button
                variant="default"
                size="sm"
                onPress={handleCommit}
                disabled={commitStatus === "pending"}
                leftIcon={<GitCommitHorizontal size={14} color={theme.colors.palette.white} />}
                testID="changes-inline-commit-submit"
              >
                {commitStatus === "pending" ? text.committing : text.commitAction}
              </Button>
            </View>
          </View>
          {commitError ? <Text style={styles.inlineCommitError}>{commitError}</Text> : null}
        </View>
      ) : null}

      <View style={styles.diffContainer}>
        {bodyContent}
        {hasChanges ? scrollbar.overlay : null}
      </View>
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  container: {
    flex: 1,
    minHeight: 0,
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: theme.spacing[2],
    paddingHorizontal: theme.spacing[3],
    paddingVertical: theme.spacing[2],
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
  },
  headerLeft: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    flex: 1,
    minWidth: 0,
  },
  branchLabel: {
    fontSize: theme.fontSize.sm,
    color: theme.colors.foreground,
    fontWeight: theme.fontWeight.medium,
    flexShrink: 1,
  },
  diffStatusContainer: {
    height: WORKSPACE_SECONDARY_HEADER_HEIGHT,
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
  },
  diffStatusInner: {
    flex: 1,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingRight: theme.spacing[3],
  },
  diffModeTrigger: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: theme.spacing[1],
    // Align text with header branch icon (at spacing[3] from edge, minus our horizontal padding)
    marginLeft: theme.spacing[3] - theme.spacing[1],
    paddingHorizontal: theme.spacing[1],
    height: {
      xs: 28,
      sm: 28,
      md: 24,
    },
    borderRadius: theme.borderRadius.base,
    flexShrink: 0,
  },
  diffModeTriggerHovered: {
    backgroundColor: theme.colors.surface2,
  },
  diffModeTriggerPressed: {
    backgroundColor: theme.colors.surface2,
  },
  diffStatusRowHovered: {
    backgroundColor: theme.colors.surface2,
  },
  diffStatusText: {
    fontSize: theme.fontSize.xs,
    lineHeight: theme.fontSize.xs * 1.25,
    color: theme.colors.foregroundMuted,
  },
  diffStatusIconHidden: {
    opacity: 0,
  },
  diffStatusButtons: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[1],
    flexWrap: "wrap",
  },
  toggleButtonGroup: {
    flexDirection: "row",
    alignItems: "center",
  },
  toggleButton: {
    alignItems: "center",
    justifyContent: "center",
    minWidth: {
      xs: 32,
      sm: 32,
      md: 24,
    },
    height: {
      xs: 32,
      sm: 32,
      md: 24,
    },
    paddingHorizontal: {
      xs: theme.spacing[2],
      sm: theme.spacing[2],
      md: theme.spacing[1],
    },
  },
  toggleButtonGroupStart: {
    borderTopLeftRadius: theme.borderRadius.base,
    borderBottomLeftRadius: theme.borderRadius.base,
  },
  toggleButtonGroupEnd: {
    borderTopRightRadius: theme.borderRadius.base,
    borderBottomRightRadius: theme.borderRadius.base,
  },
  toggleButtonSelected: {
    backgroundColor: theme.colors.surface2,
  },
  expandAllButton: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: theme.spacing[1],
    minWidth: {
      xs: 32,
      sm: 32,
      md: 24,
    },
    height: {
      xs: 32,
      sm: 32,
      md: 24,
    },
    paddingHorizontal: {
      xs: theme.spacing[2],
      sm: theme.spacing[2],
      md: theme.spacing[1],
    },
    borderRadius: theme.borderRadius.base,
    flexShrink: 0,
  },
  actionErrorText: {
    paddingHorizontal: theme.spacing[3],
    paddingBottom: theme.spacing[1],
    fontSize: theme.fontSize.xs,
    color: theme.colors.destructive,
  },
  inlineCommitPanel: {
    gap: theme.spacing[2],
    paddingHorizontal: theme.spacing[3],
    paddingVertical: theme.spacing[2],
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
    backgroundColor: theme.colors.surface1,
  },
  inlineCommitInput: {
    minHeight: 34,
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.md,
    backgroundColor: theme.colors.surface0,
    color: theme.colors.foreground,
    paddingHorizontal: theme.spacing[3],
    paddingVertical: theme.spacing[2],
    fontSize: theme.fontSize.sm,
  },
  inlineCommitMetaRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: theme.spacing[2],
  },
  inlineCommitCount: {
    flex: 1,
    minWidth: 0,
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  inlineCommitMetaActions: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    flexShrink: 0,
  },
  inlineCommitError: {
    color: theme.colors.destructive,
    fontSize: theme.fontSize.xs,
  },
  diffContainer: {
    flex: 1,
    minHeight: 0,
    position: "relative",
  },
  scrollView: {
    flex: 1,
  },
  contentContainer: {
    paddingBottom: theme.spacing[8],
  },
  loadingContainer: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    paddingTop: theme.spacing[16],
    gap: theme.spacing[4],
  },
  loadingText: {
    fontSize: theme.fontSize.base,
    color: theme.colors.foregroundMuted,
  },
  errorContainer: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    paddingTop: theme.spacing[16],
    paddingHorizontal: theme.spacing[6],
  },
  errorText: {
    fontSize: theme.fontSize.base,
    color: theme.colors.destructive,
    textAlign: "center",
  },
  emptyContainer: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    paddingTop: theme.spacing[16],
  },
  emptyText: {
    fontSize: theme.fontSize.lg,
    color: theme.colors.foregroundMuted,
  },
  fileSection: {
    overflow: "hidden",
    backgroundColor: theme.colors.surface2,
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
  },
  fileSectionHeaderContainer: {
    overflow: "hidden",
  },
  fileSectionHeaderExpanded: {
    backgroundColor: theme.colors.surface1,
  },
  fileSectionBodyContainer: {
    overflow: "hidden",
    backgroundColor: theme.colors.surface2,
  },
  fileSectionBorder: {
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
  },
  fileHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingLeft: theme.spacing[3],
    paddingRight: theme.spacing[2],
    paddingVertical: theme.spacing[2],
    gap: theme.spacing[1],
    zIndex: 2,
    elevation: 2,
  },
  fileSelectButton: {
    width: 22,
    height: 22,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: theme.borderRadius.base,
    flexShrink: 0,
  },
  fileHeaderPressed: {
    opacity: 0.7,
  },
  fileHeaderLeft: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[1],
    flex: 1,
    minWidth: 0,
  },
  fileHeaderRight: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[1],
    flexShrink: 0,
  },
  fileName: {
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.normal,
    color: theme.colors.foreground,
    flexShrink: 0,
  },
  fileDir: {
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.normal,
    color: theme.colors.foregroundMuted,
    flex: 1,
  },
  newBadge: {
    backgroundColor: "rgba(46, 160, 67, 0.2)",
    paddingHorizontal: theme.spacing[2],
    paddingVertical: theme.spacing[1],
    borderRadius: theme.borderRadius.md,
    flexShrink: 0,
  },
  newBadgeText: {
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.normal,
    color: theme.colors.diffAddition,
  },
  deletedBadge: {
    backgroundColor: "rgba(248, 81, 73, 0.2)",
    paddingHorizontal: theme.spacing[2],
    paddingVertical: theme.spacing[1],
    borderRadius: theme.borderRadius.md,
    flexShrink: 0,
  },
  deletedBadgeText: {
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.normal,
    color: theme.colors.diffDeletion,
  },
  additions: {
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.normal,
    color: theme.colors.diffAddition,
  },
  deletions: {
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.normal,
    color: theme.colors.diffDeletion,
  },
  diffContent: {
    borderTopWidth: theme.borderWidth[1],
    borderTopColor: theme.colors.border,
    backgroundColor: theme.colors.surface1,
  },
  diffContentRow: {
    flexDirection: "row",
    alignItems: "stretch",
  },
  diffContentInner: {
    flexDirection: "column",
  },
  linesContainer: {
    backgroundColor: theme.colors.surface1,
  },
  gutterColumn: {
    backgroundColor: theme.colors.surface1,
  },
  gutterCell: {
    borderRightWidth: theme.borderWidth[1],
    borderRightColor: theme.colors.border,
    justifyContent: "flex-start",
  },
  textLineContainer: {
    flexDirection: "row",
    alignItems: "stretch",
    paddingLeft: theme.spacing[2],
  },
  splitRow: {
    flexDirection: "row",
    alignItems: "stretch",
  },
  splitColumnScroll: {
    flex: 1,
  },
  splitHeaderRow: {
    backgroundColor: theme.colors.surface2,
    paddingHorizontal: theme.spacing[3],
  },
  splitCell: {
    flex: 1,
    flexBasis: 0,
    backgroundColor: theme.colors.surface2,
  },
  splitCellRow: {
    flexDirection: "row",
    alignItems: "stretch",
  },
  emptySplitCell: {
    backgroundColor: theme.colors.surfaceDiffEmpty,
  },
  splitCellWithDivider: {
    borderLeftWidth: theme.borderWidth[1],
    borderLeftColor: theme.colors.border,
  },
  diffLineContainer: {
    flexDirection: "row",
    alignItems: "stretch",
  },
  lineNumberGutter: {
    borderRightWidth: theme.borderWidth[1],
    borderRightColor: theme.colors.border,
    marginRight: theme.spacing[2],
    alignSelf: "stretch",
    justifyContent: "flex-start",
  },
  lineNumberText: {
    textAlign: "right",
    paddingRight: theme.spacing[2],
    fontSize: theme.fontSize.xs,
    lineHeight: theme.lineHeight.diff,
    fontFamily: Fonts.mono,
    color: theme.colors.foregroundMuted,
    userSelect: "none",
  },
  addLineNumberText: {
    color: theme.colors.diffAddition,
  },
  removeLineNumberText: {
    color: theme.colors.diffDeletion,
  },
  diffLineText: {
    flex: 1,
    paddingRight: theme.spacing[3],
    fontSize: theme.fontSize.xs,
    lineHeight: theme.lineHeight.diff,
    fontFamily: Fonts.mono,
    color: theme.colors.foreground,
    userSelect: "text",
  },
  addLineContainer: {
    backgroundColor: "rgba(46, 160, 67, 0.15)", // GitHub green
  },
  addLineText: {
    color: theme.colors.foreground,
  },
  removeLineContainer: {
    backgroundColor: "rgba(248, 81, 73, 0.1)", // GitHub red
  },
  removeLineText: {
    color: theme.colors.foreground,
  },
  headerLineContainer: {
    backgroundColor: theme.colors.surface2,
  },
  headerLineText: {
    color: theme.colors.foregroundMuted,
  },
  contextLineContainer: {
    backgroundColor: theme.colors.surface1,
  },
  contextLineText: {
    color: theme.colors.foregroundMuted,
  },
  emptySplitCellText: {
    color: "transparent",
  },
  statusMessageContainer: {
    borderTopWidth: theme.borderWidth[1],
    borderTopColor: theme.colors.border,
    backgroundColor: theme.colors.surface1,
    paddingHorizontal: theme.spacing[3],
    paddingVertical: theme.spacing[4],
  },
  statusMessageText: {
    fontSize: theme.fontSize.sm,
    color: theme.colors.foregroundMuted,
    fontStyle: "italic",
  },
  tooltipText: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foreground,
  },
}));

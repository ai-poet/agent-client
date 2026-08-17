import { memo, useMemo, useState } from "react";
import { View } from "react-native";
import { Layers } from "lucide-react-native";

import { getAppMessages } from "@/i18n/sub2api";
import type { AgentToolCallItem } from "@/types/stream";
import { buildToolCallDisplayModel } from "@/utils/tool-call-display";
import { ExpandableBadge, ToolCall } from "./message";
import { summarizeToolGroup, type StreamToolGroupNode } from "./agent-stream-tool-grouping";

interface ToolCallGroupProps {
  group: StreamToolGroupNode;
  cwd?: string;
  locale?: string | null;
  isLastInSequence?: boolean;
  onInlineDetailsExpandedChange?: (expanded: boolean) => void;
}

function resolveReadFileName(item: AgentToolCallItem): string | null {
  const detail = item.payload.data.detail;
  if (detail.type !== "read") {
    return null;
  }
  const filePath = (detail as { filePath?: unknown }).filePath;
  if (typeof filePath !== "string" || filePath.length === 0) {
    return null;
  }
  return filePath.split(/[\\/]/).pop() ?? filePath;
}

/** Collapsed row standing in for a run of consecutive tool calls. */
export const ToolCallGroup = memo(function ToolCallGroup({
  group,
  cwd,
  locale,
  isLastInSequence = false,
  onInlineDetailsExpandedChange,
}: ToolCallGroupProps) {
  const [isExpanded, setIsExpanded] = useState(false);
  const text = useMemo(() => getAppMessages(locale ?? "en").agentTools, [locale]);

  const summary = useMemo(
    () =>
      summarizeToolGroup(
        group,
        (item) =>
          buildToolCallDisplayModel(
            {
              name: item.payload.data.name,
              status: item.payload.data.status,
              error: item.payload.data.error,
              detail: item.payload.data.detail,
              metadata: item.payload.data.metadata,
            },
            { locale },
          ).displayName,
        resolveReadFileName,
      ),
    [group, locale],
  );

  const { label, secondaryLabel } = useMemo(() => {
    if (summary.readFileNames) {
      // Keep the preview short; the count already communicates the full size.
      const preview = summary.readFileNames.slice(0, 3);
      return {
        label: text.group.readFiles(summary.count),
        secondaryLabel: text.group.fileList(preview, summary.count),
      };
    }
    return {
      label: text.group.mixedCalls(summary.count),
      secondaryLabel: text.group.toolList(
        summary.toolCounts.map(([name, count]) => text.group.toolCount(name, count)),
      ),
    };
  }, [summary, text]);

  return (
    <ExpandableBadge
      label={label}
      secondaryLabel={secondaryLabel}
      icon={Layers}
      isExpanded={isExpanded}
      isError={group.status === "failed"}
      isLastInSequence={isLastInSequence}
      onToggle={() => {
        setIsExpanded((previous) => {
          onInlineDetailsExpandedChange?.(!previous);
          return !previous;
        });
      }}
      renderDetails={() => (
        <View>
          {group.items.map((item, index) => (
            <ToolCall
              key={item.id}
              toolName={item.payload.data.name}
              error={item.payload.data.error}
              status={item.payload.data.status}
              detail={item.payload.data.detail}
              cwd={cwd}
              metadata={item.payload.data.metadata}
              isLastInSequence={index === group.items.length - 1}
              locale={locale}
              disableOuterSpacing
            />
          ))}
        </View>
      )}
    />
  );
});

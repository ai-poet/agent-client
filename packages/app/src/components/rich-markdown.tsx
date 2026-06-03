import type { ReactNode } from "react";

export interface RichMarkdownProps {
  text: string;
  variant: "assistant" | "plan";
  onLinkPress?: (url: string) => boolean | void;
  onInlinePathPress?: (target: import("@/utils/inline-path").InlinePathTarget) => void;
  workspaceRoot?: string;
  serverId?: string;
  client?: import("@server/client/daemon-client").DaemonClient | null;
  fallback: ReactNode;
}

export function RichMarkdown({ fallback }: RichMarkdownProps) {
  return <>{fallback}</>;
}

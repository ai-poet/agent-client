import type { StreamItem } from "@/types/stream";

/**
 * What the agent is doing right now, in priority order. Anything that blocks on the user
 * outranks anything the agent is doing on its own — that is the state the user must act on.
 */
export type TurnPhase =
  | "awaiting_approval"
  | "awaiting_input"
  | "thinking"
  | "running_tool"
  | "generating"
  | "working";

export interface TurnProgressInput {
  pendingPermissionCount: number;
  /** A question card is waiting for an answer. */
  awaitingUserInput: boolean;
  /** Newest items first is not required; only the tail matters. */
  items: StreamItem[];
}

export function resolveTurnPhase(input: TurnProgressInput): TurnPhase {
  if (input.pendingPermissionCount > 0) {
    return "awaiting_approval";
  }
  if (input.awaitingUserInput) {
    return "awaiting_input";
  }

  // The most recent still-open item tells us what the agent is actually doing.
  for (let index = input.items.length - 1; index >= 0; index -= 1) {
    const item = input.items[index];
    if (!item) {
      continue;
    }
    if (item.kind === "tool_call") {
      const status =
        item.payload.source === "agent" ? item.payload.data.status : item.payload.data.status;
      if (status === "running" || status === "executing") {
        return "running_tool";
      }
      // A finished tool means the agent is composing its next step.
      return "generating";
    }
    if (item.kind === "thought") {
      return item.status === "ready" ? "generating" : "thinking";
    }
    if (item.kind === "assistant_message") {
      return "generating";
    }
    if (item.kind === "user_message") {
      // Nothing has come back yet.
      return "working";
    }
  }

  return "working";
}

/**
 * Rough output-token estimate for the streaming phase, so the counter moves immediately
 * instead of sitting at zero until the provider reports real usage. CJK packs far more
 * meaning per character than ASCII, hence the two divisors.
 */
export function estimateOutputTokens(text: string): number {
  if (!text) {
    return 0;
  }
  let ascii = 0;
  let wide = 0;
  for (const char of text) {
    const code = char.codePointAt(0) ?? 0;
    if (code > 0x2e80) {
      wide += 1;
    } else {
      ascii += 1;
    }
  }
  return Math.round(ascii / 4 + wide / 1.7);
}

/**
 * Real usage wins as soon as it catches up to the estimate. Until then the estimate keeps
 * the number moving; swapping earlier would make the counter jump backwards.
 */
export function resolveDisplayTokens(options: {
  estimated: number;
  reported: number | undefined;
}): { tokens: number; isEstimate: boolean } {
  const reported = options.reported ?? 0;
  if (reported > 0 && reported >= options.estimated) {
    return { tokens: reported, isEstimate: false };
  }
  return { tokens: options.estimated, isEstimate: true };
}

/** Compact elapsed-time label for a line that updates every second. */
export function formatElapsed(ms: number): string {
  if (!Number.isFinite(ms) || ms < 1000) {
    return "0s";
  }
  const totalSeconds = Math.floor(ms / 1000);
  if (totalSeconds < 60) {
    return `${totalSeconds}s`;
  }
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes < 60) {
    return seconds > 0 ? `${minutes}m ${seconds}s` : `${minutes}m`;
  }
  const hours = Math.floor(minutes / 60);
  const remainingMinutes = minutes % 60;
  return remainingMinutes > 0 ? `${hours}h ${remainingMinutes}m` : `${hours}h`;
}

/**
 * Collects the assistant text produced since the last user message — the basis for the
 * streaming token estimate.
 */
export function collectStreamingText(items: StreamItem[]): string {
  const parts: string[] = [];
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const item = items[index];
    if (!item || item.kind === "user_message") {
      break;
    }
    if (item.kind === "assistant_message") {
      parts.push(item.text);
    }
  }
  return parts.reverse().join("");
}

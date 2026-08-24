/**
 * Desktop provider store payload (see @getpaseo/desktop provider-switch).
 * Must stay in lockstep with `ManagedProviderTarget` there — the UI cannot offer a
 * target the switch layer refuses to write.
 */
export type ManagedProviderTarget = "claude" | "codex" | "grok" | "pi";

/** Claude Code is Anthropic-native only in settings we write; OpenAI-shaped upstreams are out of scope for now. */
export type ClaudeApiFormat = "anthropic";

/** Codex is wired for OpenAI Responses only; chat-completions is out of scope for now. */
export type CodexWireApi = "responses";

export interface DesktopProviderPayload {
  id: string;
  name: string;
  type: "default" | "custom";
  endpoint: string;
  apiKey: string;
  isDefault: boolean;
  /** Legacy rows may omit this before migration; new rows are always scoped. */
  target?: ManagedProviderTarget;
  claudeApiFormat?: ClaudeApiFormat;
  codexWireApi?: CodexWireApi;
}

export interface ProviderStore {
  providers: DesktopProviderPayload[];
  /** Legacy single active id; kept in sync when Claude and Codex point at the same entry. */
  activeProviderId: string | null;
  activeClaudeProviderId: string | null;
  activeCodexProviderId: string | null;
  /** Optional: stores written before Grok/Pi routing existed omit these. */
  activeGrokProviderId?: string | null;
  activePiProviderId?: string | null;
}

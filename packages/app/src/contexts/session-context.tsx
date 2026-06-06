import { useRef, ReactNode, useCallback, useEffect } from "react";
import { AppState } from "react-native";
import { useQueryClient } from "@tanstack/react-query";
import { useClientActivity } from "@/hooks/use-client-activity";
import { usePushTokenRegistration } from "@/hooks/use-push-token-registration";
import { clearArchiveAgentPending } from "@/hooks/use-archive-agent";
import { prefetchProvidersSnapshot } from "@/hooks/use-providers-snapshot";
import { generateMessageId, type StreamItem } from "@/types/stream";
import {
  processTimelineResponse,
  processAgentStreamEvent,
} from "@/contexts/session-stream-reducers";
import type {
  ActivityLogPayload,
  AgentAttachment,
  AgentStreamEventPayload,
  SessionOutboundMessage,
} from "@server/shared/messages";
import { parseServerInfoStatusPayload } from "@server/shared/messages";
import {
  buildAgentAttentionNotificationPayload,
  type AgentAttentionNotificationPayload,
  type NotificationPermissionRequest,
} from "@server/shared/agent-attention-notification";
import type { AgentLifecycleStatus } from "@server/shared/agent-lifecycle";
import type { DaemonClient } from "@server/client/daemon-client";
import { File } from "expo-file-system";
import { getHostRuntimeStore, useHostRuntimeIsConnected } from "@/runtime/host-runtime";
import { useVoiceAudioEngineOptional, useVoiceRuntimeOptional } from "@/contexts/voice-context";
import type { AudioPlaybackSource } from "@/voice/audio-engine-types";
import {
  useSessionStore,
  type Agent,
  type SessionState,
  type WorkspaceDescriptor,
  normalizeWorkspaceDescriptor,
} from "@/stores/session-store";
import { useDraftStore } from "@/stores/draft-store";
import { useWorkspaceSetupStore } from "@/stores/workspace-setup-store";
import type { AgentDirectoryEntry } from "@/types/agent-directory";
import { sendOsNotification } from "@/utils/os-notifications";
import { getIsAppActivelyVisible } from "@/utils/app-visibility";
import {
  getInitKey,
  getInitDeferred,
  resolveInitDeferred,
  rejectInitDeferred,
} from "@/utils/agent-initialization";
import { encodeImages } from "@/utils/encode-images";
import { derivePendingPermissionKey, normalizeAgentSnapshot } from "@/utils/agent-snapshots";
import { resolveProjectPlacement } from "@/utils/project-placement";
import { buildDraftStoreKey } from "@/stores/draft-keys";
import type { AttachmentMetadata } from "@/attachments/types";
import { splitComposerAttachmentsForSubmit } from "@/components/composer-attachments";
import { reconcilePreviousAgentStatuses } from "@/contexts/session-status-tracking";
import { patchWorkspaceScripts } from "@/contexts/session-workspace-scripts";
import { isNative } from "@/constants/platform";
import { useToast } from "@/contexts/toast-context";
import { toErrorMessage } from "@/utils/error-messages";
import { cloudServiceQueryKeys } from "@/hooks/use-sub2api-api";
import { useStableEvent } from "@/hooks/use-stable-event";

// ---------------------------------------------------------------------------
// Module-level pending agent updates buffer (scoped by serverId)
// ---------------------------------------------------------------------------
const pendingAgentUpdates = new Map<string, AgentUpdatePayload>();

function pendingKey(serverId: string, agentId: string): string {
  return `${serverId}:${agentId}`;
}

export function bufferPendingAgentUpdate(
  serverId: string,
  agentId: string,
  update: AgentUpdatePayload,
): void {
  pendingAgentUpdates.set(pendingKey(serverId, agentId), update);
}

export function flushPendingAgentUpdate(
  serverId: string,
  agentId: string,
): AgentUpdatePayload | undefined {
  const key = pendingKey(serverId, agentId);
  const update = pendingAgentUpdates.get(key);
  pendingAgentUpdates.delete(key);
  return update;
}

export function deletePendingAgentUpdate(serverId: string, agentId: string): void {
  pendingAgentUpdates.delete(pendingKey(serverId, agentId));
}

export function clearPendingAgentUpdates(serverId: string): void {
  for (const key of [...pendingAgentUpdates.keys()]) {
    if (key.startsWith(`${serverId}:`)) {
      pendingAgentUpdates.delete(key);
    }
  }
}

// Re-export types from session-store and draft-store for backward compatibility
export type { DraftInput } from "@/stores/draft-store";
export type {
  MessageEntry,
  Agent,
  ExplorerEntry,
  ExplorerFile,
  ExplorerEntryKind,
  ExplorerFileKind,
  ExplorerEncoding,
  AgentFileExplorerState,
} from "@/stores/session-store";

type AgentUpdatePayload = Extract<SessionOutboundMessage, { type: "agent_update" }>["payload"];

type FileExplorerPayload = Extract<
  SessionOutboundMessage,
  { type: "file_explorer_response" }
>["payload"];

type FileDownloadTokenPayload = Extract<
  SessionOutboundMessage,
  { type: "file_download_token_response" }
>["payload"];

type WorkspaceSetupProgressPayload = Extract<
  SessionOutboundMessage,
  { type: "workspace_setup_progress" }
>["payload"];

// ---------------------------------------------------------------------------
// Provider Props
// ---------------------------------------------------------------------------
interface SessionProviderSharedProps {
  children: ReactNode;
  serverId: string;
}

interface SessionProviderClientProps extends SessionProviderSharedProps {
  client: DaemonClient;
}

export type SessionProviderProps = SessionProviderClientProps;

function SessionProviderWithClient({ children, serverId, client }: SessionProviderClientProps) {
  return (
    <SessionProviderInternal serverId={serverId} client={client}>
      {children}
    </SessionProviderInternal>
  );
}

export function SessionProvider(props: SessionProviderProps) {
  return <SessionProviderWithClient {...props} />;
}

// ---------------------------------------------------------------------------
// Audio helpers (kept at module level for use in daemon event handlers)
// ---------------------------------------------------------------------------
interface BufferedAudioChunk {
  chunkIndex: number;
  audio: string;
  format: string;
  id: string;
}

function decodeBase64Chunk(base64: string): Uint8Array {
  return Buffer.from(base64, "base64");
}

function buildAudioPlaybackSource(chunks: BufferedAudioChunk[]): AudioPlaybackSource {
  const decodedChunks = chunks.map((chunk) => decodeBase64Chunk(chunk.audio));
  const totalSize = decodedChunks.reduce((sum, chunk) => sum + chunk.length, 0);
  const output = new Uint8Array(totalSize);
  let offset = 0;
  for (const chunk of decodedChunks) {
    output.set(chunk, offset);
    offset += chunk.length;
  }

  const format = chunks[0]?.format ?? "pcm";
  const mimeType =
    format === "pcm"
      ? "audio/pcm;rate=24000;bits=16"
      : format === "mp3"
        ? "audio/mpeg"
        : `audio/${format}`;

  const bytes = output.slice();
  return {
    size: bytes.byteLength,
    type: mimeType,
    async arrayBuffer() {
      return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
    },
  };
}

// ---------------------------------------------------------------------------
// Helper: find latest assistant message text for attention notifications
// ---------------------------------------------------------------------------
const findLatestAssistantMessageText = (items: StreamItem[]): string | null => {
  for (let i = items.length - 1; i >= 0; i -= 1) {
    const item = items[i];
    if (item.kind === "assistant_message") {
      return item.text;
    }
  }
  return null;
};

// ---------------------------------------------------------------------------
// Helper: get latest permission request for an agent
// ---------------------------------------------------------------------------
const getLatestPermissionRequest = (
  session: SessionState | undefined,
  agentId: string,
): NotificationPermissionRequest | null => {
  if (!session) {
    return null;
  }

  let latest: NotificationPermissionRequest | null = null;
  for (const pending of session.pendingPermissions.values()) {
    if (pending.agentId === agentId) {
      latest = pending.request;
    }
  }
  if (latest) {
    return latest;
  }

  const agentPending = session.agents.get(agentId)?.pendingPermissions;
  if (agentPending && agentPending.length > 0) {
    return agentPending[agentPending.length - 1] as NotificationPermissionRequest;
  }

  return null;
};

// ---------------------------------------------------------------------------
// Helper: check if agent usage has changed
// ---------------------------------------------------------------------------
function hasAgentUsageChanged(
  incomingUsage: Agent["lastUsage"] | undefined,
  currentUsage: Agent["lastUsage"] | undefined,
): boolean {
  const keys: Array<keyof NonNullable<Agent["lastUsage"]>> = [
    "inputTokens",
    "outputTokens",
    "cachedInputTokens",
    "totalCostUsd",
    "contextWindowMaxTokens",
    "contextWindowUsedTokens",
  ];

  return keys.some((key) => incomingUsage?.[key] !== currentUsage?.[key]);
}

// ---------------------------------------------------------------------------
// Throttled balance refresh
// ---------------------------------------------------------------------------
let _lastBalanceRefreshAt = 0;
const BALANCE_REFRESH_THROTTLE_MS = 10_000;

function throttledBalanceRefresh(queryClient: ReturnType<typeof useQueryClient>) {
  const now = Date.now();
  if (now - _lastBalanceRefreshAt < BALANCE_REFRESH_THROTTLE_MS) return;
  _lastBalanceRefreshAt = now;
  void queryClient.invalidateQueries({ queryKey: cloudServiceQueryKeys.root });
}

const HISTORY_STALE_AFTER_MS = 60_000;
const AUTHORITATIVE_REVALIDATION_DEBOUNCE_MS = 300;

const getAgentIdFromUpdate = (update: AgentUpdatePayload): string =>
  update.kind === "remove" ? update.agentId : update.agent.id;

// ---------------------------------------------------------------------------
// Internal Provider
// ---------------------------------------------------------------------------
function SessionProviderInternal({ children, serverId, client }: SessionProviderClientProps) {
  const voiceRuntime = useVoiceRuntimeOptional();
  const voiceAudioEngine = useVoiceAudioEngineOptional();
  const queryClient = useQueryClient();
  const isConnected = useHostRuntimeIsConnected(serverId);
  const toast = useToast();

  // Direct store access — actions are stable references, no subscription needed.
  const store = useSessionStore;
  const draftStore = useDraftStore;
  const workspaceSetupStore = useWorkspaceSetupStore;

  // State selectors (these need subscription)
  const focusedAgentId = useSessionStore(
    (state) => state.sessions[serverId]?.focusedAgentId ?? null,
  );
  const sessionAgents = useSessionStore((state) => state.sessions[serverId]?.agents);

  // Refs
  const previousAgentStatusRef = useRef<Map<string, AgentLifecycleStatus>>(new Map());
  const sendAgentMessageRef = useRef<
    | ((
        agentId: string,
        message: string,
        images?: AttachmentMetadata[],
        attachments?: AgentAttachment[],
        options?: { hidden?: boolean },
      ) => Promise<void>)
    | null
  >(null);
  const sessionStateTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const attentionNotifiedRef = useRef<Map<string, number>>(new Map());
  const appStateRef = useRef(AppState.currentState);
  const revalidationTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const revalidationInFlightRef = useRef<Promise<void> | null>(null);
  const revalidationQueuedRef = useRef(false);
  const wasConnectedRef = useRef(isConnected);
  const audioOutputBuffersRef = useRef<Map<string, BufferedAudioChunk[]>>(new Map());
  const activeAudioGroupsRef = useRef<Set<string>>(new Set());

  // -------------------------------------------------------------------------
  // AppState tracking
  // -------------------------------------------------------------------------
  useEffect(() => {
    const subscription = AppState.addEventListener("change", (nextState) => {
      appStateRef.current = nextState;
    });
    return () => {
      subscription.remove();
    };
  }, []);

  // -------------------------------------------------------------------------
  // Reconcile previous agent statuses when agents change
  // -------------------------------------------------------------------------
  useEffect(() => {
    previousAgentStatusRef.current = reconcilePreviousAgentStatuses(
      previousAgentStatusRef.current,
      sessionAgents,
    );
  }, [sessionAgents]);

  // -------------------------------------------------------------------------
  // Workspace hydration (optimised: first-page fast, rest in background)
  // -------------------------------------------------------------------------
  const hydrationCursorRef = useRef<string | null>(null);
  const hydrationRunningRef = useRef(false);

  const hydrateWorkspaces = useCallback(
    async (options?: {
      subscribe?: boolean;
      isCancelled?: () => boolean;
      firstPageOnly?: boolean;
    }) => {
      if (!client || !isConnected) {
        return;
      }

      const firstPageOnly = options?.firstPageOnly ?? false;
      const workspaces = new Map<string, WorkspaceDescriptor>(
        store.getState().sessions[serverId]?.workspaces,
      );
      let cursor = firstPageOnly ? null : hydrationCursorRef.current;
      let includeSubscribe = options?.subscribe ?? false;
      let pageCount = 0;

      hydrationRunningRef.current = true;

      try {
        while (true) {
          const payload = await client.fetchWorkspaces({
            sort: [{ key: "activity_at", direction: "desc" }],
            ...(includeSubscribe ? { subscribe: {} } : {}),
            page: cursor ? { limit: 200, cursor } : { limit: 200 },
          });
          if (options?.isCancelled?.()) {
            return;
          }

          for (const entry of payload.entries) {
            const workspace = normalizeWorkspaceDescriptor(entry);
            workspaces.set(workspace.id, workspace);
          }

          pageCount += 1;

          // After first page: immediately make workspaces available for UI
          if (pageCount === 1) {
            store.getState().setWorkspaces(serverId, workspaces);
            store.getState().setHasHydratedWorkspaces(serverId, true);

            if (firstPageOnly) {
              hydrationCursorRef.current = payload.pageInfo.nextCursor ?? null;
              return;
            }
          }

          if (!payload.pageInfo.hasMore || !payload.pageInfo.nextCursor) {
            hydrationCursorRef.current = null;
            break;
          }
          cursor = payload.pageInfo.nextCursor;
          includeSubscribe = false;
        }

        if (options?.isCancelled?.()) {
          return;
        }

        // Final write with all accumulated pages
        store.getState().setWorkspaces(serverId, workspaces);
      } finally {
        hydrationRunningRef.current = false;
      }
    },
    [client, isConnected, serverId],
  );

  // -------------------------------------------------------------------------
  // Authoritative agent snapshot application
  // -------------------------------------------------------------------------
  const applyAuthoritativeAgentSnapshot = useCallback(
    (agent: Agent) => {
      store.getState().setAgents(serverId, (prev) => {
        const current = prev.get(agent.id);
        if (current && agent.updatedAt.getTime() < current.updatedAt.getTime()) {
          const hasUsageUpdate = hasAgentUsageChanged(agent.lastUsage, current.lastUsage);
          if (hasUsageUpdate) {
            const next = new Map(prev);
            next.set(agent.id, {
              ...current,
              lastUsage: agent.lastUsage,
            });
            return next;
          }
          return prev;
        }
        const next = new Map(prev);
        next.set(agent.id, agent);
        return next;
      });

      if (agent.archivedAt) {
        clearArchiveAgentPending({
          queryClient,
          serverId,
          agentId: agent.id,
        });
      }

      store.getState().setAgentLastActivity(agent.id, agent.lastActivityAt);

      store.getState().setPendingPermissions(serverId, (prev) => {
        const existingKeysForAgent: string[] = [];
        for (const [key, pending] of prev.entries()) {
          if (pending.agentId === agent.id) {
            existingKeysForAgent.push(key);
          }
        }

        const nextEntries = agent.pendingPermissions.map((request) => ({
          key: derivePendingPermissionKey(agent.id, request),
          agentId: agent.id,
          request,
        }));

        let changed = existingKeysForAgent.length !== nextEntries.length;
        if (!changed) {
          const existingKeySet = new Set(existingKeysForAgent);
          for (const entry of nextEntries) {
            const existing = prev.get(entry.key);
            if (!existingKeySet.has(entry.key) || !existing) {
              changed = true;
              break;
            }

            const currentRequest = existing.request;
            if (
              currentRequest.id !== entry.request.id ||
              currentRequest.kind !== entry.request.kind ||
              currentRequest.name !== entry.request.name ||
              currentRequest.title !== entry.request.title ||
              currentRequest.description !== entry.request.description
            ) {
              changed = true;
              break;
            }
          }
        }

        if (!changed) {
          return prev;
        }

        const next = new Map(prev);
        for (const key of existingKeysForAgent) {
          next.delete(key);
        }
        for (const entry of nextEntries) {
          next.set(entry.key, entry);
        }
        return next;
      });

      const prevStatus = previousAgentStatusRef.current.get(agent.id);
      if (prevStatus === "running" && agent.status !== "running") {
        const session = store.getState().sessions[serverId];
        const queue = session?.queuedMessages.get(agent.id);
        if (queue && queue.length > 0) {
          const [next, ...rest] = queue;
          if (sendAgentMessageRef.current) {
            const wirePayload = splitComposerAttachmentsForSubmit(next.attachments);
            void sendAgentMessageRef.current(
              agent.id,
              next.text,
              wirePayload.images,
              wirePayload.attachments,
            );
          }
          store.getState().setQueuedMessages(serverId, (prev) => {
            const updated = new Map(prev);
            updated.set(agent.id, rest);
            return updated;
          });
        }
      }

      previousAgentStatusRef.current.set(agent.id, agent.status);
    },
    [queryClient, serverId],
  );

  // -------------------------------------------------------------------------
  // Authoritative revalidation
  // -------------------------------------------------------------------------
  const runAuthoritativeRevalidation = useCallback(async () => {
    await Promise.all([
      getHostRuntimeStore().refreshAgentDirectory({ serverId }),
      hydrateWorkspaces(),
    ]);
  }, [hydrateWorkspaces, serverId]);

  const flushAuthoritativeRevalidation = useCallback(() => {
    if (!client || !isConnected) {
      return;
    }
    if (revalidationInFlightRef.current) {
      revalidationQueuedRef.current = true;
      return;
    }

    const run = runAuthoritativeRevalidation()
      .catch((error) => {
        console.error("[Session] authoritative revalidation failed", {
          serverId,
          error,
        });
      })
      .finally(() => {
        if (revalidationInFlightRef.current === run) {
          revalidationInFlightRef.current = null;
        }
        if (!revalidationQueuedRef.current) {
          return;
        }
        revalidationQueuedRef.current = false;
        if (revalidationTimerRef.current) {
          clearTimeout(revalidationTimerRef.current);
        }
        revalidationTimerRef.current = setTimeout(() => {
          revalidationTimerRef.current = null;
          flushAuthoritativeRevalidation();
        }, AUTHORITATIVE_REVALIDATION_DEBOUNCE_MS);
      });

    revalidationInFlightRef.current = run;
  }, [client, isConnected, runAuthoritativeRevalidation, serverId]);

  const scheduleAuthoritativeRevalidation = useCallback(() => {
    if (!client || !isConnected) {
      return;
    }

    revalidationQueuedRef.current = true;
    if (revalidationTimerRef.current) {
      return;
    }
    revalidationTimerRef.current = setTimeout(() => {
      revalidationTimerRef.current = null;
      if (!revalidationQueuedRef.current) {
        return;
      }
      revalidationQueuedRef.current = false;
      flushAuthoritativeRevalidation();
    }, AUTHORITATIVE_REVALIDATION_DEBOUNCE_MS);
  }, [client, flushAuthoritativeRevalidation, isConnected]);

  const handleAppResumed = useCallback(
    (awayMs: number) => {
      scheduleAuthoritativeRevalidation();

      if (isNative) {
        const session = store.getState().sessions[serverId];
        const agentId = session?.focusedAgentId;
        const cursor = agentId ? session?.agentTimelineCursor.get(agentId) : undefined;
        if (agentId && cursor) {
          void client
            .fetchAgentTimeline(agentId, {
              direction: "after",
              cursor: { epoch: cursor.epoch, seq: cursor.endSeq },
              limit: 0,
              projection: "canonical",
            })
            .catch((error) => {
              console.warn("[Session] failed to fetch catch-up timeline on resume", agentId, error);
            });
        }
      }

      if (awayMs < HISTORY_STALE_AFTER_MS) {
        return;
      }
      store.getState().bumpHistorySyncGeneration(serverId);
    },
    [client, scheduleAuthoritativeRevalidation, serverId],
  );

  // -------------------------------------------------------------------------
  // Client activity tracking (heartbeat, push token registration)
  // -------------------------------------------------------------------------
  useClientActivity({ client, focusedAgentId, onAppResumed: handleAppResumed });
  usePushTokenRegistration({ client, serverId });

  // -------------------------------------------------------------------------
  // Agent attention notification
  // -------------------------------------------------------------------------
  const notifyAgentAttention = useCallback(
    (params: {
      agentId: string;
      reason: "finished" | "error" | "permission";
      timestamp: string;
      notification?: AgentAttentionNotificationPayload;
    }) => {
      const appState = appStateRef.current;
      const session = store.getState().sessions[serverId];
      const focusedAgentId = session?.focusedAgentId ?? null;
      if (params.reason === "error") {
        return;
      }
      const isActivelyVisible = getIsAppActivelyVisible(appState);
      const isAwayFromAgent = !isActivelyVisible || focusedAgentId !== params.agentId;
      if (!isAwayFromAgent) {
        return;
      }

      const timestampMs = new Date(params.timestamp).getTime();
      const lastNotified = attentionNotifiedRef.current.get(params.agentId);
      if (lastNotified && lastNotified >= timestampMs) {
        return;
      }
      attentionNotifiedRef.current.set(params.agentId, timestampMs);

      const head = session?.agentStreamHead.get(params.agentId) ?? [];
      const tail = session?.agentStreamTail.get(params.agentId) ?? [];
      const assistantMessage =
        findLatestAssistantMessageText(head) ?? findLatestAssistantMessageText(tail);
      const permissionRequest = getLatestPermissionRequest(session, params.agentId);

      const notification =
        params.notification ??
        buildAgentAttentionNotificationPayload({
          reason: params.reason,
          serverId,
          agentId: params.agentId,
          assistantMessage: params.reason === "finished" ? assistantMessage : null,
          permissionRequest: params.reason === "permission" ? permissionRequest : null,
        });

      void sendOsNotification({
        title: notification.title,
        body: notification.body,
        data: notification.data,
      });
    },
    [serverId],
  );

  // -------------------------------------------------------------------------
  // Initialize session
  // -------------------------------------------------------------------------
  useEffect(() => {
    store.getState().initializeSession(serverId, client);
  }, [serverId, client]);

  useEffect(() => {
    store.getState().updateSessionClient(serverId, client);
  }, [serverId, client]);

  useEffect(() => {
    const serverInfo = client.getLastServerInfoMessage();
    if (!serverInfo) {
      return;
    }

    store.getState().updateSessionServerInfo(serverId, {
      serverId: serverInfo.serverId,
      hostname: serverInfo.hostname,
      version: serverInfo.version,
      ...(serverInfo.capabilities ? { capabilities: serverInfo.capabilities } : {}),
      ...(serverInfo.features ? { features: serverInfo.features } : {}),
    });
  }, [client, serverId]);

  useEffect(() => {
    if (!isConnected) {
      return;
    }

    const serverInfo = client.getLastServerInfoMessage();
    if (!serverInfo?.features?.providersSnapshot) {
      return;
    }

    prefetchProvidersSnapshot(serverId, client);
  }, [client, isConnected, serverId]);

  // -------------------------------------------------------------------------
  // Voice runtime
  // -------------------------------------------------------------------------
  useEffect(() => {
    if (!voiceRuntime) {
      return;
    }

    return voiceRuntime.registerSession({
      serverId,
      setVoiceMode: async (enabled, agentId) => {
        if (!client) {
          throw new Error("Daemon unavailable");
        }
        await client.setVoiceMode(enabled, agentId);
      },
      sendVoiceAudioChunk: async (audioData, mimeType) => {
        if (!client) {
          throw new Error("Daemon unavailable");
        }
        await client.sendVoiceAudioChunk(audioData, mimeType);
      },
      audioPlayed: async (chunkId) => {
        if (!client) {
          throw new Error("Daemon unavailable");
        }
        await client.audioPlayed(chunkId);
      },
      abortRequest: async () => {
        if (!client) {
          throw new Error("Daemon unavailable");
        }
        await client.abortRequest();
      },
      setAssistantAudioPlaying: (isPlaying) => {
        store.getState().setIsPlayingAudio(serverId, isPlaying);
      },
    });
  }, [client, serverId, voiceRuntime]);

  useEffect(() => {
    voiceRuntime?.updateSessionConnection(serverId, isConnected);
  }, [isConnected, serverId, voiceRuntime]);

  // If the client drops mid-initialization, clear pending flags
  useEffect(() => {
    if (!isConnected) {
      store.getState().flushAgentLastActivity();
      clearPendingAgentUpdates(serverId);
      store.getState().setInitializingAgents(serverId, new Map());
    }
  }, [serverId, isConnected]);

  // -------------------------------------------------------------------------
  // Hydrate workspaces on connection
  // -------------------------------------------------------------------------
  useEffect(() => {
    if (!client || !isConnected) {
      return;
    }

    let cancelled = false;

    // Phase 1: Load first page immediately for fast first paint
    void (async () => {
      try {
        await hydrateWorkspaces({
          subscribe: true,
          isCancelled: () => cancelled,
          firstPageOnly: true,
        });
      } catch (error) {
        console.error("[Session] Failed to hydrate first page:", error);
        return;
      }

      // Phase 2: Defer remaining pages to avoid blocking UI
      if (cancelled) return;
      if (!hydrationCursorRef.current) return; // All fit in one page

      requestAnimationFrame(() => {
        if (cancelled) return;
        void (async () => {
          try {
            await hydrateWorkspaces({
              isCancelled: () => cancelled,
            });
          } catch (error) {
            console.error("[Session] Failed to hydrate remaining pages:", error);
          }
        })();
      });
    })();

    return () => {
      cancelled = true;
    };
  }, [client, hydrateWorkspaces, isConnected]);

  // -------------------------------------------------------------------------
  // Agent update payload handler
  // -------------------------------------------------------------------------
  const applyAgentUpdatePayload = useCallback(
    (update: AgentUpdatePayload) => {
      if (update.kind === "remove") {
        const agentId = update.agentId;
        previousAgentStatusRef.current.delete(agentId);
        deletePendingAgentUpdate(serverId, agentId);
        clearArchiveAgentPending({ queryClient, serverId, agentId });

        store.getState().setAgents(serverId, (prev) => {
          if (!prev.has(agentId)) {
            return prev;
          }
          const next = new Map(prev);
          next.delete(agentId);
          return next;
        });

        store.getState().setPendingPermissions(serverId, (prev) => {
          if (prev.size === 0) {
            return prev;
          }
          let changed = false;
          const next = new Map(prev);
          for (const [key, pending] of Array.from(next.entries())) {
            if (pending.agentId === agentId) {
              next.delete(key);
              changed = true;
            }
          }
          return changed ? next : prev;
        });

        store.getState().setQueuedMessages(serverId, (prev) => {
          if (!prev.has(agentId)) {
            return prev;
          }
          const next = new Map(prev);
          next.delete(agentId);
          return next;
        });

        store.getState().setAgentTimelineCursor(serverId, (prev) => {
          if (!prev.has(agentId)) {
            return prev;
          }
          const next = new Map(prev);
          next.delete(agentId);
          return next;
        });
        store.getState().setAgentAuthoritativeHistoryApplied(serverId, agentId, false);
        return;
      }

      const normalized = normalizeAgentSnapshot(update.agent, serverId);
      const agent = {
        ...normalized,
        projectPlacement: resolveProjectPlacement({
          projectPlacement: update.project,
          cwd: normalized.cwd,
        }),
      };

      applyAuthoritativeAgentSnapshot(agent);
    },
    [applyAuthoritativeAgentSnapshot, queryClient, serverId],
  );

  const applyWorkspaceSetupProgress = useCallback(
    (payload: WorkspaceSetupProgressPayload) => {
      workspaceSetupStore.getState().upsertProgress({ serverId, payload });
    },
    [serverId],
  );

  const requestCanonicalCatchUp = useCallback(
    (agentId: string, cursor: { epoch: string; endSeq: number }) => {
      void client
        .fetchAgentTimeline(agentId, {
          direction: "after",
          cursor: { epoch: cursor.epoch, seq: cursor.endSeq },
          limit: 0,
          projection: "canonical",
        })
        .catch((error) => {
          console.warn("[Session] failed to fetch canonical catch-up timeline", agentId, error);
        });
    },
    [client],
  );

  const applyTimelineResponse = useCallback(
    (
      payload: Extract<
        SessionOutboundMessage,
        { type: "fetch_agent_timeline_response" }
      >["payload"],
    ) => {
      const agentId = payload.agentId;
      const initKey = getInitKey(serverId, agentId);
      const shouldMarkAuthoritativeHistoryApplied =
        payload.direction === "tail" || payload.direction === "after";

      const session = store.getState().sessions[serverId];
      const isInitializing = session?.initializingAgents.get(agentId) === true;
      const activeInitDeferred = getInitDeferred(initKey);
      const hasActiveInitDeferred = Boolean(activeInitDeferred);
      const currentCursor = session?.agentTimelineCursor.get(agentId);
      const currentTail = session?.agentStreamTail.get(agentId) ?? [];
      const currentHead = session?.agentStreamHead.get(agentId) ?? [];

      if (payload.agent) {
        const normalized = normalizeAgentSnapshot(payload.agent, serverId);
        applyAuthoritativeAgentSnapshot({
          ...normalized,
          projectPlacement: session?.agents.get(agentId)?.projectPlacement ?? null,
        });
      }

      const result = processTimelineResponse({
        payload,
        currentTail,
        currentHead,
        currentCursor,
        isInitializing,
        hasActiveInitDeferred,
        initRequestDirection: activeInitDeferred?.requestDirection ?? "tail",
      });

      if (result.error) {
        if (result.clearInitializing) {
          store.getState().setInitializingAgents(serverId, (prev) => {
            if (prev.get(agentId) !== true) {
              return prev;
            }
            const next = new Map(prev);
            next.set(agentId, false);
            return next;
          });
        }
        if (result.initResolution === "reject") {
          rejectInitDeferred(initKey, new Error(result.error));
        }
        return;
      }

      if (result.tail !== currentTail) {
        store.getState().setAgentStreamTail(serverId, (prev) => {
          const next = new Map(prev);
          next.set(agentId, result.tail);
          return next;
        });
      }

      if (result.head !== currentHead) {
        if (result.head.length === 0) {
          store.getState().clearAgentStreamHead(serverId, agentId);
        } else {
          store.getState().setAgentStreamHead(serverId, (prev) => {
            const next = new Map(prev);
            next.set(agentId, result.head);
            return next;
          });
        }
      }

      if (result.cursorChanged) {
        store.getState().setAgentTimelineCursor(serverId, (prev) => {
          const current = prev.get(agentId);
          if (!result.cursor) {
            if (!current) {
              return prev;
            }
            const next = new Map(prev);
            next.delete(agentId);
            return next;
          }
          if (
            current &&
            current.epoch === result.cursor.epoch &&
            current.startSeq === result.cursor.startSeq &&
            current.endSeq === result.cursor.endSeq
          ) {
            return prev;
          }
          const next = new Map(prev);
          next.set(agentId, result.cursor);
          return next;
        });
      }

      for (const effect of result.sideEffects) {
        if (effect.type === "catch_up") {
          requestCanonicalCatchUp(agentId, effect.cursor);
        } else if (effect.type === "flush_pending_updates") {
          const deferredUpdate = flushPendingAgentUpdate(serverId, agentId);
          if (deferredUpdate) {
            applyAgentUpdatePayload(deferredUpdate);
          }
        }
      }

      if (result.clearInitializing) {
        store.getState().setInitializingAgents(serverId, (prev) => {
          if (prev.get(agentId) !== true) {
            return prev;
          }
          const next = new Map(prev);
          next.set(agentId, false);
          return next;
        });
      }

      if (shouldMarkAuthoritativeHistoryApplied) {
        store.getState().setAgentAuthoritativeHistoryApplied(serverId, agentId, true);
      }
      if (result.initResolution === "resolve") {
        resolveInitDeferred(initKey);
      }
      if (result.clearInitializing) {
        store.getState().markAgentHistorySynchronized(serverId, agentId);
      }
    },
    [applyAuthoritativeAgentSnapshot, applyAgentUpdatePayload, requestCanonicalCatchUp, serverId],
  );

  useEffect(() => {
    if (isConnected) {
      return;
    }
    clearPendingAgentUpdates(serverId);
  }, [isConnected, serverId]);

  useEffect(() => {
    const wasConnected = wasConnectedRef.current;
    wasConnectedRef.current = isConnected;
    if (!wasConnected && isConnected) {
      scheduleAuthoritativeRevalidation();
    }
  }, [isConnected, scheduleAuthoritativeRevalidation]);

  useEffect(() => {
    return () => {
      if (revalidationTimerRef.current) {
        clearTimeout(revalidationTimerRef.current);
      }
    };
  }, []);

  // -------------------------------------------------------------------------
  // Daemon message handlers
  // -------------------------------------------------------------------------
  useEffect(() => {
    const unsubAgentUpdate = client.on("agent_update", (message) => {
      if (message.type !== "agent_update") return;
      const update = message.payload;
      const agentId = getAgentIdFromUpdate(update);
      const initKey = getInitKey(serverId, agentId);
      const session = store.getState().sessions[serverId];
      const isSyncingHistory =
        session?.initializingAgents.get(agentId) === true && Boolean(getInitDeferred(initKey));

      if (isSyncingHistory) {
        bufferPendingAgentUpdate(serverId, agentId, update);
        return;
      }

      deletePendingAgentUpdate(serverId, agentId);
      applyAgentUpdatePayload(update);
    });

    const unsubAgentStream = client.on("agent_stream", (message) => {
      if (message.type !== "agent_stream") return;
      const { agentId, event, timestamp, seq, epoch } = message.payload;
      const parsedTimestamp = new Date(timestamp);
      const streamEvent = event as AgentStreamEventPayload;
      if (
        event.type === "turn_started" ||
        event.type === "turn_completed" ||
        event.type === "turn_failed" ||
        event.type === "turn_canceled"
      ) {
        voiceRuntime?.onTurnEvent(serverId, agentId, event.type);
      }

      if (
        event.type === "turn_completed" ||
        event.type === "turn_failed" ||
        event.type === "turn_canceled"
      ) {
        throttledBalanceRefresh(queryClient);
      }

      if (event.type === "attention_required") {
        if (event.shouldNotify) {
          notifyAgentAttention({
            agentId,
            reason: event.reason,
            timestamp: event.timestamp,
            notification: event.notification,
          });
        }
      }

      const session = store.getState().sessions[serverId];
      const currentTail = session?.agentStreamTail.get(agentId) ?? [];
      const currentHead = session?.agentStreamHead.get(agentId) ?? [];
      const currentCursor = session?.agentTimelineCursor.get(agentId);
      const currentAgentEntry = session?.agents.get(agentId);
      const currentAgent = currentAgentEntry
        ? {
            status: currentAgentEntry.status,
            updatedAt: currentAgentEntry.updatedAt,
            lastActivityAt: currentAgentEntry.lastActivityAt,
          }
        : null;

      const result = processAgentStreamEvent({
        event: streamEvent,
        seq,
        epoch,
        currentTail,
        currentHead,
        currentCursor,
        currentAgent,
        timestamp: parsedTimestamp,
      });

      if (result.changedTail || result.changedHead) {
        store.getState().setAgentStreamState(serverId, agentId, {
          ...(result.changedTail ? { tail: result.tail } : {}),
          ...(result.changedHead ? { head: result.head } : {}),
        });
      }

      if (result.cursorChanged && result.cursor) {
        const nextCursor = result.cursor;
        store.getState().setAgentTimelineCursor(serverId, (prev) => {
          const current = prev.get(agentId);
          if (
            current &&
            typeof seq === "number" &&
            typeof epoch === "string" &&
            current.epoch === epoch &&
            seq >= current.startSeq &&
            seq <= current.endSeq
          ) {
            return prev;
          }
          if (
            current &&
            current.epoch === nextCursor.epoch &&
            current.startSeq === nextCursor.startSeq &&
            current.endSeq === nextCursor.endSeq
          ) {
            return prev;
          }
          const next = new Map(prev);
          next.set(agentId, nextCursor);
          return next;
        });
      }

      if (result.agentChanged && result.agent) {
        const nextAgent = result.agent;
        store.getState().setAgents(serverId, (prev) => {
          const current = prev.get(agentId);
          if (!current) {
            return prev;
          }
          const next = new Map(prev);
          next.set(agentId, {
            ...current,
            status: nextAgent.status,
            updatedAt: nextAgent.updatedAt,
            lastActivityAt: nextAgent.lastActivityAt,
          });
          return next;
        });
      }

      for (const effect of result.sideEffects) {
        if (effect.type === "catch_up") {
          requestCanonicalCatchUp(agentId, effect.cursor);
        }
      }
    });

    const unsubAgentTimeline = client.on("fetch_agent_timeline_response", (message) => {
      if (message.type !== "fetch_agent_timeline_response") return;
      applyTimelineResponse(message.payload);
    });

    const unsubWorkspaceUpdate = client.on("workspace_update", (message) => {
      if (message.type !== "workspace_update") return;
      if (message.payload.kind === "remove") {
        workspaceSetupStore.getState().removeWorkspace({
          serverId,
          workspaceId: String(message.payload.id),
        });
        store.getState().removeWorkspace(serverId, String(message.payload.id));
        return;
      }
      const workspace = normalizeWorkspaceDescriptor(message.payload.workspace);
      store.getState().mergeWorkspaces(serverId, [workspace]);
    });

    const unsubScriptStatusUpdate = client.on("script_status_update", (message) => {
      if (message.type !== "script_status_update") return;
      store
        .getState()
        .setWorkspaces(serverId, (prev) => patchWorkspaceScripts(prev, message.payload));
    });

    const unsubWorkspaceSetupProgress = client.on("workspace_setup_progress", (message) => {
      if (message.type !== "workspace_setup_progress") return;
      applyWorkspaceSetupProgress(message.payload);
    });

    const unsubWorkspaceSetupStatusResponse = client.on(
      "workspace_setup_status_response",
      (message) => {
        if (message.type !== "workspace_setup_status_response") return;
        const { workspaceId, snapshot } = message.payload;
        if (snapshot) {
          applyWorkspaceSetupProgress({ workspaceId, ...snapshot });
        }
      },
    );

    const unsubStatus = client.on("status", (message) => {
      if (message.type !== "status") return;
      const serverInfo = parseServerInfoStatusPayload(message.payload);
      if (serverInfo) {
        store.getState().updateSessionServerInfo(serverId, {
          serverId: serverInfo.serverId,
          hostname: serverInfo.hostname,
          version: serverInfo.version,
          ...(serverInfo.capabilities ? { capabilities: serverInfo.capabilities } : {}),
          ...(serverInfo.features ? { features: serverInfo.features } : {}),
        });
        return;
      }
    });

    const unsubPermissionRequest = client.on("agent_permission_request", (message) => {
      if (message.type !== "agent_permission_request") return;
      const { agentId, request } = message.payload;

      store.getState().setPendingPermissions(serverId, (prev) => {
        const next = new Map(prev);
        const key = derivePendingPermissionKey(agentId, request);
        next.set(key, { key, agentId, request });
        return next;
      });
    });

    const unsubPermissionResolved = client.on("agent_permission_resolved", (message) => {
      if (message.type !== "agent_permission_resolved") return;
      const { requestId, agentId } = message.payload;

      store.getState().setPendingPermissions(serverId, (prev) => {
        const next = new Map(prev);
        const derivedKey = `${agentId}:${requestId}`;
        if (!next.delete(derivedKey)) {
          for (const [key, pending] of next.entries()) {
            if (pending.agentId === agentId && pending.request.id === requestId) {
              next.delete(key);
              break;
            }
          }
        }
        return next;
      });
    });

    const unsubAudioOutput = client.on("audio_output", async (message) => {
      if (message.type !== "audio_output") return;
      if (!voiceAudioEngine) {
        return;
      }

      const payload = message.payload;
      if (payload.isVoiceMode && voiceRuntime) {
        voiceRuntime.handleAudioOutput(serverId, payload);
        return;
      }

      const playbackGroupId = payload.groupId ?? payload.id;
      const chunkIndex = payload.chunkIndex ?? 0;
      const isFinalChunk = payload.isLastChunk ?? true;

      if (!audioOutputBuffersRef.current.has(playbackGroupId)) {
        audioOutputBuffersRef.current.set(playbackGroupId, []);
      }

      const bufferedChunks = audioOutputBuffersRef.current.get(playbackGroupId)!;
      bufferedChunks.push({
        chunkIndex,
        audio: payload.audio,
        format: payload.format,
        id: payload.id,
      });

      activeAudioGroupsRef.current.add(playbackGroupId);
      store.getState().setIsPlayingAudio(serverId, true);

      if (!isFinalChunk) {
        return;
      }

      bufferedChunks.sort((left, right) => left.chunkIndex - right.chunkIndex);
      const chunkIds = bufferedChunks.map((chunk) => chunk.id);
      const shouldPlay =
        !payload.isVoiceMode || (voiceRuntime?.shouldPlayVoiceAudio(serverId) ?? false);
      const audioBlob = buildAudioPlaybackSource(bufferedChunks);
      const confirmAudioPlayed = async () => {
        await Promise.all(
          chunkIds.map((chunkId) =>
            client.audioPlayed(chunkId).catch((error) => {
              console.warn("[Session] Failed to confirm audio playback:", error);
            }),
          ),
        );
      };

      let startedVoicePlayback = false;
      try {
        if (shouldPlay) {
          if (payload.isVoiceMode) {
            startedVoicePlayback = true;
            voiceRuntime?.onAssistantAudioStarted(serverId);
          }
          await voiceAudioEngine.play(audioBlob);
        }
        await confirmAudioPlayed();
      } catch (error) {
        console.error("[Session] Audio playback error:", error);
        await confirmAudioPlayed();
      } finally {
        audioOutputBuffersRef.current.delete(playbackGroupId);
        activeAudioGroupsRef.current.delete(playbackGroupId);
        store.getState().setIsPlayingAudio(serverId, activeAudioGroupsRef.current.size > 0);

        if (startedVoicePlayback) {
          voiceRuntime?.onAssistantAudioFinished(serverId);
        }
      }
    });

    const unsubActivity = client.on("activity_log", (message) => {
      if (message.type !== "activity_log") return;
      const data = message.payload;
      if (data.type === "system" && data.content.includes("Transcribing")) {
        return;
      }

      if (data.type === "tool_call" && data.metadata) {
        const {
          toolCallId,
          toolName,
          arguments: args,
        } = data.metadata as {
          toolCallId: string;
          toolName: string;
          arguments: unknown;
        };

        store.getState().setMessages(serverId, (prev) => [
          ...prev,
          {
            type: "tool_call",
            id: toolCallId,
            timestamp: Date.now(),
            toolName,
            args,
            status: "executing",
          },
        ]);
        return;
      }

      if (data.type === "tool_result" && data.metadata) {
        const { toolCallId, result } = data.metadata as {
          toolCallId: string;
          result: unknown;
        };

        store
          .getState()
          .setMessages(serverId, (prev) =>
            prev.map((msg) =>
              msg.type === "tool_call" && msg.id === toolCallId
                ? { ...msg, result, status: "completed" as const }
                : msg,
            ),
          );
        return;
      }

      if (data.type === "error" && data.metadata && "toolCallId" in data.metadata) {
        const { toolCallId, error } = data.metadata as {
          toolCallId: string;
          error: unknown;
        };

        store
          .getState()
          .setMessages(serverId, (prev) =>
            prev.map((msg) =>
              msg.type === "tool_call" && msg.id === toolCallId
                ? { ...msg, error, status: "failed" as const }
                : msg,
            ),
          );
      }

      let activityType: "system" | "info" | "success" | "error" = "info";
      if (data.type === "error") activityType = "error";

      if (data.type === "transcript") {
        store.getState().setMessages(serverId, (prev) => [
          ...prev,
          {
            type: "user",
            id: generateMessageId(),
            timestamp: Date.now(),
            message: data.content,
          },
        ]);
        return;
      }

      if (data.type === "assistant") {
        store.getState().setMessages(serverId, (prev) => [
          ...prev,
          {
            type: "assistant",
            id: generateMessageId(),
            timestamp: Date.now(),
            message: data.content,
          },
        ]);
        store.getState().setCurrentAssistantMessage(serverId, "");
        return;
      }

      store.getState().setMessages(serverId, (prev) => [
        ...prev,
        {
          type: "activity",
          id: generateMessageId(),
          timestamp: Date.now(),
          activityType,
          message: data.content,
          metadata: data.metadata,
        },
      ]);
    });

    const unsubChunk = client.on("assistant_chunk", (message) => {
      if (message.type !== "assistant_chunk") return;
      store.getState().setCurrentAssistantMessage(serverId, (prev) => prev + message.payload.chunk);
    });

    const unsubTranscription = client.on("transcription_result", (message) => {
      if (message.type !== "transcription_result") return;

      const transcriptText = message.payload.text.trim();
      voiceRuntime?.onTranscriptionResult(serverId, transcriptText);
      if (!transcriptText) {
        return;
      }

      store.getState().setCurrentAssistantMessage(serverId, "");
    });

    const unsubVoiceInputState = client.on("voice_input_state", (message) => {
      if (message.type !== "voice_input_state") return;
      voiceRuntime?.onServerSpeechStateChanged(serverId, message.payload.isSpeaking);
    });

    const unsubAgentDeleted = client.on("agent_deleted", (message) => {
      if (message.type !== "agent_deleted") {
        return;
      }
      const { agentId } = message.payload;
      deletePendingAgentUpdate(serverId, agentId);
      clearArchiveAgentPending({ queryClient, serverId, agentId });

      // Clean up module-level Map refs to prevent memory leak
      previousAgentStatusRef.current.delete(agentId);
      attentionNotifiedRef.current.delete(agentId);
      audioOutputBuffersRef.current.delete(agentId);

      store.getState().setAgents(serverId, (prev) => {
        if (!prev.has(agentId)) {
          return prev;
        }
        const next = new Map(prev);
        next.delete(agentId);
        return next;
      });

      store.setState((state) => {
        if (!state.agentLastActivity.has(agentId)) {
          return state;
        }
        const nextActivity = new Map(state.agentLastActivity);
        nextActivity.delete(agentId);
        return {
          ...state,
          agentLastActivity: nextActivity,
        };
      });

      store.getState().setAgentStreamTail(serverId, (prev) => {
        if (!prev.has(agentId)) {
          return prev;
        }
        const next = new Map(prev);
        next.delete(agentId);
        return next;
      });
      store.getState().clearAgentStreamHead(serverId, agentId);
      store.getState().setAgentTimelineCursor(serverId, (prev) => {
        if (!prev.has(agentId)) {
          return prev;
        }
        const next = new Map(prev);
        next.delete(agentId);
        return next;
      });

      draftStore.getState().clearDraftInput({
        draftKey: buildDraftStoreKey({ serverId, agentId }),
      });

      store.getState().setPendingPermissions(serverId, (prev) => {
        let changed = false;
        const next = new Map(prev);
        for (const [key, pending] of prev.entries()) {
          if (pending.agentId === agentId) {
            next.delete(key);
            changed = true;
          }
        }
        return changed ? next : prev;
      });

      store.getState().setInitializingAgents(serverId, (prev) => {
        if (!prev.has(agentId)) {
          return prev;
        }
        const next = new Map(prev);
        next.delete(agentId);
        return next;
      });
    });

    const unsubAgentArchived = client.on("agent_archived", (message) => {
      if (message.type !== "agent_archived") {
        return;
      }
      const { agentId, archivedAt } = message.payload;
      clearArchiveAgentPending({ queryClient, serverId, agentId });

      store.getState().setAgents(serverId, (prev) => {
        const existing = prev.get(agentId);
        if (!existing) {
          return prev;
        }
        const next = new Map(prev);
        next.set(agentId, {
          ...existing,
          archivedAt: new Date(archivedAt),
        });
        return next;
      });
    });

    return () => {
      unsubAgentUpdate();
      unsubAgentStream();
      unsubAgentTimeline();
      unsubWorkspaceUpdate();
      unsubScriptStatusUpdate();
      unsubWorkspaceSetupProgress();
      unsubWorkspaceSetupStatusResponse();
      unsubStatus();
      unsubPermissionRequest();
      unsubPermissionResolved();
      unsubAudioOutput();
      unsubActivity();
      unsubChunk();
      unsubTranscription();
      unsubVoiceInputState();
      unsubAgentDeleted();
      unsubAgentArchived();
    };
  }, [
    client,
    queryClient,
    serverId,
    notifyAgentAttention,
    requestCanonicalCatchUp,
    applyAgentUpdatePayload,
    applyWorkspaceSetupProgress,
    applyTimelineResponse,
    voiceRuntime,
    voiceAudioEngine,
  ]);

  // -------------------------------------------------------------------------
  // Agent actions (exported to consumers)
  // -------------------------------------------------------------------------
  const sendAgentMessage = useCallback(
    async (
      agentId: string,
      message: string,
      images?: AttachmentMetadata[],
      attachments?: AgentAttachment[],
      options?: { hidden?: boolean },
    ) => {
      const messageId = generateMessageId();
      if (!options?.hidden) {
        const userMessage: StreamItem = {
          kind: "user_message",
          id: messageId,
          text: message,
          timestamp: new Date(),
        };

        const currentHead = store.getState().sessions[serverId]?.agentStreamHead?.get(agentId);
        if (currentHead && currentHead.length > 0) {
          store.getState().setAgentStreamHead(serverId, (prev) => {
            const head = prev.get(agentId) || [];
            const updated = new Map(prev);
            updated.set(agentId, [...head, userMessage]);
            return updated;
          });
        } else {
          store.getState().setAgentStreamTail(serverId, (prev) => {
            const currentStream = prev.get(agentId) || [];
            const updated = new Map(prev);
            updated.set(agentId, [...currentStream, userMessage]);
            return updated;
          });
        }
      }

      const imagesData = await encodeImages(images);
      if (!client) {
        console.warn("[Session] sendAgentMessage skipped: daemon unavailable");
        return;
      }
      void client
        .sendAgentMessage(agentId, message, {
          messageId,
          ...(options?.hidden ? { hidden: true } : {}),
          ...(imagesData && imagesData.length > 0 ? { images: imagesData } : {}),
          ...(attachments && attachments.length > 0 ? { attachments } : {}),
        })
        .catch((error) => {
          console.error("[Session] Failed to send agent message:", error);
        });
    },
    [encodeImages, serverId, client],
  );

  sendAgentMessageRef.current = sendAgentMessage;

  const cancelAgentRun = useCallback(
    (agentId: string) => {
      if (!client) {
        console.warn("[Session] cancelAgent skipped: daemon unavailable");
        return;
      }
      void client.cancelAgent(agentId).catch((error) => {
        console.error("[Session] Failed to cancel agent:", error);
      });
    },
    [client],
  );

  const deleteAgent = useCallback(
    (agentId: string) => {
      if (!client) {
        console.warn("[Session] deleteAgent skipped: daemon unavailable");
        return;
      }
      void client.deleteAgent(agentId).catch((error) => {
        console.error("[Session] Failed to delete agent:", error);
      });
    },
    [client],
  );

  const archiveAgent = useCallback(
    (agentId: string) => {
      if (!client) {
        console.warn("[Session] archiveAgent skipped: daemon unavailable");
        return;
      }
      void client.archiveAgent(agentId).catch((error) => {
        console.error("[Session] Failed to archive agent:", error);
      });
    },
    [client],
  );

  const restartServer = useCallback(
    (reason?: string) => {
      if (!client) {
        console.warn("[Session] restartServer skipped: daemon unavailable");
        return;
      }
      void client.restartServer(reason).catch((error) => {
        console.error("[Session] Failed to restart server:", error);
      });
    },
    [client],
  );

  const createAgent = useCallback(
    async ({
      config,
      initialPrompt,
      images,
      attachments,
      git,
      worktreeName,
      requestId,
    }: {
      config: any;
      initialPrompt: string;
      images?: AttachmentMetadata[];
      attachments?: AgentAttachment[];
      git?: any;
      worktreeName?: string;
      requestId?: string;
    }) => {
      if (!client) {
        console.warn("[Session] createAgent skipped: daemon unavailable");
        return;
      }
      const trimmedPrompt = initialPrompt.trim();
      let imagesData: Array<{ data: string; mimeType: string }> | undefined;
      try {
        imagesData = await encodeImages(images);
      } catch (error) {
        console.error("[Session] Failed to prepare images for agent creation:", error);
      }
      return client.createAgent({
        config,
        ...(trimmedPrompt ? { initialPrompt: trimmedPrompt } : {}),
        ...(imagesData && imagesData.length > 0 ? { images: imagesData } : {}),
        ...(attachments && attachments.length > 0 ? { attachments } : {}),
        ...(git ? { git } : {}),
        ...(worktreeName ? { worktreeName } : {}),
        ...(requestId ? { requestId } : {}),
      });
    },
    [encodeImages, client],
  );

  const setAgentMode = useCallback(
    (agentId: string, modeId: string) => {
      if (!client) {
        console.warn("[Session] setAgentMode skipped: daemon unavailable");
        return;
      }
      void client.setAgentMode(agentId, modeId).catch((error) => {
        console.error("[Session] Failed to set agent mode:", error);
        toast.error(toErrorMessage(error));
      });
    },
    [client, toast],
  );

  const setAgentModel = useCallback(
    (agentId: string, modelId: string | null) => {
      if (!client) {
        console.warn("[Session] setAgentModel skipped: daemon unavailable");
        return;
      }
      void client.setAgentModel(agentId, modelId).catch((error) => {
        console.error("[Session] Failed to set agent model:", error);
        toast.error(toErrorMessage(error));
      });
    },
    [client, toast],
  );

  const setAgentThinkingOption = useCallback(
    (agentId: string, thinkingOptionId: string | null) => {
      if (!client) {
        console.warn("[Session] setAgentThinkingOption skipped: daemon unavailable");
        return;
      }
      void client.setAgentThinkingOption(agentId, thinkingOptionId).catch((error) => {
        console.error("[Session] Failed to set agent thinking option:", error);
        toast.error(toErrorMessage(error));
      });
    },
    [client, toast],
  );

  const respondToPermission = useCallback(
    (agentId: string, requestId: string, response: any) => {
      if (!client) {
        console.warn("[Session] respondToPermission skipped: daemon unavailable");
        return;
      }
      void client.respondToPermission(agentId, requestId, response).catch((error) => {
        console.error("[Session] Failed to respond to permission:", error);
      });
    },
    [client],
  );

  // -------------------------------------------------------------------------
  // Cleanup on unmount
  // -------------------------------------------------------------------------
  useEffect(() => {
    return () => {
      // Clear all Map refs to prevent memory leaks
      previousAgentStatusRef.current.clear();
      attentionNotifiedRef.current.clear();
      audioOutputBuffersRef.current.clear();
      activeAudioGroupsRef.current.clear();
      if (revalidationTimerRef.current) {
        clearTimeout(revalidationTimerRef.current);
        revalidationTimerRef.current = null;
      }
      workspaceSetupStore.getState().clearServer(serverId);
      store.getState().clearSession(serverId);
    };
  }, [serverId]);

  return children;
}

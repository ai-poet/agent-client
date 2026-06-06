import type { CommitGraphResponse } from "@server/shared/messages";
import { type QueryClient, useQuery, useQueryClient } from "@tanstack/react-query";
import { useHostRuntimeClient, useHostRuntimeIsConnected } from "@/runtime/host-runtime";

export const COMMIT_GRAPH_STALE_TIME = 30_000;

export function commitGraphQueryKey(serverId: string, cwd: string) {
  return ["commitGraph", serverId, cwd] as const;
}

interface UseCommitGraphQueryOptions {
  serverId: string;
  cwd: string;
  limit?: number;
}

export type CommitGraphPayload = CommitGraphResponse["payload"];

interface CommitGraphClient {
  getCommitGraph: (options: { cwd: string; limit?: number }) => Promise<CommitGraphPayload>;
}

function normalizeCommitGraphError(error: unknown): Error {
  const rpcError = error as Error & { code?: unknown; requestType?: unknown };
  if (
    rpcError?.name === "DaemonRpcError" &&
    rpcError.code === "unknown_schema" &&
    rpcError.requestType === "commit_graph_request"
  ) {
    return new Error("Commit graph requires a newer Paseo daemon. Update or restart the daemon.");
  }

  return error instanceof Error ? error : new Error(String(error));
}

function fetchCommitGraph(
  client: CommitGraphClient,
  cwd: string,
  limit?: number,
): Promise<CommitGraphPayload> {
  return client
    .getCommitGraph({ cwd, limit })
    .then((payload) => {
      if (payload.error) {
        throw new Error(payload.error);
      }
      return payload;
    })
    .catch((error) => {
      throw normalizeCommitGraphError(error);
    });
}

async function peekOrFetchSnapshot({
  queryClient,
  client,
  serverId,
  cwd,
  limit,
}: {
  queryClient: QueryClient;
  client: CommitGraphClient;
  serverId: string;
  cwd: string;
  limit?: number;
}): Promise<CommitGraphPayload> {
  const queryKey = commitGraphQueryKey(serverId, cwd);
  const cached = queryClient.getQueryData<CommitGraphPayload>(queryKey);
  if (cached) {
    if (cached.error) {
      throw new Error(cached.error);
    }
    return cached;
  }

  const snapshot = await fetchCommitGraph(client, cwd, limit);
  queryClient.setQueryData(queryKey, snapshot);
  return snapshot;
}

export function useCommitGraphQuery({ serverId, cwd, limit }: UseCommitGraphQueryOptions) {
  const queryClient = useQueryClient();
  const client = useHostRuntimeClient(serverId);
  const isConnected = useHostRuntimeIsConnected(serverId);

  const query = useQuery({
    queryKey: commitGraphQueryKey(serverId, cwd),
    queryFn: async () => {
      if (!client) {
        throw new Error("Daemon client not available");
      }
      return await peekOrFetchSnapshot({ queryClient, client, serverId, cwd, limit });
    },
    enabled: !!client && isConnected && !!cwd,
    staleTime: COMMIT_GRAPH_STALE_TIME,
    refetchOnMount: true,
    refetchOnReconnect: true,
    refetchOnWindowFocus: false,
  });

  const payloadError = query.data?.error ?? null;
  const error =
    query.error !== null
      ? normalizeCommitGraphError(query.error)
      : payloadError
        ? new Error(payloadError)
        : null;

  return {
    graph: payloadError ? null : (query.data?.graph ?? null),
    isLoading: query.isLoading,
    isFetching: query.isFetching,
    isError: query.isError || payloadError !== null,
    error,
    refetch: query.refetch,
  };
}

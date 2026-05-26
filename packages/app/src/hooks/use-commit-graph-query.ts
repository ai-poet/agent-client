import { type QueryClient, useQuery, useQueryClient } from "@tanstack/react-query";
import { useHostRuntimeClient, useHostRuntimeIsConnected } from "@/runtime/host-runtime";
import type { CommitGraphResponse } from "@server/shared/messages";

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

function fetchCommitGraph(
  client: CommitGraphClient,
  cwd: string,
  limit?: number,
): Promise<CommitGraphPayload> {
  return client.getCommitGraph({ cwd, limit });
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
    refetchOnMount: false,
    refetchOnReconnect: false,
    refetchOnWindowFocus: false,
  });

  return {
    graph: query.data?.graph ?? null,
    isLoading: query.isLoading,
    isFetching: query.isFetching,
    isError: query.isError,
    error: query.error,
  };
}

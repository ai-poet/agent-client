import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { useHostRuntimeClient } from "@/runtime/host-runtime";
import type { ScheduleCadence } from "@/utils/schedule-format";

export interface AutomationSummary {
  id: string;
  name: string | null;
  prompt: string;
  cadence: ScheduleCadence;
  status: "active" | "paused" | "completed";
  nextRunAt: string | null;
  lastRunAt: string | null;
}

interface CreateAutomationInput {
  name: string | null;
  prompt: string;
  cadence: ScheduleCadence;
  provider: string;
  cwd: string;
}

export function useAutomations(serverId: string) {
  const client = useHostRuntimeClient(serverId);
  const queryClient = useQueryClient();
  const queryKey = ["automations", serverId];

  const query = useQuery({
    queryKey,
    enabled: Boolean(client),
    staleTime: 15_000,
    queryFn: async (): Promise<AutomationSummary[]> => {
      if (!client) {
        return [];
      }
      const payload = await client.scheduleList();
      if (payload.error) {
        throw new Error(payload.error);
      }
      return payload.schedules as AutomationSummary[];
    },
  });

  const invalidate = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey });
  }, [queryClient, serverId]);

  const create = useMutation({
    mutationFn: async (input: CreateAutomationInput) => {
      if (!client) {
        throw new Error("Not connected");
      }
      const payload = await client.scheduleCreate({
        prompt: input.prompt,
        name: input.name,
        cadence: input.cadence,
        target: {
          type: "new-agent",
          config: { provider: input.provider, cwd: input.cwd },
        },
      });
      if (payload.error) {
        throw new Error(payload.error);
      }
    },
    onSuccess: invalidate,
  });

  // Pause/resume persist immediately — a toggle that only takes effect on a later "save"
  // is a reliable way to make someone think a run was cancelled when it wasn't.
  const setPaused = useMutation({
    mutationFn: async (input: { id: string; paused: boolean }) => {
      if (!client) {
        throw new Error("Not connected");
      }
      const payload = input.paused
        ? await client.schedulePause({ id: input.id })
        : await client.scheduleResume({ id: input.id });
      if (payload.error) {
        throw new Error(payload.error);
      }
    },
    onSuccess: invalidate,
  });

  const remove = useMutation({
    mutationFn: async (id: string) => {
      if (!client) {
        throw new Error("Not connected");
      }
      const payload = await client.scheduleDelete({ id });
      if (payload.error) {
        throw new Error(payload.error);
      }
    },
    onSuccess: invalidate,
  });

  return {
    automations: query.data ?? [],
    isLoading: query.isLoading,
    isFetching: query.isFetching,
    error: query.error instanceof Error ? query.error.message : null,
    isConnected: Boolean(client),
    create,
    setPaused,
    remove,
    refetch: query.refetch,
  };
}

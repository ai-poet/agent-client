import { useState, useCallback, useMemo } from "react";
import { useQuery, type QueryClient } from "@tanstack/react-query";
import type { DaemonClient } from "@server/client/daemon-client";
import type { ComboboxOption } from "@/components/ui/combobox";
import type { ToastApi } from "@/components/toast-host";
import { invalidateCheckoutGitQueriesForClient } from "@/stores/checkout-git-actions-store";
import { confirmDialog } from "@/utils/confirm-dialog";
import type { getAppMessages } from "@/i18n/sub2api";

interface UseBranchSwitcherInput {
  client: DaemonClient | null;
  normalizedServerId: string;
  normalizedWorkspaceId: string;
  currentBranchName: string | null;
  isGitCheckout: boolean;
  isConnected: boolean;
  toast: ToastApi;
  queryClient: QueryClient;
  text: ReturnType<typeof getAppMessages>["workspace"];
}

interface UseBranchSwitcherResult {
  branchOptions: ComboboxOption[];
  isOpen: boolean;
  setIsOpen: (open: boolean) => void;
  handleBranchSelect: (branchId: string) => void;
  invalidateStashAndCheckout: () => Promise<void>;
}

export function useBranchSwitcher({
  client,
  normalizedServerId,
  normalizedWorkspaceId,
  currentBranchName,
  isGitCheckout,
  isConnected,
  toast,
  queryClient,
  text,
}: UseBranchSwitcherInput): UseBranchSwitcherResult {
  const [isOpen, setIsOpen] = useState(false);

  const branchSuggestionsQuery = useQuery({
    queryKey: ["branchSuggestions", normalizedServerId, normalizedWorkspaceId],
    queryFn: async () => {
      if (!client) {
        throw new Error("Daemon client unavailable");
      }
      const payload = await client.getBranchSuggestions({
        cwd: normalizedWorkspaceId,
        limit: 200,
      });
      if (payload.error) {
        throw new Error(payload.error);
      }
      return payload.branches ?? [];
    },
    enabled: isOpen && isGitCheckout && Boolean(client) && isConnected,
    retry: false,
    staleTime: 15_000,
  });

  const branchOptions = useMemo<ComboboxOption[]>(() => {
    const branches = branchSuggestionsQuery.data ?? [];
    return branches.map((name) => ({ id: name, label: name }));
  }, [branchSuggestionsQuery.data]);

  const stashListQueryKey = useMemo(
    () => ["stashList", normalizedServerId, normalizedWorkspaceId] as const,
    [normalizedServerId, normalizedWorkspaceId],
  );

  const invalidateStashAndCheckout = useCallback(async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: stashListQueryKey }),
      invalidateCheckoutGitQueriesForClient(queryClient, {
        serverId: normalizedServerId,
        cwd: normalizedWorkspaceId,
      }),
    ]);
  }, [queryClient, stashListQueryKey, normalizedServerId, normalizedWorkspaceId]);

  const stashAndSwitch = useCallback(
    async (branchId: string) => {
      if (!client) return;
      const shouldStash = await confirmDialog({
        title: text.uncommittedChangesTitle,
        message: text.stashBeforeSwitchMessage,
        confirmLabel: text.stashAndSwitch,
        cancelLabel: text.close,
      });
      if (!shouldStash) return;

      try {
        const stashPayload = await client.stashSave(normalizedWorkspaceId, {
          branch: currentBranchName ?? undefined,
        });
        if (stashPayload.error) {
          toast.error(stashPayload.error.message);
          return;
        }
        await invalidateStashAndCheckout();
        const switchPayload = await client.checkoutSwitchBranch(normalizedWorkspaceId, branchId);
        if (switchPayload.error) {
          toast.error(switchPayload.error.message);
          return;
        }
        await invalidateStashAndCheckout();
      } catch (err) {
        toast.error(err instanceof Error ? err.message : text.failedStashChanges);
      }
    },
    [
      client,
      currentBranchName,
      invalidateStashAndCheckout,
      normalizedWorkspaceId,
      text.close,
      text.failedStashChanges,
      text.stashAndSwitch,
      text.stashBeforeSwitchMessage,
      text.uncommittedChangesTitle,
      toast,
    ],
  );

  const handleBranchSelect = useCallback(
    (branchId: string) => {
      if (branchId === currentBranchName) return;

      void (async () => {
        if (!client) return;
        try {
          const payload = await client.checkoutSwitchBranch(normalizedWorkspaceId, branchId);
          if (payload.error) {
            // If the error is about uncommitted changes, offer the stash dialog
            if (payload.error.message.toLowerCase().includes("uncommitted")) {
              await stashAndSwitch(branchId);
              return;
            }
            toast.error(payload.error.message);
            return;
          }
          // Success — refresh and check for stashes on the target branch
          await invalidateStashAndCheckout();
          try {
            const stashPayload = await client.stashList(normalizedWorkspaceId, { paseoOnly: true });
            const targetStash = stashPayload.entries.find((e) => e.branch === branchId);
            if (targetStash) {
              const shouldRestore = await confirmDialog({
                title: text.restoreStashedChangesTitle,
                message: text.restoreStashedChangesMessage,
                confirmLabel: text.restore,
                cancelLabel: text.later,
              });
              if (shouldRestore) {
                const popPayload = await client.stashPop(normalizedWorkspaceId, targetStash.index);
                if (popPayload.error) {
                  toast.error(popPayload.error.message);
                } else {
                  toast.show(text.stashedChangesRestored);
                }
                await invalidateStashAndCheckout();
              }
            }
          } catch {
            // Non-critical — user can still restore on next branch switch
          }
        } catch (err) {
          toast.error(err instanceof Error ? err.message : text.failedSwitchBranch);
        }
      })();
    },
    [
      client,
      currentBranchName,
      invalidateStashAndCheckout,
      normalizedWorkspaceId,
      stashAndSwitch,
      text.failedSwitchBranch,
      text.later,
      text.restore,
      text.restoreStashedChangesMessage,
      text.restoreStashedChangesTitle,
      text.stashedChangesRestored,
      toast,
    ],
  );

  return { branchOptions, isOpen, setIsOpen, handleBranchSelect, invalidateStashAndCheckout };
}

import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import type { AgentProviderDefinition } from "@server/server/agent/provider-manifest";
import type { AgentModelDefinition } from "@server/server/agent/agent-sdk-types";
import { getIsElectron } from "@/constants/platform";
import { invokeDesktopCommand } from "@/desktop/electron/invoke";
import { useAppSettings } from "@/hooks/use-settings";
import {
  useSub2APIAvailableGroups,
  useSub2APIClient,
  useSub2APIKeys,
} from "@/hooks/use-sub2api-api";
import type { ProviderStore } from "@/screens/settings/sub2api-provider-types";
import {
  buildGlobalCloudRouteGroups,
  resolveActiveGlobalCloudProviders,
} from "@/hooks/cloud-model-routing-utils";

export {
  buildCloudModelRoutingGroups,
  buildGlobalCloudRouteGroups,
} from "@/hooks/cloud-model-routing-utils";

const GLOBAL_PROVIDER_STORE_QUERY_KEY = ["desktop-provider-store", "cloud-route-status"] as const;

export function useCloudModelRouting(input: {
  serverId: string | null | undefined;
  cwd: string | null | undefined;
  providerDefinitions: AgentProviderDefinition[];
  allProviderModels?: Map<string, AgentModelDefinition[]>;
}) {
  const { settings } = useAppSettings();
  const cloudClient = useSub2APIClient();
  const cloudKeysQuery = useSub2APIKeys(1, 200);
  const cloudGroupsQuery = useSub2APIAvailableGroups();
  const isElectron = getIsElectron();
  const isCloudMode =
    settings.accessMode === "builtin" && cloudClient.isLoggedIn && Boolean(cloudClient.endpoint);

  const desktopProviderStoreQuery = useQuery({
    queryKey: GLOBAL_PROVIDER_STORE_QUERY_KEY,
    enabled: isElectron && isCloudMode,
    staleTime: 5_000,
    refetchInterval: isCloudMode ? 10_000 : false,
    queryFn: async (): Promise<ProviderStore> => {
      return await invokeDesktopCommand<ProviderStore>("get_providers");
    },
  });

  const cloudGroups = useMemo(
    () =>
      isCloudMode
        ? buildGlobalCloudRouteGroups({
            activeProviders: resolveActiveGlobalCloudProviders(desktopProviderStoreQuery.data),
            cloudEndpoint: cloudClient.endpoint,
            keys: cloudKeysQuery.data?.items,
            groups: cloudGroupsQuery.data,
            providerDefinitions: input.providerDefinitions,
            allProviderModels: input.allProviderModels,
          })
        : [],
    [
      cloudClient.endpoint,
      cloudGroupsQuery.data,
      cloudKeysQuery.data?.items,
      desktopProviderStoreQuery.data,
      input.providerDefinitions,
      input.allProviderModels,
      isCloudMode,
    ],
  );

  return {
    cloudGroups,
  };
}

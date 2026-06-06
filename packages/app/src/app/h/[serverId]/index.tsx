import { Redirect, useLocalSearchParams } from "expo-router";
import { useAppSettings } from "@/hooks/use-settings";
import { buildHostOpenProjectRoute, buildHostSimpleRoute } from "@/utils/host-routes";

export default function HostIndexRoute() {
  const params = useLocalSearchParams<{ serverId?: string }>();
  const { settings } = useAppSettings();
  const serverId = typeof params.serverId === "string" ? params.serverId : "";
  if (!serverId) return null;
  return (
    <Redirect
      href={
        settings.experienceMode === "simple"
          ? buildHostSimpleRoute(serverId)
          : buildHostOpenProjectRoute(serverId)
      }
    />
  );
}

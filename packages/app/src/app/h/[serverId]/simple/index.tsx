import { useLocalSearchParams } from "expo-router";
import { HostRouteBootstrapBoundary } from "@/components/host-route-bootstrap-boundary";
import { SimpleHomeScreen } from "@/screens/simple/simple-home-screen";

export default function SimpleHomeRoute() {
  const params = useLocalSearchParams<{ serverId?: string }>();
  const serverId = typeof params.serverId === "string" ? params.serverId : "";
  if (!serverId) return null;
  return (
    <HostRouteBootstrapBoundary>
      <SimpleHomeScreen serverId={serverId} />
    </HostRouteBootstrapBoundary>
  );
}

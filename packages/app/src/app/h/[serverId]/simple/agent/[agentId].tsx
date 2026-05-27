import { useLocalSearchParams } from "expo-router";
import { HostRouteBootstrapBoundary } from "@/components/host-route-bootstrap-boundary";
import { SimpleAgentScreen } from "@/screens/simple/simple-agent-screen";

export default function SimpleAgentRoute() {
  const params = useLocalSearchParams<{
    serverId?: string;
    agentId?: string;
  }>();
  const serverId = typeof params.serverId === "string" ? params.serverId : "";
  const agentId = typeof params.agentId === "string" ? params.agentId : "";
  if (!serverId || !agentId) return null;
  return (
    <HostRouteBootstrapBoundary>
      <SimpleAgentScreen serverId={serverId} agentId={agentId} />
    </HostRouteBootstrapBoundary>
  );
}

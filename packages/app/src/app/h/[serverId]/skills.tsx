import { useLocalSearchParams } from "expo-router";
import { HostRouteBootstrapBoundary } from "@/components/host-route-bootstrap-boundary";
import { SkillLibraryScreen } from "@/screens/skill-library-screen";

export default function HostSkillsRoute() {
  return (
    <HostRouteBootstrapBoundary>
      <HostSkillsRouteContent />
    </HostRouteBootstrapBoundary>
  );
}

function HostSkillsRouteContent() {
  const params = useLocalSearchParams<{ serverId?: string }>();
  const serverId = typeof params.serverId === "string" ? params.serverId : "";

  return <SkillLibraryScreen key={serverId} serverId={serverId} />;
}

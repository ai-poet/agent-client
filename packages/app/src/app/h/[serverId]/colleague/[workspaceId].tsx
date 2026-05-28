import { useLocalSearchParams } from "expo-router";
import { HostRouteBootstrapBoundary } from "@/components/host-route-bootstrap-boundary";
import { WorkspaceColleagueScreen } from "@/screens/workspace-colleague-screen";
import { decodeWorkspaceIdFromPathSegment } from "@/utils/host-routes";

function getParamValue(value: string | string[] | undefined): string {
  if (typeof value === "string") {
    return value.trim();
  }
  if (Array.isArray(value)) {
    const firstValue = value[0];
    return typeof firstValue === "string" ? firstValue.trim() : "";
  }
  return "";
}

export default function HostWorkspaceColleagueRoute() {
  return (
    <HostRouteBootstrapBoundary>
      <HostWorkspaceColleagueRouteContent />
    </HostRouteBootstrapBoundary>
  );
}

function HostWorkspaceColleagueRouteContent() {
  const params = useLocalSearchParams<{
    serverId?: string | string[];
    workspaceId?: string | string[];
  }>();
  const serverId = getParamValue(params.serverId);
  const workspaceSegment = getParamValue(params.workspaceId);
  const workspaceId = workspaceSegment
    ? (decodeWorkspaceIdFromPathSegment(workspaceSegment) ?? "")
    : "";

  return (
    <WorkspaceColleagueScreen
      key={`${serverId}:${workspaceId}`}
      serverId={serverId}
      workspaceId={workspaceId}
    />
  );
}

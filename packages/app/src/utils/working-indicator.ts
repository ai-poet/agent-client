export const WORKING_INDICATOR_CYCLE_MS = 1200;
export const WORKING_INDICATOR_OFFSETS = [
  0,
  160 / WORKING_INDICATOR_CYCLE_MS,
  320 / WORKING_INDICATOR_CYCLE_MS,
] as const;

function normalizePhase(value: number): number {
  "worklet";
  const wrapped = value % 1;
  return wrapped < 0 ? wrapped + 1 : wrapped;
}

export function getWorkingIndicatorDotStrength(progress: number, offset: number): number {
  "worklet";
  const phase = normalizePhase(progress + offset);
  if (phase <= 0.5) {
    return phase * 2;
  }
  return (1 - phase) * 2;
}

export function shouldShowAgentWorkingIndicator({
  agentStatus,
  pendingPermissionCount,
}: {
  agentStatus: string;
  pendingPermissionCount: number;
}): boolean {
  return agentStatus === "running" && pendingPermissionCount === 0;
}

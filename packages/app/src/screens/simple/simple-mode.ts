import type { AggregatedAgent } from "@/hooks/use-aggregated-agents";

export const SIMPLE_EXPERIENCE_LABEL = "simple";

export function isSimpleModeAgent(agent: Pick<AggregatedAgent, "labels">): boolean {
  return agent.labels?.experienceMode === SIMPLE_EXPERIENCE_LABEL;
}

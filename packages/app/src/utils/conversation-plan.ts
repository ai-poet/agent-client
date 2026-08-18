import type { StreamItem, TodoEntry } from "@/types/stream";

export interface ConversationPlan {
  /** Id of the todo_list item this plan came from. */
  id: string;
  items: TodoEntry[];
  completed: number;
  total: number;
  /** First unfinished step, or null when everything is done. */
  currentStep: string | null;
}

/**
 * Resolves the plan to pin above the composer.
 *
 * Only the newest plan counts, and only while it still has open work. A finished plan
 * hides itself rather than falling back to an older one — showing a stale plan is worse
 * than showing none.
 */
export function resolveActivePlan(items: StreamItem[]): ConversationPlan | null {
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const item = items[index];
    if (item?.kind !== "todo_list") {
      continue;
    }

    const entries = item.items;
    const total = entries.length;
    if (total === 0) {
      return null;
    }

    const completed = entries.filter((entry) => entry.completed).length;
    if (completed >= total) {
      // The newest plan is done — nothing to pin, and we must not regress to an older one.
      return null;
    }

    return {
      id: item.id,
      items: entries,
      completed,
      total,
      currentStep: entries.find((entry) => !entry.completed)?.text ?? null,
    };
  }

  return null;
}

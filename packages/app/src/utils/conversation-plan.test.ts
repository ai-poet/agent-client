import { describe, expect, it } from "vitest";

import type { StreamItem, TodoEntry } from "@/types/stream";
import { resolveActivePlan } from "./conversation-plan";

let sequence = 0;

function todoList(entries: TodoEntry[]): StreamItem {
  sequence += 1;
  return {
    kind: "todo_list",
    id: `todo-${sequence}`,
    timestamp: new Date(0),
    provider: "claude",
    items: entries,
  } as StreamItem;
}

function assistantMessage(): StreamItem {
  sequence += 1;
  return {
    kind: "assistant_message",
    id: `assistant-${sequence}`,
    text: "working",
    timestamp: new Date(0),
  } as StreamItem;
}

describe("resolveActivePlan", () => {
  it("pins the newest plan while work remains", () => {
    const plan = resolveActivePlan([
      todoList([
        { text: "First", completed: true },
        { text: "Second", completed: false },
        { text: "Third", completed: false },
      ]),
    ]);

    expect(plan).toMatchObject({ completed: 1, total: 3, currentStep: "Second" });
  });

  it("hides itself once every step is done", () => {
    expect(
      resolveActivePlan([
        todoList([
          { text: "First", completed: true },
          { text: "Second", completed: true },
        ]),
      ]),
    ).toBeNull();
  });

  it("never falls back to an older plan when the newest is complete", () => {
    const plan = resolveActivePlan([
      todoList([{ text: "Old open step", completed: false }]),
      assistantMessage(),
      todoList([{ text: "New done step", completed: true }]),
    ]);

    // Showing the stale open plan here would misrepresent the agent's state.
    expect(plan).toBeNull();
  });

  it("prefers the newest plan when both have open work", () => {
    const plan = resolveActivePlan([
      todoList([{ text: "Old step", completed: false }]),
      todoList([{ text: "New step", completed: false }]),
    ]);

    expect(plan?.currentStep).toBe("New step");
  });

  it("ignores an empty plan", () => {
    expect(resolveActivePlan([todoList([])])).toBeNull();
  });

  it("returns null when there is no plan at all", () => {
    expect(resolveActivePlan([assistantMessage()])).toBeNull();
    expect(resolveActivePlan([])).toBeNull();
  });
});

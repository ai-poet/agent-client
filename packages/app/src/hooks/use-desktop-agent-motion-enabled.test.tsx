/**
 * @vitest-environment jsdom
 */
import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useDesktopAgentMotionEnabled } from "./use-desktop-agent-motion-enabled";

const { platformState, accessibilityState } = vi.hoisted(() => ({
  platformState: {
    isElectron: false,
  },
  accessibilityState: {
    reduceMotionEnabled: false,
    listener: null as ((value: boolean) => void) | null,
    remove: vi.fn(),
  },
}));

vi.mock("@/constants/platform", () => ({
  getIsElectron: () => platformState.isElectron,
}));

vi.mock("react-native", () => ({
  AccessibilityInfo: {
    isReduceMotionEnabled: vi.fn(() => Promise.resolve(accessibilityState.reduceMotionEnabled)),
    addEventListener: vi.fn(
      (_eventName: "reduceMotionChanged", listener: (value: boolean) => void) => {
        accessibilityState.listener = listener;
        return {
          remove: accessibilityState.remove,
        };
      },
    ),
  },
}));

function renderProbe(onValue: (value: boolean) => void): Root {
  function Probe() {
    onValue(useDesktopAgentMotionEnabled());
    return null;
  }

  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);

  act(() => {
    root.render(<Probe />);
  });

  return root;
}

async function flushMotionState() {
  await act(async () => {
    await Promise.resolve();
  });
}

describe("useDesktopAgentMotionEnabled", () => {
  let root: Root | null = null;

  beforeEach(() => {
    platformState.isElectron = false;
    accessibilityState.reduceMotionEnabled = false;
    accessibilityState.listener = null;
    accessibilityState.remove.mockClear();
  });

  afterEach(() => {
    if (root) {
      act(() => {
        root?.unmount();
      });
      root = null;
    }
  });

  it("stays disabled outside Electron", async () => {
    const values: boolean[] = [];

    root = renderProbe((value) => values.push(value));
    await flushMotionState();

    expect(values.at(-1)).toBe(false);
    expect(accessibilityState.listener).toBeNull();
  });

  it("enables desktop motion when Electron allows motion", async () => {
    platformState.isElectron = true;
    accessibilityState.reduceMotionEnabled = false;
    const values: boolean[] = [];

    root = renderProbe((value) => values.push(value));
    await flushMotionState();

    expect(values).toEqual([false, true]);
  });

  it("keeps desktop motion disabled when reduced motion is enabled", async () => {
    platformState.isElectron = true;
    accessibilityState.reduceMotionEnabled = true;
    const values: boolean[] = [];

    root = renderProbe((value) => values.push(value));
    await flushMotionState();

    expect(values.at(-1)).toBe(false);
  });

  it("reacts to reduced-motion preference changes", async () => {
    platformState.isElectron = true;
    accessibilityState.reduceMotionEnabled = false;
    const values: boolean[] = [];

    root = renderProbe((value) => values.push(value));
    await flushMotionState();

    act(() => {
      accessibilityState.listener?.(true);
    });
    expect(values.at(-1)).toBe(false);

    act(() => {
      accessibilityState.listener?.(false);
    });
    expect(values.at(-1)).toBe(true);
  });
});

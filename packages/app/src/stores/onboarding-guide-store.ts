import { create } from "zustand";

export type OnboardingGuideSource = "auto" | "manual";

interface OnboardingGuideState {
  open: boolean;
  source: OnboardingGuideSource;
  stepIndex: number;
  autoPrompted: boolean;
  openGuide: (input?: { source?: OnboardingGuideSource }) => void;
  closeGuide: () => void;
  nextStep: (stepCount: number) => void;
  previousStep: () => void;
  setStepIndex: (stepIndex: number) => void;
}

export const useOnboardingGuideStore = create<OnboardingGuideState>((set) => ({
  open: false,
  source: "manual",
  stepIndex: 0,
  autoPrompted: false,
  openGuide: (input) =>
    set({
      open: true,
      source: input?.source ?? "manual",
      stepIndex: 0,
      ...(input?.source === "auto" ? { autoPrompted: true } : {}),
    }),
  closeGuide: () => set({ open: false }),
  nextStep: (stepCount) =>
    set((state) => ({
      stepIndex: Math.min(state.stepIndex + 1, Math.max(0, stepCount - 1)),
    })),
  previousStep: () =>
    set((state) => ({
      stepIndex: Math.max(0, state.stepIndex - 1),
    })),
  setStepIndex: (stepIndex) =>
    set({
      stepIndex: Math.max(0, stepIndex),
    }),
}));

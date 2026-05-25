import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  type PropsWithChildren,
} from "react";
import { Dimensions, View, type StyleProp, type ViewProps, type ViewStyle } from "react-native";
import { isWeb } from "@/constants/platform";

export type OnboardingGuideTargetId =
  | "sidebar.projects"
  | "sidebar.projectAdd"
  | "sidebar.newWorktree"
  | "sidebar.initializeGit"
  | "workspace.branchSwitcher"
  | "workspace.branchWorktree"
  | "agent.composer"
  | "changes.commit";

export interface OnboardingGuideTargetRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

type TargetRef = React.RefObject<View | null>;

interface OnboardingGuideTargetRegistry {
  register: (id: OnboardingGuideTargetId, ref: TargetRef) => () => void;
  measure: (id: OnboardingGuideTargetId) => Promise<OnboardingGuideTargetRect | null>;
  reveal: (id: OnboardingGuideTargetId) => Promise<boolean>;
}

const OnboardingGuideTargetContext = createContext<OnboardingGuideTargetRegistry | null>(null);

function measureView(ref: TargetRef): Promise<OnboardingGuideTargetRect | null> {
  return new Promise((resolve) => {
    const element = ref.current;
    if (!element?.measureInWindow) {
      resolve(null);
      return;
    }
    element.measureInWindow((x, y, width, height) => {
      if (width <= 0 || height <= 0) {
        resolve(null);
        return;
      }
      resolve({ x, y, width, height });
    });
  });
}

function rectIntersectsWindow(rect: OnboardingGuideTargetRect): boolean {
  const screen = Dimensions.get("window");
  return (
    rect.x < screen.width &&
    rect.y < screen.height &&
    rect.x + rect.width > 0 &&
    rect.y + rect.height > 0
  );
}

function revealView(ref: TargetRef): boolean {
  if (!isWeb) {
    return false;
  }
  const element = ref.current as unknown as {
    scrollIntoView?: (options?: ScrollIntoViewOptions) => void;
  } | null;
  if (typeof element?.scrollIntoView !== "function") {
    return false;
  }
  element.scrollIntoView({ block: "center", inline: "nearest", behavior: "smooth" });
  return true;
}

export function OnboardingGuideTargetProvider({ children }: PropsWithChildren) {
  const targetsRef = useRef(new Map<OnboardingGuideTargetId, Set<TargetRef>>());

  const register = useCallback((id: OnboardingGuideTargetId, ref: TargetRef) => {
    const refs = targetsRef.current.get(id) ?? new Set<TargetRef>();
    refs.add(ref);
    targetsRef.current.set(id, refs);
    return () => {
      refs.delete(ref);
      if (refs.size === 0) {
        targetsRef.current.delete(id);
      }
    };
  }, []);

  const measure = useCallback(async (id: OnboardingGuideTargetId) => {
    const refs = Array.from(targetsRef.current.get(id) ?? []);
    let fallbackRect: OnboardingGuideTargetRect | null = null;
    for (let index = refs.length - 1; index >= 0; index -= 1) {
      const rect = await measureView(refs[index]!);
      if (rect) {
        fallbackRect = fallbackRect ?? rect;
        if (rectIntersectsWindow(rect)) {
          return rect;
        }
      }
    }
    return fallbackRect;
  }, []);

  const reveal = useCallback(async (id: OnboardingGuideTargetId) => {
    const refs = Array.from(targetsRef.current.get(id) ?? []);
    for (let index = refs.length - 1; index >= 0; index -= 1) {
      if (revealView(refs[index]!)) {
        return true;
      }
    }
    return false;
  }, []);

  const value = useMemo(() => ({ register, measure, reveal }), [measure, register, reveal]);

  return (
    <OnboardingGuideTargetContext.Provider value={value}>
      {children}
    </OnboardingGuideTargetContext.Provider>
  );
}

export function useOnboardingGuideTargetRegistry(): OnboardingGuideTargetRegistry | null {
  return useContext(OnboardingGuideTargetContext);
}

export function OnboardingGuideTarget({
  id,
  children,
  pointerEvents,
  style,
}: PropsWithChildren<{
  id: OnboardingGuideTargetId;
  pointerEvents?: ViewProps["pointerEvents"];
  style?: StyleProp<ViewStyle>;
}>) {
  const registry = useOnboardingGuideTargetRegistry();
  const ref = useRef<View | null>(null);

  useEffect(() => {
    if (!registry) {
      return;
    }
    return registry.register(id, ref);
  }, [id, registry]);

  return (
    <View ref={ref} collapsable={false} pointerEvents={pointerEvents} style={style}>
      {children}
    </View>
  );
}

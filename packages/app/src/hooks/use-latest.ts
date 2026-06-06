import { useRef } from "react";

/**
 * Returns a ref that always holds the latest value.
 *
 * Unlike useRef + useEffect, this updates synchronously on render
 * without scheduling an extra effect. Use when you need a stable
 * callback that reads the latest prop/state inside event handlers
 * or async callbacks.
 */
export function useLatest<T>(value: T) {
  const ref = useRef(value);
  ref.current = value;
  return ref;
}

import { useLayoutEffect, useRef, useState, type RefObject } from "react";

/**
 * Live width of an element (`0` until measured). Layout choices that depend on the room a view really
 * has (a side panel next to a sidebar) must use this rather than a viewport media query.
 */
export function useElementWidth<T extends HTMLElement>(): [RefObject<T | null>, number] {
  const ref = useRef<T | null>(null);
  const [width, setWidth] = useState(0);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const update = () => setWidth(el.clientWidth);
    update();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(update);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);
  return [ref, width];
}

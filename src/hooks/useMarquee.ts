import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';

export interface MarqueeRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

interface DragState {
  startX: number;
  startY: number;
  additive: boolean;
}

/** Rubber-band selection over rendered rows marked with `data-file-path`. */
export function useMarquee(
  containerRef: RefObject<HTMLElement>,
  onSelectMany: (paths: string[], additive: boolean) => void,
  onClear: () => void,
): MarqueeRect | null {
  const [rect, setRect] = useState<MarqueeRect | null>(null);
  const drag = useRef<DragState | null>(null);
  const selectMany = useRef(onSelectMany);
  const clear = useRef(onClear);
  selectMany.current = onSelectMany;
  clear.current = onClear;

  const intersects = useCallback((container: HTMLElement, box: MarqueeRect): string[] => {
    const nodes = container.querySelectorAll<HTMLElement>('[data-file-path]');
    const paths: string[] = [];
    nodes.forEach((node) => {
      const bounds = node.getBoundingClientRect();
      const overlaps =
        bounds.right > box.left &&
        bounds.left < box.left + box.width &&
        bounds.bottom > box.top &&
        bounds.top < box.top + box.height;
      if (overlaps) {
        const path = node.dataset.filePath;
        if (path) paths.push(path);
      }
    });
    return paths;
  }, []);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const onPointerDown = (event: MouseEvent) => {
      if (event.button !== 0) return;
      const target = event.target as HTMLElement | null;
      if (target?.closest('button, input, a, select, [data-no-marquee]')) return;
      if (!container.contains(target)) return;
      drag.current = { startX: event.clientX, startY: event.clientY, additive: event.ctrlKey || event.metaKey };
      setRect({ left: event.clientX, top: event.clientY, width: 0, height: 0 });
    };

    const onPointerMove = (event: MouseEvent) => {
      const start = drag.current;
      if (!start) return;
      event.preventDefault();
      const box: MarqueeRect = {
        left: Math.min(start.startX, event.clientX),
        top: Math.min(start.startY, event.clientY),
        width: Math.abs(event.clientX - start.startX),
        height: Math.abs(event.clientY - start.startY),
      };
      setRect(box);
      if (box.width > 3 || box.height > 3) {
        selectMany.current(intersects(container, box), start.additive);
      }
    };

    const onPointerUp = () => {
      const start = drag.current;
      drag.current = null;
      setRect(null);
      if (start && !start.additive) {
        // A click on empty space clears the selection, like Explorer.
        clear.current();
      }
    };

    container.addEventListener('mousedown', onPointerDown);
    window.addEventListener('mousemove', onPointerMove);
    window.addEventListener('mouseup', onPointerUp);
    return () => {
      container.removeEventListener('mousedown', onPointerDown);
      window.removeEventListener('mousemove', onPointerMove);
      window.removeEventListener('mouseup', onPointerUp);
    };
  }, [containerRef, intersects]);

  return rect;
}

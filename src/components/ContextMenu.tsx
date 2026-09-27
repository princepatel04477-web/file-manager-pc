import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { LucideIcon } from 'lucide-react';

export interface ContextMenuItem {
  id: string;
  label: string;
  icon: LucideIcon;
  shortcut?: string;
  danger?: boolean;
  disabled?: boolean;
  separatorBefore?: boolean;
  onSelect: () => void;
}

interface ContextMenuProps {
  x: number;
  y: number;
  items: ContextMenuItem[];
  title?: string;
  onClose: () => void;
}

/** Keyboard- and pointer-driven context menu anchored to the click position. */
export function ContextMenu({ x, y, items, title, onClose }: ContextMenuProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [origin, setOrigin] = useState({ x, y });

  useLayoutEffect(() => {
    const node = ref.current;
    if (!node) return;
    const rect = node.getBoundingClientRect();
    const nextX = Math.min(x, Math.max(8, window.innerWidth - rect.width - 10));
    const nextY = Math.min(y, Math.max(8, window.innerHeight - rect.height - 10));
    if (nextX !== origin.x || nextY !== origin.y) setOrigin({ x: nextX, y: nextY });
  }, [origin.x, origin.y, x, y]);

  useEffect(() => {
    const dismiss = () => onClose();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener('pointerdown', dismiss);
    window.addEventListener('blur', dismiss);
    window.addEventListener('keydown', onKey);
    window.addEventListener('resize', dismiss);
    return () => {
      window.removeEventListener('pointerdown', dismiss);
      window.removeEventListener('blur', dismiss);
      window.removeEventListener('keydown', onKey);
      window.removeEventListener('resize', dismiss);
    };
  }, [onClose]);

  useEffect(() => {
    const node = ref.current;
    node?.querySelector<HTMLButtonElement>('button:not([disabled])')?.focus();
  }, []);

  return (
    <div
      ref={ref}
      className="context-menu"
      role="menu"
      aria-label={title ?? 'File actions'}
      style={{ left: origin.x, top: origin.y }}
      onPointerDown={(event) => event.stopPropagation()}
      onContextMenu={(event) => event.preventDefault()}
      onKeyDown={(event) => {
        if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
        event.preventDefault();
        const buttons = Array.from(ref.current?.querySelectorAll<HTMLButtonElement>('button:not([disabled])') ?? []);
        const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
        const next = event.key === 'ArrowDown' ? index + 1 : index - 1;
        const wrapped = next < 0 ? buttons.length - 1 : next >= buttons.length ? 0 : next;
        buttons[wrapped]?.focus();
      }}
    >
      {title && <div className="context-menu-title">{title}</div>}
      {items.map((item) => {
        const Icon = item.icon;
        return (
          <div key={item.id} className={item.separatorBefore ? 'context-menu-group' : undefined}>
            <button
              type="button"
              role="menuitem"
              className={`context-menu-item${item.danger ? ' danger' : ''}`}
              disabled={item.disabled}
              onClick={() => {
                item.onSelect();
                onClose();
              }}
            >
              <Icon size={14} strokeWidth={1.8} />
              <span>{item.label}</span>
              {item.shortcut && <kbd>{item.shortcut}</kbd>}
            </button>
          </div>
        );
      })}
    </div>
  );
}

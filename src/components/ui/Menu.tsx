// A small popup menu anchored to the button that opened it: the first item
// takes focus, arrow keys, Home and End move between items, Esc closes and
// returns focus to the button, Tab or a click outside closes it.
//
// It renders into document.body, so it is never clipped by a scrolling list
// and never sits inside a clickable row; its clicks and keys still stop at
// the menu, so nothing behind it (a row, the list's shortcuts) reacts.

import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode, type RefObject } from "react";
import { createPortal } from "react-dom";
import "./Menu.css";

/** What the arrow keys move between. Separators and captions are skipped. */
const ITEMS = "[role='menuitem'], [role='menuitemradio']";

interface MenuProps {
  anchor: RefObject<HTMLElement | null>;
  /** Accessible name of the menu. */
  label: string;
  /** A quiet caption above the items, e.g. what they belong to. Hidden from screen readers, which hear `label`. */
  caption?: string;
  /** Line the menu up with the anchor's start (left) or end (right) edge. */
  align?: "start" | "end";
  className?: string;
  /** `refocus`: put focus back on the anchor (after Esc), rather than leave it where the user moved it. */
  onClose(refocus: boolean): void;
  /** `MenuItem`s. */
  children: ReactNode;
}

export function Menu({ anchor, label, caption, align = "end", className, onClose, children }: MenuProps) {
  const menu = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  // Measure the menu as rendered (hidden), then place it below the anchor,
  // or above it when there is no room below, inside the window.
  useLayoutEffect(() => {
    const r = anchor.current?.getBoundingClientRect();
    const m = menu.current?.getBoundingClientRect();
    if (!r) return;
    const width = m?.width ?? 0;
    const height = m?.height ?? 0;
    const top = r.bottom + height + 8 > window.innerHeight ? Math.max(4, r.top - height - 4) : r.bottom + 4;
    const want = align === "end" ? r.right - width : r.left;
    setPos({ top, left: Math.max(4, Math.min(want, window.innerWidth - width - 4)) });
  }, [anchor, align]);

  useEffect(() => {
    menu.current?.querySelector<HTMLElement>(ITEMS)?.focus();
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (!menu.current?.contains(t) && !anchor.current?.contains(t)) onCloseRef.current(false);
    };
    const onBlur = () => onCloseRef.current(false);
    document.addEventListener("mousedown", onDown);
    window.addEventListener("blur", onBlur);
    window.addEventListener("resize", onBlur);
    return () => {
      document.removeEventListener("mousedown", onDown);
      window.removeEventListener("blur", onBlur);
      window.removeEventListener("resize", onBlur);
    };
  }, [anchor]);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    e.stopPropagation();
    const items = Array.from(menu.current?.querySelectorAll<HTMLElement>(ITEMS) ?? []);
    const i = items.indexOf(document.activeElement as HTMLElement);
    if (e.key === "Escape") {
      e.preventDefault();
      onClose(true);
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      items[(i + 1) % items.length]?.focus();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      items[(i - 1 + items.length) % items.length]?.focus();
    } else if (e.key === "Home") {
      e.preventDefault();
      items[0]?.focus();
    } else if (e.key === "End") {
      e.preventDefault();
      items[items.length - 1]?.focus();
    } else if (e.key === "Tab") {
      onClose(false);
    }
  };

  return createPortal(
    <div
      ref={menu}
      className={className ? `menu ${className}` : "menu"}
      role="menu"
      aria-label={label}
      style={pos ? { top: pos.top, left: pos.left } : { visibility: "hidden", top: 0, left: 0 }}
      onKeyDown={onKeyDown}
      onClick={(e) => e.stopPropagation()}
    >
      {caption && (
        <div className="menu-caption" aria-hidden="true">
          {caption}
        </div>
      )}
      {children}
    </div>,
    document.body,
  );
}

interface MenuItemProps {
  onSelect(): void;
  danger?: boolean;
  title?: string;
  /**
   * Makes the item one of a set of choices (`menuitemradio`): true for the
   * chosen one, which shows a check mark and is announced as checked.
   */
  checked?: boolean;
  className?: string;
  children: ReactNode;
}

export function MenuItem({ onSelect, danger = false, title, checked, className, children }: MenuItemProps) {
  const classes = ["menu-item", danger && "danger", checked !== undefined && "radio", className].filter(Boolean);
  return (
    <button
      type="button"
      role={checked === undefined ? "menuitem" : "menuitemradio"}
      aria-checked={checked}
      tabIndex={-1}
      className={classes.join(" ")}
      title={title}
      onClick={onSelect}
    >
      {checked !== undefined && (
        <span className="menu-check" aria-hidden="true">
          {checked ? "✓" : ""}
        </span>
      )}
      {children}
    </button>
  );
}

/** A thin line between groups of items. */
export function MenuSeparator() {
  return <div className="menu-sep" role="separator" />;
}

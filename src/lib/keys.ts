// Small helpers for global keyboard shortcuts.

export function isMac(): boolean {
  if (typeof navigator === "undefined") return false;
  const nav = navigator as Navigator & { userAgentData?: { platform?: string } };
  const platform = nav.userAgentData?.platform ?? navigator.platform ?? "";
  return /mac|iphone|ipad/i.test(platform);
}

/** True when the platform's command modifier is held: ⌘ on macOS, Ctrl elsewhere. */
export function hasMod(e: { metaKey: boolean; ctrlKey: boolean }): boolean {
  return isMac() ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
}

/** Display form of a shortcut: "⌘K" on macOS, "Ctrl K" elsewhere. */
export function shortcutLabel(key: string): string {
  return isMac() ? `⌘${key}` : `Ctrl ${key}`;
}

/** True when keyboard focus is in a text field, a menu or a dialog. */
export function focusIsBusy(target: EventTarget | null = document.activeElement): boolean {
  if (!(target instanceof Element)) return false;
  if (target.closest("[role='dialog'], [role='menu']")) return true;
  return isTextField(target);
}

/** True for elements that take typed text. */
export function isTextField(el: Element | null): boolean {
  if (!el) return false;
  if (el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) return true;
  if (el instanceof HTMLInputElement) {
    return !["button", "checkbox", "radio", "submit", "reset", "range", "color", "file"].includes(el.type);
  }
  return (el as HTMLElement).isContentEditable === true;
}

/** True while a modal dialog is open or focus is inside any dialog. */
export function isDialogActive(): boolean {
  if (typeof document === "undefined") return false;
  const active = document.activeElement;
  if (active?.closest('[role="dialog"], [role="alertdialog"], dialog')) return true;
  return document.querySelector('[aria-modal="true"], dialog[open]') != null;
}

/** True for buttons, links and other controls that handle Enter and Space themselves. */
export function isOwnActivationControl(el: Element | null): boolean {
  if (!el || !(el instanceof HTMLElement)) return false;
  if (el.dataset.prRow != null) return false;
  return el.matches('button, a[href], summary, input[type="checkbox"], input[type="radio"], [role="button"], [role="link"], [role="checkbox"], [role="switch"], [role="tab"], [role="menuitem"], [role="option"]');
}

/**
 * True when a click on `target` belongs to a control of its own inside
 * `container` (a button in a clickable row, an item in a menu it opened),
 * so the container should leave the click alone.
 */
export function isFromNestedControl(target: EventTarget | null, container: Element): boolean {
  if (!(target instanceof Element)) return false;
  const control = target.closest('button, a[href], input, select, textarea, [role="button"], [role="link"], [role="menuitem"], [role="menu"]');
  return control != null && control !== container;
}

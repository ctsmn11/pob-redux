export const isMac = /mac/i.test(navigator.userAgent);

/** Ctrl, or Cmd on macOS. */
export function modKey(e: KeyboardEvent | MouseEvent): boolean {
  return isMac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
}

/** The keycaps for a shortcut written "Mod+Shift+K". */
export function keycaps(shortcut: string): string[] {
  return shortcut.split("+").map((k) => {
    if (k === "Mod") return isMac ? "⌘" : "Ctrl";
    if (k === "Shift") return isMac ? "⇧" : "Shift";
    if (k === "Alt") return isMac ? "⌥" : "Alt";
    return k;
  });
}

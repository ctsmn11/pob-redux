import { modKey } from "$lib/keys";

export function wheelScroll(node: HTMLElement, _resetOn?: unknown) {
  const overflows = () => node.scrollHeight > node.clientHeight + 1;
  const sync = () => node.classList.toggle("overflowing", overflows());
  // Tooltips take no pointer events, so the wheel is caught at the window before the view under them.
  const onWheel = (e: WheelEvent) => {
    if (!modKey(e) || !overflows()) return;
    e.preventDefault();
    e.stopPropagation();
    node.scrollTop += e.deltaMode === 1 ? e.deltaY * 16 : e.deltaY;
  };
  const observer = new ResizeObserver(sync);
  observer.observe(node);
  sync();
  window.addEventListener("wheel", onWheel, { capture: true, passive: false });
  return {
    update() {
      node.scrollTop = 0;
      queueMicrotask(sync);
    },
    destroy() {
      observer.disconnect();
      window.removeEventListener("wheel", onWheel, { capture: true });
    },
  };
}

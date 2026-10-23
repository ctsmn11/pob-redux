<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { chat } from "$lib/state/chat.svelte";
  import { m } from "$lib/paraglide/messages";
  let count = $state(0);
  async function refresh() { count = await invoke<number>("memory_pending").catch(() => 0); }
  onMount(() => {
    void refresh();
    const off = listen("memory:changed", () => void refresh());
    return () => { void off.then((fn) => fn()); };
  });
</script>

{#if count}
  <button class="notice" onclick={() => chat.openSettings()}>{m.memory_pending_notice({ count })}</button>
{/if}

<style>
  .notice { padding: 10px; border-bottom: 1px solid var(--border); text-align: left; font-size: 12px; }
</style>

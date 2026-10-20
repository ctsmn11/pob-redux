<script lang="ts">
  import { engine, type ItemCustomization, type ItemCustomizationEdit, type ItemTarget } from "$lib/engine.svelte";
  import { m } from "$lib/paraglide/messages";

  let { target, raw, sources, implicit = false, onchange }: {
    target: ItemTarget;
    raw: string;
    sources: { id: string; label: string }[];
    implicit?: boolean;
    onchange: (edit: ItemCustomizationEdit) => Promise<ItemCustomization | false | undefined>;
  } = $props();

  let source = $state("Custom");
  let query = $state("");
  let customText = $state("");
  let options = $state<{ id: string; label: string; level: number | null }[]>([]);
  let total = $state(0);
  let option = $state("");
  let loading = $state(false);
  let error = $state<string | null>(null);
  const title = $derived(implicit ? m.items_add_implicit() : m.items_add_modifier());

  $effect(() => {
    if (source !== "Custom" && !sources.some((s) => s.id === source)) source = "Custom";
  });

  $effect(() => {
    raw;
    const currentTarget = target;
    const currentSource = source;
    const currentQuery = query;
    let active = true;
    options = [];
    option = "";
    error = null;
    loading = currentSource !== "Custom";
    const timer = setTimeout(async () => {
      if (currentSource === "Custom") return;
      try {
        const result = await engine.itemModifierOptions(currentTarget, currentSource, currentQuery, implicit);
        if (active) {
          options = result.options;
          total = result.total;
          option = result.options[0]?.id ?? "";
        }
      } catch (e) {
        if (active) error = String(e);
      } finally {
        if (active) loading = false;
      }
    }, 180);
    return () => { active = false; clearTimeout(timer); };
  });

  function add() {
    onchange(source === "Custom"
      ? { operation: "add_modifier", text: customText, implicit }
      : { operation: "add_modifier", source, modId: option, label: options.find((o) => o.id === option)?.label, implicit });
  }
</script>

<div class="label">{title}</div>
<select class="select" aria-label={implicit ? m.items_implicit_source() : m.items_modifier_source()} bind:value={source}>
  <option value="Custom">{m.items_custom_modifier()}</option>
  {#each sources as s (s.id)}
    <option value={s.id}>{s.label}</option>
  {/each}
</select>
{#if source === "Custom"}
  <input class="input" aria-label={implicit ? m.items_new_implicit() : m.items_new_modifier()} placeholder={m.items_modifier_example()} bind:value={customText} />
{:else}
  <input class="input" aria-label={m.items_search_modifiers()} placeholder={m.common_search()} bind:value={query} />
  <select class="select" aria-label={implicit ? m.items_new_implicit() : m.items_new_modifier()} bind:value={option} disabled={loading}>
    {#each options as opt (opt.id)}<option value={opt.id}>{opt.label}{opt.level ? ` · ${m.items_mod_level({ level: opt.level })}` : ""}</option>{/each}
  </select>
  <span class="dim">{loading ? m.items_mod_loading() : m.items_mod_count({ shown: options.length, total })}</span>
{/if}
{#if error}<div class="error" role="alert">{error}</div>{/if}
<button class="btn sm" disabled={source === "Custom" ? !customText.trim() : loading || !option} onclick={add}>{title}</button>

<style>
  .select { width: 100%; min-width: 0; font-size: var(--fs-xs); }
  .error { color: var(--bad); }
</style>

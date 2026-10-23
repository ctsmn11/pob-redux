<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import { build } from "$lib/state/build.svelte";
  import { game } from "$lib/state/game.svelte";
  import { m } from "$lib/paraglide/messages";

  interface Document { id: string; revision: number; game: string | null; path: string | null; text: string }
  interface Proposal { id: string; document_id: string; author: string; reason: string; before: string; text: string; status: string; destination: Document }
  interface Snapshot { enabled: boolean; global: Document; current: Document | null; documents: { document: Document; orphan: boolean }[]; proposals: Proposal[] }
  let data = $state<Snapshot | null>(null);
  let selected = $state("");
  let drafts = $state<Record<string, { revision: number; text: string }>>({});
  let error = $state("");
  let busy = $state(false);
  const doc = $derived(data?.documents.find((d) => d.document.id === selected)?.document);
  const pending = $derived(data?.proposals.filter((p) => p.status === "pending") ?? []);
  const label = (d: Document) => d.path ? `${d.game}: ${d.path}` : m.memory_global();

  async function refresh() {
    try {
      const next = await invoke<Snapshot>("memory_manage");
      for (const { document: d } of next.documents) {
        // Keep an unsaved draft and its revision so a concurrent edit produces a conflict.
        if (!drafts[d.id] || drafts[d.id].text === data?.documents.find((x) => x.document.id === d.id)?.document.text) {
          drafts[d.id] = { revision: d.revision, text: d.text };
        }
      }
      if (!next.documents.some((d) => d.document.id === selected)) selected = next.global.id;
      data = next;
    } catch (e) { error = String(e); }
  }
  async function run(command: string, args: Record<string, unknown>) {
    busy = true;
    error = "";
    try {
      await invoke(command, args);
      if (command === "memory_edit" || command === "memory_delete") delete drafts[String(args.document)];
      await refresh();
    } catch (e) { error = String(e); }
    finally { busy = false; }
  }
  async function remove(d: Document) {
    if (await confirm(m.memory_delete_confirm({ destination: label(d) }), { title: m.memory_title(), kind: "warning" })) {
      await run("memory_delete", { document: d.id, revision: drafts[d.id].revision });
    }
  }
  $effect(() => { void build.info?.file; void game.current; void refresh(); });
  onMount(() => {
    const off = listen("memory:changed", () => void refresh());
    return () => { void off.then((fn) => fn()); };
  });
</script>

<section class="memory">
  <label class="toggle">
    <input type="checkbox" checked={data?.enabled ?? false} disabled={busy || !data} onchange={(e) => run("memory_enable", { enabled: e.currentTarget.checked })} />
    <strong>{m.memory_title()}</strong>
  </label>
  <p>{m.memory_privacy()}</p>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if data}
    <p>{m.memory_current({ destination: data.current ? label(data.current) : m.memory_unsaved() })}</p>
    <label for="memory-document">{m.memory_document()}</label>
    <select id="memory-document" bind:value={selected} disabled={busy}>
      {#each data.documents as row (row.document.id)}
        <option value={row.document.id}>{label(row.document)}{row.orphan ? ` (${m.memory_orphan()})` : ""}</option>
      {/each}
    </select>
    {#if doc && drafts[doc.id]}
      <p class="destination">{label(doc)} · {m.memory_revision({ revision: doc.revision })}</p>
      <textarea aria-label={m.memory_document()} rows="7" bind:value={drafts[doc.id].text} disabled={busy}></textarea>
      <div class="actions">
        <span>{Array.from(drafts[doc.id].text).length} / 16000</span>
        <button class="btn sm" disabled={busy || Array.from(drafts[doc.id].text).length > 16000} onclick={() => run("memory_edit", { document: doc.id, revision: drafts[doc.id].revision, text: drafts[doc.id].text })}>{m.memory_save()}</button>
        <button class="btn sm ghost" disabled={busy} onclick={() => { drafts[doc.id] = { revision: doc.revision, text: doc.text }; }}>{m.memory_reload()}</button>
        <button class="btn sm ghost" disabled={busy} onclick={() => remove(doc)}>{m.memory_delete()}</button>
      </div>
    {/if}
    <h3>{m.memory_proposals({ count: pending.length })}</h3>
    {#each pending as p (p.id)}
      <article>
        <strong>{p.author}</strong>
        <p class="destination">{label(p.destination)} · {m.memory_revision({ revision: p.destination.revision })}</p>
        <p>{p.reason}</p>
        <div class="comparison">
          <div><b>{m.memory_before()}</b><pre class="before">{p.before || m.memory_empty()}</pre></div>
          <div><b>{m.memory_after()}</b><pre class="after">{p.text || m.memory_empty()}</pre></div>
        </div>
        <div class="actions">
          <button class="btn sm" disabled={busy || !data.enabled} onclick={() => run("memory_resolve", { proposal: p.id, approve: true })}>{m.memory_approve()}</button>
          <button class="btn sm ghost" disabled={busy} onclick={() => run("memory_resolve", { proposal: p.id, approve: false })}>{m.memory_reject()}</button>
        </div>
      </article>
    {/each}
    {#if data.proposals.some((p) => p.status !== "pending")}
      <details><summary>{m.memory_history()}</summary>
        {#each data.proposals.filter((p) => p.status !== "pending").reverse() as p (p.id)}
          <p class="destination">{p.author} · {label(p.destination)} · {p.status === "approved" ? m.memory_approved() : p.status === "rejected" ? m.memory_rejected() : m.memory_invalidated()}</p>
        {/each}
      </details>
    {/if}
  {/if}
</section>

<style>
  .memory { padding: 16px; border-bottom: 1px solid var(--border); }
  .toggle, .actions { display: flex; align-items: center; gap: 10px; }
  p { margin: 8px 0; font-size: 12px; opacity: .8; }
  select, textarea { display: block; width: 100%; margin: 8px 0; }
  textarea { resize: vertical; font-family: monospace; }
  .destination { overflow-wrap: anywhere; }
  .actions { flex-wrap: wrap; font-size: 12px; }
  article { border: 1px solid var(--border); padding: 12px; margin: 12px 0; }
  .comparison { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 12px; font-size: 12px; }
  pre { white-space: pre-wrap; overflow-wrap: anywhere; padding: 8px; max-height: 260px; overflow: auto; }
  .before { background: #ad333318; }
  .after { background: #33ad5518; }
  .error { color: #f88; opacity: 1; }
</style>

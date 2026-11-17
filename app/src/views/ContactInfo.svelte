<script lang="ts">
  import { historySearch, describeChatError, type MessageDto } from "../chat";
  import type { ContactDto } from "../contacts";
  import { contactsRename, contactsSetVerified, describeContactsError } from "../contacts";
  import { contacts, contactsNotice, refreshContacts } from "../lib/contactsStore";
  import { lanPeers, connections } from "../lib/netStore";
  import { resetConversation } from "../lib/chatStore";
  import { back, showTab } from "../lib/router";
  import { formatLocalTime } from "../lib/time";
  import { netConnectTicket, describeNetError } from "../net";
  import TrustBadge from "./TrustBadge.svelte";
  import NodeIdLine from "./NodeIdLine.svelte";
  import NicknameForm from "./NicknameForm.svelte";
  import VerifyContactDialog from "./VerifyContactDialog.svelte";
  import RemoveContactDialog from "./RemoveContactDialog.svelte";
  import ClearHistoryDialog from "./ClearHistoryDialog.svelte";

  let { nodeId }: { nodeId: string } = $props();

  const contact = $derived($contacts.find((c) => c.node_id === nodeId));
  const lanTicket = $derived($lanPeers.find((p) => p.node_id === nodeId)?.ticket);
  const conn = $derived($connections[nodeId]);
  const isConnected = $derived(conn?.state === "connected");

  let renaming = $state(false);
  let verifying = $state(false);
  let removalTarget = $state<ContactDto | null>(null);
  let busy = $state(false);
  let renameError = $state<string | null>(null);
  let error = $state<string | null>(null);
  let query = $state("");
  let results = $state<MessageDto[] | null>(null);
  let searchError = $state<string | null>(null);
  let searchSeq = 0;
  let clearing = $state(false);
  let cleared = $state(false);

  // The contacts store has no loaded flag: a missing contact counts as removed once it was
  // seen present here, or once the store holds other contacts (so it has loaded).
  let seenContact = false;
  $effect(() => {
    if (contact) seenContact = true;
    else if (!removalTarget && (seenContact || $contacts.length > 0)) showTab("contacts");
  });

  async function connect() {
    error = null;
    busy = true;
    try {
      await netConnectTicket(lanTicket!);
    } catch (e) {
      error = describeNetError(e);
    } finally {
      busy = false;
    }
  }

  async function rename(nickname: string) {
    renameError = null;
    busy = true;
    try {
      await contactsRename(nodeId, nickname);
      await refreshContacts();
      renaming = false;
    } catch (e) {
      renameError = describeContactsError(e);
    } finally {
      busy = false;
    }
  }

  async function unverify() {
    error = null;
    try {
      await contactsSetVerified(nodeId, false);
      await refreshContacts();
    } catch (e) {
      error = describeContactsError(e);
    }
  }

  function historyCleared() {
    resetConversation(nodeId);
    query = "";
    results = null;
    searchSeq++;
    clearing = false;
    cleared = true;
  }

  async function search() {
    const q = query;
    const seq = ++searchSeq;
    searchError = null;
    if (q.length < 1) {
      results = null;
      return;
    }
    try {
      const found = await historySearch(q, nodeId);
      if (seq === searchSeq) results = found;
    } catch (e) {
      if (seq === searchSeq) {
        results = null;
        searchError = describeChatError(e);
      }
    }
  }

  function searchKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      query = "";
      void search();
    }
  }
</script>

{#if contact || removalTarget}
  <section aria-label="Contact info" class="stack contact-info">
    <header class="row contact-info-head">
      <button type="button" class="btn btn-secondary btn-sm" data-screen-focus onclick={back}>Back</button>
      <h2 class="contact-info-name truncate"><bdi>{contact?.nickname ?? nodeId.slice(0, 8)}</bdi></h2>
      {#if contact}<TrustBadge verified={contact.verified} />{/if}
      <button
        type="button"
        class="btn btn-ghost btn-sm btn-icon"
        aria-label="Rename"
        onclick={() => {
          renameError = null;
          renaming = !renaming;
        }}
      >
        <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9" /><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z" /></svg>
      </button>
      <button
        type="button"
        class="btn btn-danger btn-sm btn-icon"
        aria-label="Remove…"
        onclick={() => (removalTarget = contact ?? null)}
      >
        <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18" /><path d="M8 6V4h8v2" /><path d="M19 6l-1 14H6L5 6" /><path d="M10 11v6" /><path d="M14 11v6" /></svg>
      </button>
    </header>

    {#if renaming && contact}
      <NicknameForm
        initial={contact.nickname}
        selfName={contact.self_name}
        submitLabel="Save nickname"
        {busy}
        error={renameError}
        onSubmit={rename}
        onCancel={() => (renaming = false)}
      />
    {/if}

    <NodeIdLine {nodeId} />
    {#if contact}
      <p data-testid="self-name-context">
        {#if contact.self_name}They call themselves “<bdi data-testid="peer-name">{contact.self_name}</bdi>” (chosen by them, not verified){:else}They did not send a name{/if}
      </p>
      <div class="row">
        {#if contact.verified === true}
          <button type="button" class="btn btn-secondary btn-sm" onclick={unverify}>Mark unverified</button>
        {:else}
          <button type="button" class="btn btn-secondary btn-sm" onclick={() => (verifying = true)}>Mark verified…</button>
        {/if}
      </div>
    {/if}
    {#if verifying && contact}
      <VerifyContactDialog {contact} onDone={() => (verifying = false)} onCancel={() => (verifying = false)} />
    {/if}

    <div class="row">
      <p class="muted" data-testid="contact-connection">{#if conn}<bdi>{conn.direction} | {conn.state}{conn.reason ? ` | ${conn.reason}` : ""}</bdi>{:else}Not connected{/if}</p>
      {#if lanTicket && !isConnected}
        <button type="button" class="btn btn-secondary btn-sm" disabled={busy} onclick={connect}>Connect</button>
      {/if}
    </div>
    {#if error}<p role="alert" class="alert alert-danger">{error}</p>{/if}

    <div class="row">
      <button type="button" class="btn btn-danger btn-sm" disabled={clearing} onclick={() => ((clearing = true), (cleared = false))}>
        Clear history…
      </button>
    </div>
    {#if clearing}
      <ClearHistoryDialog
        {nodeId}
        nickname={contact?.nickname ?? nodeId.slice(0, 8)}
        onDone={historyCleared}
        onCancel={() => (clearing = false)}
      />
    {/if}
    {#if cleared}<p role="status" class="alert alert-info">History cleared.</p>{/if}

    <input
      class="input"
      type="search"
      aria-label="Search this conversation"
      placeholder="Search this conversation"
      bind:value={query}
      oninput={search}
      onkeydown={searchKey}
    />
    {#if searchError}<p role="alert" class="alert alert-danger">{searchError}</p>{/if}
    {#if results !== null}
      <ul class="list" aria-label="Search results" data-testid="search-results">
        {#each results as r (r.id)}
          <li>
            <span>{r.direction === "out" ? "You" : "Them"}</span>
            <time>{formatLocalTime(r.sent_at_ms)}</time>
            <bdi dir="auto" class="result-body wrap-anywhere">{r.body}</bdi>
          </li>
        {:else}
          <li>No matches.</li>
        {/each}
      </ul>
    {/if}

    {#if removalTarget}
      <RemoveContactDialog
        contact={removalTarget}
        onCancel={() => (removalTarget = null)}
        onDone={(r) => {
          const name = removalTarget!.nickname;
          contactsNotice.set(`Removed “${name}”. Your message history with them ${r.history_erased ? "was deleted" : "was kept"}.`);
          showTab("contacts");
        }}
      />
    {/if}
  </section>
{/if}

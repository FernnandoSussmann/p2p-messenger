<script lang="ts">
  import { tick } from "svelte";
  import { historySearch, describeChatError, type MessageDto } from "../chat";
  import { contacts } from "../lib/contactsStore";
  import { connections } from "../lib/netStore";
  import { conversation, loadOlder, sendMessage } from "../lib/chatStore";
  import { formatLocalTime } from "../lib/time";
  import TrustBadge from "./TrustBadge.svelte";
  import MessageBubble from "./MessageBubble.svelte";
  import Composer from "./Composer.svelte";

  interface Props {
    nodeId: string;
    onBack: () => void;
  }
  let { nodeId, onBack }: Props = $props();

  const contact = $derived($contacts.find((c) => c.node_id === nodeId));
  const connected = $derived($connections[nodeId]?.state === "connected");
  const conv = $derived($conversation?.nodeId === nodeId ? $conversation : null);
  const messages = $derived(conv?.messages ?? []);
  const newestId = $derived(messages.length ? messages[messages.length - 1].id : null);

  let log = $state<HTMLOListElement>();
  let nearBottom = true;
  let query = $state("");
  let results = $state<MessageDto[] | null>(null);
  let searchError = $state<string | null>(null);
  let searchSeq = 0;
  let lastNewest: string | null = null;

  function onScroll() {
    if (!log) return;
    nearBottom = log.scrollHeight - log.scrollTop - log.clientHeight < 80;
  }

  $effect(() => {
    const id = newestId;
    if (id !== lastNewest) {
      const first = lastNewest === null;
      lastNewest = id;
      if (first || nearBottom) {
        void tick().then(() => {
          if (log) log.scrollTop = log.scrollHeight;
        });
      }
    }
  });

  async function older() {
    const before = log?.scrollHeight ?? 0;
    await loadOlder();
    await tick();
    if (log) log.scrollTop += log.scrollHeight - before;
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

<section aria-label="Conversation">
  <header>
    <button type="button" onclick={onBack}>Back</button>
    <strong><bdi>{contact?.nickname ?? nodeId.slice(0, 8)}</bdi></strong>
    {#if contact}<TrustBadge verified={contact.verified} />{/if}
    <span data-testid="connection-text">{connected ? "Connected" : "Not connected"}</span>
  </header>
  {#if !contact}
    <p role="status">This contact was removed.</p>
  {:else if !connected}
    <p role="status">
      Not connected. Messages wait as Pending and are sent automatically once you are connected
      again: when they connect to you, or when you use Contacts → Connect or paste their ticket in
      Network.
    </p>
  {/if}
  <input
    type="search"
    aria-label="Search this conversation"
    placeholder="Search this conversation"
    bind:value={query}
    oninput={search}
    onkeydown={searchKey}
  />
  {#if searchError}<p role="alert">{searchError}</p>{/if}
  {#if results !== null}
    <ul aria-label="Search results" data-testid="search-results">
      {#each results as r (r.id)}
        <li>
          <span>{r.direction === "out" ? "You" : "Them"}</span>
          <time>{formatLocalTime(r.sent_at_ms)}</time>
          <bdi dir="auto" class="result-body">{r.body}</bdi>
        </li>
      {:else}
        <li>No matches.</li>
      {/each}
    </ul>
  {/if}
  {#if conv?.error}<p role="alert">{conv.error}</p>{/if}
  {#if conv && conv.nextBefore !== null}
    <button type="button" disabled={conv.loading} onclick={older}>Load older messages</button>
  {/if}
  <ol role="log" aria-live="polite" bind:this={log} onscroll={onScroll}>
    {#each messages as m (m.id)}
      <MessageBubble message={m} />
    {/each}
  </ol>
  <Composer disabled={!contact} onSend={sendMessage} />
</section>

<style>
  header {
    display: flex;
    gap: 0.5em;
    align-items: center;
  }
  ol {
    padding: 0;
    max-height: 50vh;
    overflow-y: auto;
  }
  .result-body {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    unicode-bidi: isolate;
  }
</style>

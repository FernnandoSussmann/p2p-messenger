<script lang="ts">
  import { tick } from "svelte";
  import { contacts } from "../lib/contactsStore";
  import { connections } from "../lib/netStore";
  import { conversation, loadOlder, sendMessage } from "../lib/chatStore";
  import { showContactInfo, showTab } from "../lib/router";
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
  let lastNewest: string | null = null;
  let seenContact = false;

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

  // The contacts store has no loaded flag: a missing contact counts as removed once it was
  // seen present here, or once the store holds other contacts (so it has loaded).
  $effect(() => {
    if (contact) seenContact = true;
    else if (seenContact || $contacts.length > 0) showTab("contacts");
  });
</script>

{#if contact}
  <section class="conversation" aria-label="Conversation">
    <header class="conv-header">
      <button type="button" class="btn btn-secondary btn-sm" data-screen-focus onclick={onBack}>Back</button>
      <strong class="conv-name"><bdi>{contact.nickname}</bdi></strong>
      <TrustBadge verified={contact.verified} />
      <span class="muted" data-testid="connection-text">{connected ? "Connected" : "Not connected"}</span>
      <button type="button" class="btn btn-ghost btn-icon conv-info" aria-label="Contact info" onclick={() => showContactInfo(nodeId)}>
        <svg aria-hidden="true" viewBox="0 0 24 24" width="18" height="18" fill="currentColor"><circle cx="12" cy="5" r="2" /><circle cx="12" cy="12" r="2" /><circle cx="12" cy="19" r="2" /></svg>
      </button>
    </header>
    {#if !connected}
      <p role="status" class="alert alert-info">
        Not connected. Messages wait as Pending and are sent automatically once you are connected
        again: when they connect to you, or when you use Connect in their contact info or paste
        their ticket in Network.
      </p>
    {/if}
    {#if conv?.error}<p role="alert" class="alert alert-danger">{conv.error}</p>{/if}
    {#if conv && conv.nextBefore !== null}
      <button type="button" class="btn btn-secondary btn-sm" disabled={conv.loading} onclick={older}>Load older messages</button>
    {/if}
    <ol class="chat-log" role="log" aria-live="polite" bind:this={log} onscroll={onScroll}>
      {#each messages as m (m.id)}
        <MessageBubble message={m} />
      {/each}
    </ol>
    <Composer onSend={sendMessage} />
  </section>
{/if}

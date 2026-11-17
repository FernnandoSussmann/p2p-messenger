<script lang="ts">
  import type { ContactDto } from "../contacts";
  import { connections } from "../lib/netStore";
  import { openChat, unread } from "../lib/chatStore";
  import TrustBadge from "./TrustBadge.svelte";

  let { contact }: { contact: ContactDto } = $props();

  const conn = $derived($connections[contact.node_id]);
  const unreadCount = $derived($unread[contact.node_id] ?? 0);
</script>

<!-- Only the local nickname is shown here (user decision). contact.self_name is
     intentionally not rendered; it is shown on the contact-info screen only. -->
<li class="contact-item">
  <button type="button" class="contact-row" data-testid="contact-row" onclick={() => openChat(contact.node_id)}>
    <TrustBadge verified={contact.verified} />
    <span class="contact-name" data-testid="contact-nickname"><bdi>{contact.nickname}</bdi></span>
    <span class="muted contact-conn" data-testid="contact-connection"
      >{#if conn}<bdi>{conn.direction} | {conn.state}{conn.reason ? ` | ${conn.reason}` : ""}</bdi>{:else}Not connected{/if}</span
    >
    {#if unreadCount > 0}<span class="chip" data-testid="contact-unread">{unreadCount} unread</span>{/if}
  </button>
</li>

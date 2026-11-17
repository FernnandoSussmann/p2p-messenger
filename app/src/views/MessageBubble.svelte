<script lang="ts">
  import type { MessageDto } from "../chat";
  import { describeChatError } from "../chat";
  import { resendMessage } from "../lib/chatStore";
  import { formatLocalTime, guardedIso } from "../lib/time";

  let { message }: { message: MessageDto } = $props();

  let error = $state<string | null>(null);
  let busy = $state(false);

  const labels: Record<string, string> = {
    sending: "Sending…",
    delivered: "Delivered",
    pending: "Pending: will be sent when connected",
    failed: "Not delivered",
  };

  async function resend() {
    error = null;
    busy = true;
    try {
      await resendMessage(message.id);
    } catch (e) {
      error = describeChatError(e);
    } finally {
      busy = false;
    }
  }
</script>

<li
  class={["bubble", message.direction === "out" ? "own out" : "peer in", message.status === "failed" && message.direction === "out" && "failed"]}
  data-testid="message"
>
  {#if message.direction === "out"}<span class="who">You</span>{/if}
  <bdi dir="auto" class="body wrap-anywhere">{message.body}</bdi>
  <div class="stamp">
    <time datetime={guardedIso(message.sent_at_ms)} title={message.direction === "in" ? "Sent (their clock)" : undefined}>{formatLocalTime(message.sent_at_ms)}</time>
    {#if message.direction === "in"}
      {#if message.received_at_ms !== null && guardedIso(message.received_at_ms) !== undefined}
        <span>|</span>
        <time datetime={guardedIso(message.received_at_ms)} title="Received (this device)">{formatLocalTime(message.received_at_ms)}</time>
      {/if}
    {:else}
      <span class="status" data-status={message.status}>{labels[message.status] ?? ""}</span>
      {#if message.status === "failed"}
        <button type="button" class="btn btn-ghost btn-sm" disabled={busy} onclick={resend}>Resend</button>
      {/if}
    {/if}
  </div>
  {#if error}<p role="alert" class="alert alert-danger">{error}</p>{/if}
</li>

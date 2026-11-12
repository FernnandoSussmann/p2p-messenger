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

<li class={message.direction === "out" ? "own" : "peer"} data-testid="message">
  <bdi dir="auto" class="body">{message.body}</bdi>
  <div class="meta">
    <time datetime={guardedIso(message.sent_at_ms)}>{formatLocalTime(message.sent_at_ms)}</time>
    {#if message.direction === "out"}
      <span class="status" data-status={message.status}>{labels[message.status] ?? ""}</span>
      {#if message.status === "failed"}
        <button type="button" disabled={busy} onclick={resend}>Resend</button>
      {/if}
    {/if}
  </div>
  {#if error}<p role="alert">{error}</p>{/if}
</li>

<style>
  li {
    list-style: none;
    max-width: 75%;
    margin: 0.4em 0;
    padding: 0.4em 0.7em;
    border-radius: 0.6em;
  }
  .own {
    margin-left: auto;
    text-align: right;
    background: #e3f0ff;
  }
  .peer {
    margin-right: auto;
    text-align: left;
    background: #eee;
  }
  .body {
    display: block;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    unicode-bidi: isolate;
  }
  .meta {
    font-size: 0.75em;
    color: #555;
  }
</style>

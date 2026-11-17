<script lang="ts">
  import { MAX_BODY_BYTES, describeChatError, utf8Length } from "../chat";

  interface Props {
    disabled?: boolean;
    onSend: (body: string) => Promise<void>;
  }
  let { disabled = false, onSend }: Props = $props();

  let body = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);

  const bytes = $derived(utf8Length(body));
  const canSend = $derived(!disabled && !busy && body.trim() !== "" && bytes <= MAX_BODY_BYTES);

  async function send() {
    if (!canSend) return;
    error = null;
    busy = true;
    try {
      await onSend(body);
      body = "";
    } catch (e) {
      error = describeChatError(e);
    } finally {
      busy = false;
    }
  }

  function keydown(e: KeyboardEvent) {
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      void send();
    }
  }
</script>

<div class="composer">
  {#if bytes > MAX_BODY_BYTES * 0.9}
    <p data-testid="byte-counter" class="hint" class:over={bytes > MAX_BODY_BYTES}>{bytes} / {MAX_BODY_BYTES} bytes</p>
  {/if}
  <textarea class="textarea" aria-label="Message" rows="3" bind:value={body} {disabled} onkeydown={keydown}></textarea>
  <button type="button" class="btn btn-primary" disabled={!canSend} onclick={send}>Send</button>
  {#if error}<p role="alert" class="alert alert-danger">{error}</p>{/if}
</div>

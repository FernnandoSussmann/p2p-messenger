<script lang="ts">
  import { tick } from "svelte";
  import { MAX_BODY_BYTES, describeChatError, utf8Length } from "../chat";

  interface Props {
    disabled?: boolean;
    onSend: (body: string) => Promise<void>;
  }
  let { disabled = false, onSend }: Props = $props();

  let body = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let box = $state<HTMLTextAreaElement>();
  const MAX_PX = 160;

  function grow() {
    if (!box) return;
    box.style.height = "auto";
    box.style.height = `${Math.min(box.scrollHeight, MAX_PX)}px`;
  }

  const bytes = $derived(utf8Length(body));
  const canSend = $derived(!disabled && !busy && body.trim() !== "" && bytes <= MAX_BODY_BYTES);

  async function send() {
    if (!canSend) return;
    error = null;
    busy = true;
    try {
      await onSend(body);
      body = "";
      await tick();
      grow();
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
  <div class="composer-row">
    <textarea
      class="textarea"
      rows="1"
      aria-label="Message"
      placeholder="Type anything…"
      bind:this={box}
      bind:value={body}
      {disabled}
      oninput={grow}
      onkeydown={keydown}
    ></textarea>
    <button type="button" class="btn btn-primary btn-icon" aria-label="Send" disabled={!canSend} onclick={send}>
      <svg aria-hidden="true" viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14M13 6l6 6-6 6" /></svg>
    </button>
  </div>
  {#if error}<p role="alert" class="alert alert-danger">{error}</p>{/if}
</div>

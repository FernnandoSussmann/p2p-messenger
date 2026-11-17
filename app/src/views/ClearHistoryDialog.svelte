<script lang="ts">
  import { historyClear, describeChatError } from "../chat";
  import { HISTORY_DELETE_GUARANTEE } from "../lib/historyCopy";

  interface Props {
    nodeId: string;
    nickname: string;
    onDone: () => void;
    onCancel: () => void;
  }
  let { nodeId, nickname, onDone, onCancel }: Props = $props();

  let busy = $state(false);
  let error = $state<string | null>(null);

  async function confirm() {
    busy = true;
    error = null;
    try {
      await historyClear(nodeId);
      onDone();
    } catch (e) {
      error = describeChatError(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="dialog" role="dialog" aria-label="Clear history">
  <p><strong>Clear message history with “<bdi>{nickname}</bdi>”?</strong></p>
  <p>This permanently deletes every message in this conversation, including search results.</p>
  <p>{HISTORY_DELETE_GUARANTEE}</p>
  <p>This cannot be undone.</p>
  {#if busy}<p role="status" class="alert alert-info">Clearing… This may take a moment.</p>{/if}
  {#if error}<p role="alert" class="alert alert-danger">{error}</p>{/if}
  <!-- svelte-ignore a11y_autofocus -->
  <div class="dialog-foot">
    <button type="button" class="btn btn-secondary" autofocus disabled={busy} onclick={onCancel}>Cancel</button>
    <button type="button" class="btn btn-danger" disabled={busy} onclick={confirm}>Clear history</button>
  </div>
</div>

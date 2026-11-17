<script lang="ts">
  import { untrack } from "svelte";

  interface Props {
    initial?: string;
    selfName: string | null;
    submitLabel: string;
    busy?: boolean;
    error?: string | null;
    onSubmit: (nickname: string) => void;
    onCancel?: () => void;
    cancelLabel?: string;
  }

  let {
    initial,
    selfName,
    submitLabel,
    busy = false,
    error = null,
    onSubmit,
    onCancel,
    cancelLabel = "Cancel",
  }: Props = $props();

  let value = $state(untrack(() => initial) ?? "");
  const trimmed = $derived(value.trim());
</script>

<form
  class="stack"
  onsubmit={(e) => {
    e.preventDefault();
    if (trimmed && !busy) onSubmit(trimmed);
  }}
>
  <p data-testid="self-name-context">
    {#if selfName}They call themselves “<bdi data-testid="peer-name">{selfName}</bdi>” (chosen by them, not verified){:else}They did not send a name{/if}
  </p>
  <label class="field">
    Your name for them
    <input class="input" type="text" maxlength="256" bind:value />
  </label>
  {#if error}<p role="alert" class="alert alert-danger">{error}</p>{/if}
  <div class="row">
    <button type="submit" class="btn btn-primary" disabled={!trimmed || busy}>{submitLabel}</button>
    {#if onCancel}<button type="button" class="btn btn-secondary" onclick={onCancel}>{cancelLabel}</button>{/if}
  </div>
</form>

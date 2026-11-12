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
  onsubmit={(e) => {
    e.preventDefault();
    if (trimmed && !busy) onSubmit(trimmed);
  }}
>
  <p data-testid="self-name-context">
    {#if selfName}They call themselves “<bdi data-testid="peer-name">{selfName}</bdi>” (chosen by them, not verified){:else}They did not send a name{/if}
  </p>
  <label>
    Your name for them
    <input type="text" maxlength="256" bind:value />
  </label>
  {#if error}<p role="alert">{error}</p>{/if}
  <button type="submit" disabled={!trimmed || busy}>{submitLabel}</button>
  {#if onCancel}<button type="button" onclick={onCancel}>{cancelLabel}</button>{/if}
</form>

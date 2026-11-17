<script lang="ts">
  import { THEMES, type Theme } from "../lib/theme";

  let {
    current,
    onChoose,
    disabled = false,
  }: { current: Theme; onChoose: (t: Theme) => void; disabled?: boolean } = $props();

  const LABELS: Record<Theme, string> = { midnight: "Midnight", dark: "Dark", light: "Light" };
  let buttons: HTMLButtonElement[] = [];

  function onKey(e: KeyboardEvent, i: number) {
    const step =
      e.key === "ArrowRight" || e.key === "ArrowDown"
        ? 1
        : e.key === "ArrowLeft" || e.key === "ArrowUp"
          ? -1
          : 0;
    if (!step) return;
    e.preventDefault();
    const n = (i + step + THEMES.length) % THEMES.length;
    buttons[n]?.focus();
    onChoose(THEMES[n]);
  }
</script>

<div class="theme-picker" role="radiogroup" aria-label="Theme">
  {#each THEMES as t, i (t)}
    <span class="swatch-wrap">
      <button
        type="button"
        role="radio"
        class="swatch"
        data-theme={t}
        aria-checked={t === current}
        aria-label={LABELS[t]}
        tabindex={t === current ? 0 : -1}
        {disabled}
        bind:this={buttons[i]}
        onclick={() => onChoose(t)}
        onkeydown={(e) => onKey(e, i)}
      ></button>
    </span>
  {/each}
</div>

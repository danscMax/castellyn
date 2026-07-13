<script lang="ts" generics="T extends string | number">
  // A1: one segmented single-choice control, replacing the hand-rolled variants that each tab grew
  // (sw-btn+is-active in Analytics, sw-btn+primary/ghost in Settings, a bordered box in Environments).
  // Reuses the button canon (sw-btn / sw-btn-primary / sw-btn-ghost) — no colours of its own, so the
  // contrast guard has nothing to scan and every adopter looks the same in both themes.
  type Opt = { value: T; label: string; title?: string };
  let {
    value,
    options,
    onChange,
    disabled = false,
    compact = false,
    ariaLabel
  }: {
    value: T;
    options: Opt[];
    onChange: (v: T) => void;
    disabled?: boolean;
    /** Smaller type for dense tab bars (Analytics/Environments); default matches full sw-btn size. */
    compact?: boolean;
    ariaLabel?: string;
  } = $props();
</script>

<div class="flex flex-wrap gap-sw-2" role="group" aria-label={ariaLabel}>
  {#each options as o (o.value)}
    <button
      type="button"
      class="sw-btn {compact ? 'text-sw-xs' : ''} {value === o.value ? 'sw-btn-primary' : 'sw-btn-ghost'}"
      aria-pressed={value === o.value}
      title={o.title}
      {disabled}
      onclick={() => onChange(o.value)}>{o.label}</button>
  {/each}
</div>

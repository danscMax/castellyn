<script lang="ts">
  // Pill toggle switch (Sweet Whisper style). Two-way bindable `checked`, plus an optional
  // `onCheckedChange` callback for side effects (so callers don't need a raw checkbox).
  //
  // The control renders no text, so its accessible name has to be supplied. A wrapping <label> does
  // NOT name it — that association only works for form controls, not for a <button role="switch">.
  // `ariaLabel` is the name; `title` doubles as one when it already says what the switch does.
  let {
    checked = $bindable(false),
    disabled = false,
    title = '',
    ariaLabel = '',
    onCheckedChange
  }: {
    checked?: boolean;
    disabled?: boolean;
    title?: string;
    ariaLabel?: string;
    onCheckedChange?: (checked: boolean) => void;
  } = $props();

  function toggle() {
    if (disabled) return;
    checked = !checked;
    onCheckedChange?.(checked);
  }
</script>

<button
  type="button"
  class="tgl"
  class:on={checked}
  {disabled}
  title={title || undefined}
  aria-label={ariaLabel || title || undefined}
  role="switch"
  aria-checked={checked}
  onclick={toggle}
>
  <span class="knob"></span>
</button>

<style>
  .tgl {
    position: relative;
    /* 24px tall so the switch meets the WCAG 2.2 minimum pointer-target size (2.5.8). */
    width: 40px;
    height: 24px;
    flex-shrink: 0;
    border-radius: 999px;
    border: 1px solid var(--sw-border);
    background: var(--sw-bg-hover);
    cursor: pointer;
    padding: 0;
    transition: background 0.15s ease, border-color 0.15s ease;
  }
  .tgl.on {
    background: var(--sw-accent);
    border-color: var(--sw-accent);
  }
  .tgl:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .knob {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.4);
    transition: transform 0.15s ease;
  }
  /* 40 − 3 (left) − 3 (right) − 18 (knob) = 16px of travel. */
  .tgl.on .knob {
    transform: translateX(16px);
  }
  /* Respect a user who asked the OS to stop animating things. */
  @media (prefers-reduced-motion: reduce) {
    .tgl,
    .knob {
      transition: none;
    }
  }
</style>

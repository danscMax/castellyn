<script lang="ts">
  // Shared modal wrapper (ported from Sweet Whisper's ModalShell): backdrop + centered card +
  // fade/scale animation + Escape/Enter + a simple focus trap. Each dialog passes its own
  // header/body/footer as the default snippet and keeps its own open/close wiring.
  import type { Snippet } from 'svelte';
  import { tick } from 'svelte';
  import { fade } from 'svelte/transition';
  import { t } from '$lib/i18n';

  let {
    open = false,
    onClose,
    onEnter,
    size = 'md',
    role = 'dialog',
    closeOnBackdrop = true,
    initialFocus = null,
    labelledBy,
    describedBy,
    children
  }: {
    open?: boolean;
    onClose: () => void;
    onEnter?: () => void;
    size?: 'sm' | 'md' | 'lg' | 'xl';
    role?: 'dialog' | 'alertdialog';
    closeOnBackdrop?: boolean;
    /** CSS selector (within the card) of the element to focus on open; defaults to the card itself.
        Lets a destructive dialog put initial focus on its SAFE choice (Cancel) instead of nothing. */
    initialFocus?: string | null;
    /** id of the element labelling this dialog (typically the title h3). Screen reader announces it. */
    labelledBy?: string | null;
    /** id of the element describing this dialog (typically the message p or the type-to-confirm input). */
    describedBy?: string | null;
    children: Snippet;
  } = $props();

  const WIDTH: Record<string, string> = {
    sm: '420px',
    md: '500px',
    lg: '800px',
    xl: '850px'
  };

  let cardEl = $state<HTMLDivElement | null>(null);
  const FOCUSABLE =
    'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

  // Move focus into the dialog on open, restore it on close.
  $effect(() => {
    if (!open) return;
    const prev = document.activeElement as HTMLElement | null;
    tick().then(() => {
      const target = initialFocus ? cardEl?.querySelector<HTMLElement>(initialFocus) : null;
      (target ?? cardEl)?.focus();
    });
    return () => prev?.focus?.();
  });

  // Lock background scroll while open; restore the exact prior value on close/unmount.
  $effect(() => {
    if (!open) return;
    const prevOverflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    return () => {
      document.body.style.overflow = prevOverflow;
    };
  });

  function onBackdrop(e: MouseEvent) {
    if (closeOnBackdrop && e.target === e.currentTarget) onClose();
  }
  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      onClose();
      return;
    }
    if (e.key === 'Enter' && onEnter) {
      const a = document.activeElement;
      if (a instanceof HTMLElement) {
        // Buttons/links/selects run their own native Enter activation. A <textarea> needs
        // Enter for newlines. A plain <input> should still submit — only exclude one wired
        // to a <datalist>, where Enter accepts the suggestion instead of submitting.
        if (['BUTTON', 'A', 'SELECT', 'TEXTAREA'].includes(a.tagName)) return;
        if (a instanceof HTMLInputElement && a.list) return;
      }
      onEnter();
      return;
    }
    if (e.key === 'Tab' && cardEl) {
      const f = Array.from(cardEl.querySelectorAll<HTMLElement>(FOCUSABLE)).filter((el) => el.offsetParent !== null);
      if (!f.length) return;
      const first = f[0];
      const last = f[f.length - 1];
      // The card itself (tabindex=-1) holds focus on open, and the backdrop sits BEFORE it in the
      // overlay — without this branch Shift+Tab walked out of the dialog entirely, taking the
      // Escape handler (bound on .overlay) with it.
      if (!cardEl.contains(document.activeElement)) {
        e.preventDefault();
        (e.shiftKey ? last : first).focus();
        return;
      }
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    }
  }
</script>

{#if open}
  <div class="overlay" onkeydown={onKeydown} role="presentation">
    <!-- tabindex=-1: a mouse-only affordance. In the tab order it is a stop BEFORE the card that
         leads nowhere, and Escape already provides the keyboard close. -->
    <button type="button" class="backdrop" tabindex="-1" aria-label={t('common.close')} onclick={onBackdrop} transition:fade={{ duration: 130 }}></button>
    <div
      bind:this={cardEl}
      class="card"
      style="width: min({WIDTH[size]}, 94vw)"
      {role}
      aria-modal="true"
      aria-labelledby={labelledBy}
      aria-describedby={describedBy}
      tabindex="-1"
    >
      {@render children()}
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 50;
  }
  .backdrop {
    position: absolute;
    inset: 0;
    border: none;
    padding: 0;
    background: rgba(0, 0, 0, 0.55);
    backdrop-filter: blur(3px);
    cursor: default;
  }
  .card {
    position: relative;
    max-height: 92vh;
    overflow-y: auto;
    background: var(--sw-bg-secondary);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-lg);
    padding: var(--sw-space-6);
    box-shadow: 0 24px 60px rgba(0, 0, 0, 0.45);
    outline: none;
    animation: modal-in 0.18s ease-out;
  }
  @keyframes modal-in {
    from {
      opacity: 0;
      transform: scale(0.96);
    }
    to {
      opacity: 1;
      transform: scale(1);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .card {
      animation: none;
    }
  }
</style>

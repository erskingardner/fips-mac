<script lang="ts">
  export type IconName =
    | "brand"
    | "node"
    | "overview"
    | "peers"
    | "transports"
    | "access"
    | "settings"
    | "refresh"
    | "copy"
    | "udp"
    | "tcp"
    | "tor"
    | "link";

  let {
    name,
    size = 18,
    strokeWidth = 1.7,
    label = "",
  }: { name: IconName; size?: number; strokeWidth?: number; label?: string } = $props();
</script>

<svg
  class="icon icon-{name}"
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width={strokeWidth}
  stroke-linecap="round"
  stroke-linejoin="round"
  role={label ? "img" : undefined}
  aria-label={label || undefined}
  aria-hidden={label ? undefined : "true"}
>
  {#if name === "brand"}
    <path d="M5 4h13M5 4v16M5 11h9" />
    <circle cx="5" cy="4" r="1.75" />
    <circle cx="11.5" cy="4" r="1.45" />
    <circle cx="18" cy="4" r="1.75" />
    <circle cx="5" cy="11" r="1.75" />
    <circle cx="14" cy="11" r="1.75" />
    <circle cx="5" cy="20" r="1.75" />
  {:else if name === "node"}
    <path d="m12 2.8 7.8 4.5v9.4L12 21.2l-7.8-4.5V7.3L12 2.8Z" />
    <path class="soft" d="M12 12 4.5 7.5M12 12l7.5-4.5M12 12v8.7" />
    <circle cx="12" cy="12" r="2.15" />
    <circle cx="4.5" cy="7.5" r="1.35" />
    <circle cx="19.5" cy="7.5" r="1.35" />
    <circle cx="12" cy="20.5" r="1.35" />
  {:else if name === "overview"}
    <rect x="3.5" y="4" width="17" height="16" rx="2.5" />
    <path d="m7 14 3-3 2.5 2 4.5-5" />
    <path class="soft" d="M7 17h10" />
  {:else if name === "peers"}
    <circle cx="8" cy="8" r="3" />
    <circle cx="17" cy="15.5" r="3" />
    <path d="m10.3 10 4.4 3.5M3.5 19c.8-3.1 2.5-4.7 5.2-4.7M15 5c2.8.1 4.6 1.6 5.4 4.4" />
  {:else if name === "transports"}
    <path d="M4 8h14M15 5l3 3-3 3M20 16H6M9 13l-3 3 3 3" />
  {:else if name === "settings"}
    <path d="M4 6h10M18 6h2M4 12h2M10 12h10M4 18h8M16 18h4" />
    <circle cx="16" cy="6" r="2" />
    <circle cx="8" cy="12" r="2" />
    <circle cx="14" cy="18" r="2" />
  {:else if name === "access"}
    <path d="M12 3 19 6v5c0 4.5-2.7 7.9-7 10-4.3-2.1-7-5.5-7-10V6l7-3Z" />
    <path d="m8.5 12 2.3 2.3 4.8-4.8" />
  {:else if name === "refresh"}
    <path d="M20 7v5h-5M4 17v-5h5" />
    <path d="M18.1 12A6.5 6.5 0 0 0 6.8 7.6L4 12M5.9 12a6.5 6.5 0 0 0 11.3 4.4L20 12" />
  {:else if name === "copy"}
    <rect x="8" y="8" width="11" height="11" rx="2" />
    <path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2" />
  {:else if name === "udp"}
    <circle cx="12" cy="12" r="2" />
    <path d="M8.5 8.5a5 5 0 0 0 0 7M15.5 8.5a5 5 0 0 1 0 7M5.5 5.5a9.2 9.2 0 0 0 0 13M18.5 5.5a9.2 9.2 0 0 1 0 13" />
  {:else if name === "tcp"}
    <path d="M5 8h12M14 5l3 3-3 3M19 16H7M10 13l-3 3 3 3" />
    <circle cx="4" cy="8" r="1.25" />
    <circle cx="20" cy="16" r="1.25" />
  {:else if name === "tor"}
    <path d="M12 21a7 7 0 0 0 0-14V3M12 21a7 7 0 0 1 0-14M12 17a3 3 0 0 0 0-6M12 7h3M12 11h3M12 15h2" />
  {:else}
    <path d="M9.5 14.5 14.5 9M7.2 17.8l-1 1a3.5 3.5 0 0 1-5-5l4-4a3.5 3.5 0 0 1 5 0M16.8 6.2l1-1a3.5 3.5 0 0 1 5 5l-4 4a3.5 3.5 0 0 1-5 0" transform="translate(-.5 -.5)" />
  {/if}
</svg>

<style>
  .icon { display: block; overflow: visible; }
  .soft { opacity: .42; }
  .icon-brand circle { fill: currentColor; stroke: none; }
  .icon-node circle { fill: currentColor; stroke: none; }
</style>

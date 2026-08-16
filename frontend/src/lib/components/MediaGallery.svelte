<script lang="ts">
  import type { AssetHeadDto } from "$lib/bindings/AssetHeadDto";
  import { mediaSrc } from "$lib/api/media";
  import { m } from "$lib/paraglide/messages.js";

  let { assets, urls = [] }: { assets: AssetHeadDto[]; urls?: string[] } =
    $props();
</script>

{#if assets.length === 0 && urls.length === 0}
  <p class="empty-row">{m.empty_media()}</p>
{:else}
  <div class="media-gallery">
    {#each assets as asset (asset.id)}
      {#await mediaSrc(asset.id) then src}
        <figure>
          <img {src} alt={asset.name} loading="lazy" />
          <figcaption>{asset.name}</figcaption>
        </figure>
      {/await}
    {/each}
    {#each urls as url (url)}
      <figure>
        <!-- 上游给的外链,不经应用路由 -->
        <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -->
        <a href={url} target="_blank" rel="noreferrer noopener">{url}</a>
      </figure>
    {/each}
  </div>
{/if}

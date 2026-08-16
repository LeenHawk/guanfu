<script lang="ts">
  import type { AssetHeadDto } from "$lib/bindings/AssetHeadDto";
  import { SvelteSet } from "svelte/reactivity";

  import { mediaSrc } from "$lib/api/media";
  import { m } from "$lib/paraglide/messages.js";

  let { assets, urls = [] }: { assets: AssetHeadDto[]; urls?: string[] } =
    $props();

  /**
   * 进入视口才去取 src。
   *
   * `loading="lazy"` 只能推迟浏览器取图,救不了这里:桌面壳的 `mediaSrc`
   * 会把整份文件 base64 成 data URL,一旦提前 await,媒体库里每个资产都
   * 已经进内存了。所以要推迟的是"取 src"这一步本身。
   */
  let visible = new SvelteSet<number>();

  function watch(node: HTMLElement, id: number) {
    if (typeof IntersectionObserver === "undefined") {
      visible.add(id);
      return {};
    }
    const observer = new IntersectionObserver(
      (entries) => {
        if (!entries.some((entry) => entry.isIntersecting)) return;
        visible.add(id);
        observer.disconnect();
      },
      // 提前一屏开始取,滚到位时通常已经就绪。
      { rootMargin: "300px" },
    );
    observer.observe(node);
    return { destroy: () => observer.disconnect() };
  }
</script>

{#if assets.length === 0 && urls.length === 0}
  <p class="empty-row">{m.empty_media()}</p>
{:else}
  <div class="media-gallery">
    {#each assets as asset (asset.id)}
      <figure use:watch={asset.id}>
        {#if visible.has(asset.id)}
          {#await mediaSrc(asset.id) then src}
            <img {src} alt={asset.name} loading="lazy" />
          {/await}
        {:else}
          <!-- 占位撑住格位,取到图时不会把后面的卡片顶下去 -->
          <div class="media-placeholder" aria-hidden="true"></div>
        {/if}
        <figcaption>{asset.name}</figcaption>
      </figure>
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

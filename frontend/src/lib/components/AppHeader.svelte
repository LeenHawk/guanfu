<script lang="ts">
  import { m } from "$lib/paraglide/messages.js";
  import { getLocale, setLocale } from "$lib/paraglide/runtime.js";
  import { applyTheme, savedTheme, type Theme } from "$lib/ui/theme";

  let theme = $state<Theme>("system");
  let locale = $state(getLocale());

  $effect(() => {
    theme = savedTheme();
    applyTheme(theme);
    const media = matchMedia("(prefers-color-scheme: dark)");
    const update = () => theme === "system" && applyTheme(theme);
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  });

  function changeTheme(event: Event) {
    theme = (event.currentTarget as HTMLSelectElement).value as Theme;
    applyTheme(theme);
  }

  function changeLocale(event: Event) {
    locale = (event.currentTarget as HTMLSelectElement).value as "zh-CN" | "en";
    document.documentElement.lang = locale;
    setLocale(locale);
  }
</script>

<header class="app-header">
  <div class="brand">
    <img src="/favicon.png" alt="" width="42" height="42" />
    <div>
      <h1>{m.app_title()}</h1>
      <p>{m.app_subtitle()}</p>
    </div>
  </div>
  <div class="preferences">
    <label>
      <span class="sr-only">{m.language()}</span>
      <select value={locale} onchange={changeLocale} aria-label={m.language()}>
        <option value="zh-CN">中文</option>
        <option value="en">English</option>
      </select>
    </label>
    <label>
      <span class="sr-only">{m.theme()}</span>
      <select value={theme} onchange={changeTheme} aria-label={m.theme()}>
        <option value="system">{m.theme_system()}</option>
        <option value="light">{m.theme_light()}</option>
        <option value="dark">{m.theme_dark()}</option>
      </select>
    </label>
  </div>
</header>

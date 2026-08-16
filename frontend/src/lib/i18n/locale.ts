export type Locale = "zh-CN" | "zh-TW" | "en";

/**
 * `<html lang>` 的取值。
 *
 * 繁体要的是不同字形而非不同字符,靠 CSS 的 `:lang()` 选字体。而
 * `:lang(zh-Hant)` 按前缀匹配,`lang="zh-TW"` 匹配不上——必须写成
 * 带字形子标签的 `zh-Hant-TW`,繁体字形才会真的生效。
 */
export function htmlLang(locale: Locale): string {
  return locale === "zh-TW" ? "zh-Hant-TW" : locale;
}

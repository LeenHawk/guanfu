/**
 * 极小的 markdown 子集:段落、**粗体**、*斜体*、`代码`。
 *
 * 角色扮演文本里 *动作* 极常见,所以斜体是必需的;先转义 HTML 再套用
 * 行内规则,保证模型输出无法注入标记。
 */
export function renderMarkdown(text: string): string {
  return text
    .split(/\n{2,}/)
    .map((block) => `<p>${inline(escapeHtml(block))}</p>`)
    .join("");
}

function escapeHtml(text: string): string {
  return text
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function inline(text: string): string {
  return text
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/\*([^*]+)\*/g, "<em>$1</em>")
    .replaceAll("\n", "<br />");
}

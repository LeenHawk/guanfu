export type Theme = "system" | "light" | "dark";

export function applyTheme(theme: Theme): void {
  localStorage.setItem("guanfu-theme", theme);
  const dark =
    theme === "dark" ||
    (theme === "system" && matchMedia("(prefers-color-scheme: dark)").matches);
  document.documentElement.classList.toggle("dark", dark);
  const favicon = document.querySelector<HTMLLinkElement>("#app-favicon");
  if (favicon) favicon.href = dark ? "/favicon-dark.svg" : "/favicon.png";
}

export function savedTheme(): Theme {
  const value = localStorage.getItem("guanfu-theme");
  return value === "light" || value === "dark" ? value : "system";
}

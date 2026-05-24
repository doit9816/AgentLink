import { createI18n } from "vue-i18n";
import zhCN from "../locales/zh-CN.json";
import en from "../locales/en.json";

export const SUPPORTED_LOCALES = ["zh-CN", "en"];
export const LOCALE_SYSTEM = "system";

export function normalizeLocale(value) {
  if (value === "zh-CN" || value === "en") return value;
  return null;
}

export function detectSystemLocale() {
  const lang = String(navigator.language || "en").toLowerCase();
  if (lang.startsWith("zh")) return "zh-CN";
  return "en";
}

export function resolveLocale(savedPreference) {
  const saved = normalizeLocale(savedPreference);
  if (saved) return saved;
  return detectSystemLocale();
}

export function applyDocumentLocale(locale) {
  document.documentElement.lang = locale === "zh-CN" ? "zh-CN" : "en";
}

const initialLocale = resolveLocale(null);

export const i18n = createI18n({
  legacy: false,
  locale: initialLocale,
  fallbackLocale: "en",
  messages: {
    "zh-CN": zhCN,
    en
  }
});

export function setAppLocale(locale) {
  i18n.global.locale.value = locale;
  applyDocumentLocale(locale);
}

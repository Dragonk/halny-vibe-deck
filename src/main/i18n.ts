// Minimal i18n map for main-process UI surfaces (currently: native context menu).
// Kept separate from the renderer i18n so the main bundle stays dependency-free.

import { app } from "electron";

export type MenuLabelKey = "cut" | "copy" | "paste" | "selectAll";

const LABELS: Record<string, Record<MenuLabelKey, string>> = {
  en: { cut: "Cut", copy: "Copy", paste: "Paste", selectAll: "Select all" },
  pl: { cut: "Wytnij", copy: "Kopiuj", paste: "Wklej", selectAll: "Zaznacz wszystko" },
  fr: { cut: "Couper", copy: "Copier", paste: "Coller", selectAll: "Tout sélectionner" },
  es: { cut: "Cortar", copy: "Copiar", paste: "Pegar", selectAll: "Seleccionar todo" },
  de: { cut: "Ausschneiden", copy: "Kopieren", paste: "Einfügen", selectAll: "Alles auswählen" },
  it: { cut: "Taglia", copy: "Copia", paste: "Incolla", selectAll: "Seleziona tutto" },
};

/** Labels for the requested app language; "auto" falls back to the OS locale. */
export function menuLabels(language: string | undefined): Record<MenuLabelKey, string> {
  const key = language && language !== "auto" ? language : (app.getLocale() || "en").split("-")[0];
  return LABELS[key] ?? LABELS.en;
}

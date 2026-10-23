// Localized display names for ACP config option values (mode / thinking).
// The CLI sends option values like "accept-edits" or "medium"; the UI
// shows them in the language selected in Settings.

export type TermsLang = 'en' | 'pl' | 'fr' | 'es' | 'de' | 'it'

const SUPPORTED: string[] = ['en', 'pl', 'fr', 'es', 'de', 'it']

export function resolveLanguage(language: string | undefined | null): TermsLang {
  let base: string = 'en'
  if (language && language !== 'auto') {
    base = language
  } else if (typeof navigator !== 'undefined' && navigator.language) {
    base = navigator.language.split('-')[0]
  }
  base = base.toLowerCase()
  return (SUPPORTED.includes(base) ? base : 'en') as TermsLang
}

const MODE_NAMES: Record<TermsLang, Record<string, string>> = {
  en: { ask: 'Ask', 'accept-edits': 'Accept Edits', 'smart-approve': 'Smart Approve', 'auto-approve': 'Auto Approve', explore: 'Explore', lean: 'Lean' },
  pl: { ask: 'Pytaj', 'accept-edits': 'Akceptuj edycje', 'smart-approve': 'Inteligentne zatwierdzanie', 'auto-approve': 'Autozatwierdzanie', explore: 'Eksploruj', lean: 'Oszczędny' },
  fr: { ask: 'Demander', 'accept-edits': 'Accepter les modifications', 'smart-approve': 'Approbation intelligente', 'auto-approve': 'Approbation automatique', explore: 'Explorer', lean: 'Léger' },
  es: { ask: 'Preguntar', 'accept-edits': 'Aceptar ediciones', 'smart-approve': 'Aprobación inteligente', 'auto-approve': 'Aprobación automática', explore: 'Explorar', lean: 'Ligero' },
  de: { ask: 'Fragen', 'accept-edits': 'Bearbeitungen akzeptieren', 'smart-approve': 'Smart-Genehmigung', 'auto-approve': 'Automatisch genehmigen', explore: 'Erkunden', lean: 'Schlank' },
  it: { ask: 'Chiedi', 'accept-edits': 'Accetta modifiche', 'smart-approve': 'Approvazione smart', 'auto-approve': 'Approvazione automatica', explore: 'Esplora', lean: 'Snello' },
}

const THINKING_NAMES: Record<TermsLang, Record<string, string>> = {
  en: { off: 'Off', low: 'Low', medium: 'Medium', high: 'High', max: 'Max' },
  pl: { off: 'Wyłączony', low: 'Niski', medium: 'Średni', high: 'Wysoki', max: 'Maksymalny' },
  fr: { off: 'Désactivé', low: 'Faible', medium: 'Moyen', high: 'Élevé', max: 'Maximal' },
  es: { off: 'Desactivado', low: 'Bajo', medium: 'Medio', high: 'Alto', max: 'Máximo' },
  de: { off: 'Aus', low: 'Niedrig', medium: 'Mittel', high: 'Hoch', max: 'Maximal' },
  it: { off: 'Disattivato', low: 'Basso', medium: 'Medio', high: 'Alto', max: 'Massimo' },
}

export function translateModeName(value: string, lang: TermsLang): string {
  return MODE_NAMES[lang][value] ?? value
}

export function translateThinkingLevel(value: string, lang: TermsLang): string {
  return THINKING_NAMES[lang][value] ?? value
}

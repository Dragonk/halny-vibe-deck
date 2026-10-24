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

const MODE_DESCRIPTIONS: Record<TermsLang, Record<string, string>> = {
  en: { ask: 'Ask before every file edit and command', 'accept-edits': 'File edits are auto-accepted, commands still ask', 'smart-approve': 'The agent decides what is safe to auto-approve', 'auto-approve': 'Edits and commands are approved without asking', explore: 'Read-only: the agent explores without changing files', lean: 'Minimal context, fastest responses' },
  pl: { ask: 'Pytaj przed każdą edycją pliku i komendą', 'accept-edits': 'Edycje plików zatwierdzane automatycznie, komendy pytają', 'smart-approve': 'Agent sam decyduje, co może bezpiecznie zatwierdzić', 'auto-approve': 'Edycje i komendy zatwierdzane bez pytania', explore: 'Tryb tylko do odczytu — agent niczego nie zmienia', lean: 'Minimalny kontekst, najszybsze odpowiedzi' },
  fr: { ask: 'Demander avant chaque modification de fichier et chaque commande', 'accept-edits': 'Modifications de fichiers acceptées automatiquement, les commandes demandent', 'smart-approve': 'L’agent décide ce qui peut être approuvé automatiquement', 'auto-approve': 'Modifications et commandes approuvées sans demander', explore: 'Lecture seule : l’agent explore sans modifier les fichiers', lean: 'Contexte minimal, réponses les plus rapides' },
  es: { ask: 'Preguntar antes de cada edición de archivo y comando', 'accept-edits': 'Las ediciones de archivos se aceptan automáticamente, los comandos preguntan', 'smart-approve': 'El agente decide qué es seguro aprobar automáticamente', 'auto-approve': 'Ediciones y comandos aprobados sin preguntar', explore: 'Solo lectura: el agente explora sin cambiar archivos', lean: 'Contexto mínimo, respuestas más rápidas' },
  de: { ask: 'Vor jeder Dateiänderung und jedem Befehl fragen', 'accept-edits': 'Dateiänderungen werden automatisch akzeptiert, Befehle fragen', 'smart-approve': 'Der Agent entscheidet, was sicher automatisch genehmigt wird', 'auto-approve': 'Änderungen und Befehle werden ohne Nachfragen genehmigt', explore: 'Nur Lesen: der Agent erkundet, ohne Dateien zu ändern', lean: 'Minimaler Kontext, schnellste Antworten' },
  it: { ask: 'Chiedi prima di ogni modifica ai file e ogni comando', 'accept-edits': 'Le modifiche ai file sono accettate automaticamente, i comandi chiedono', 'smart-approve': 'L’agente decide cosa è sicuro approvare automaticamente', 'auto-approve': 'Modifiche e comandi approvati senza chiedere', explore: 'Sola lettura: l’agente esplora senza modificare i file', lean: 'Contesto minimo, risposte più rapide' },
}

const THINKING_DESCRIPTIONS: Record<TermsLang, Record<string, string>> = {
  en: { off: 'No extended reasoning', low: 'Light reasoning, fast answers', medium: 'Balanced reasoning depth', high: 'Deep reasoning for complex tasks', max: 'Maximum reasoning: slowest and most thorough' },
  pl: { off: 'Bez dodatkowego rozumowania', low: 'Lekkie rozumowanie, szybkie odpowiedzi', medium: 'Zrównoważona głębia rozumowania', high: 'Głębokie rozumowanie dla złożonych zadań', max: 'Maksymalne rozumowanie — najwolniejsze, ale najdokładniejsze' },
  fr: { off: 'Sans raisonnement étendu', low: 'Raisonnement léger, réponses rapides', medium: 'Profondeur de raisonnement équilibrée', high: 'Raisonnement approfondi pour les tâches complexes', max: 'Raisonnement maximal : le plus lent et le plus approfondi' },
  es: { off: 'Sin razonamiento extendido', low: 'Razonamiento ligero, respuestas rápidas', medium: 'Profundidad de razonamiento equilibrada', high: 'Razonamiento profundo para tareas complejas', max: 'Razonamiento máximo: el más lento y minucioso' },
  de: { off: 'Keine erweiterte Denkphase', low: 'Leichte Denkphase, schnelle Antworten', medium: 'Ausgewogene Denktiefe', high: 'Tiefe Denkphase für komplexe Aufgaben', max: 'Maximale Denkphase: am langsamsten und gründlichsten' },
  it: { off: 'Senza ragionamento esteso', low: 'Ragionamento leggero, risposte rapide', medium: 'Profondità di ragionamento bilanciata', high: 'Ragionamento profondo per attività complesse', max: 'Ragionamento massimo: il più lento e approfondito' },
}

export function translateModeDescription(value: string, lang: TermsLang): string {
  return MODE_DESCRIPTIONS[lang][value] ?? ''
}

export function translateThinkingDescription(value: string, lang: TermsLang): string {
  return THINKING_DESCRIPTIONS[lang][value] ?? ''
}

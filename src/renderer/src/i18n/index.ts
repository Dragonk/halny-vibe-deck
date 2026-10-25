import { useMemo } from 'react'
import { en } from './en'
import { pl } from './pl'
import { fr } from './fr'
import { es } from './es'
import { de } from './de'
import { it } from './it'

export type Language = 'auto' | 'en' | 'pl' | 'fr' | 'es' | 'de' | 'it'

export type TranslationKeys = keyof typeof en

const translations: Record<Exclude<Language, 'auto'>, Record<TranslationKeys, string>> = {
  en,
  pl,
  fr,
  es,
  de,
  it,
}

function getDeviceLanguage(): Exclude<Language, 'auto'> {
  const lang = navigator.language.split('-')[0]
  if (lang in translations) {
    return lang as keyof typeof translations
  }
  return 'en'
}

/** Resolved UI language for a config value ('auto' -> device language). */
export function resolveLanguage(language: Language): Exclude<Language, 'auto'> {
  if (language === 'auto') return getDeviceLanguage()
  return translations[language] ? language : 'en'
}

export function useTranslation(language: Language) {
  return useMemo(() => {
    if (language === 'auto') {
      return translations[getDeviceLanguage()]
    }
    return translations[language] ?? translations.en
  }, [language])
}

export function t(translations: Record<TranslationKeys, string>, key: TranslationKeys, params?: Record<string, string | number>): string {
  let text = translations[key] ?? key
  if (params) {
    Object.entries(params).forEach(([k, v]) => {
      text = text.replace(new RegExp(`{{${k}}}`, 'g'), String(v))
    })
  }
  return text
}

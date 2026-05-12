// react-i18next 초기화.
//
// 시스템 언어를 탐지하고 (`navigator.language`) WinMux가 지원하는 두 언어
// 중 하나로 fallback한다. spec § Settings → General의 "UI language" 옵션은
// 후속에 추가된다(`docs/spec/10-i18n.md`).

import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';

import en from '@/locales/en.json';
import ko from '@/locales/ko.json';

type SupportedLanguage = 'en' | 'ko';

function pickLanguage(): SupportedLanguage {
  const candidates: string[] = [];
  if (typeof navigator !== 'undefined') {
    if (navigator.languages) {
      for (const c of navigator.languages) candidates.push(c);
    }
    if (navigator.language) {
      candidates.push(navigator.language);
    }
  }
  for (const c of candidates) {
    const base = c.toLowerCase().split('-')[0];
    if (base === 'ko') return 'ko';
    if (base === 'en') return 'en';
  }
  return 'en';
}

void i18n.use(initReactI18next).init({
  resources: {
    en: { translation: en },
    ko: { translation: ko },
  },
  lng: pickLanguage(),
  fallbackLng: 'en',
  interpolation: { escapeValue: false },
  returnNull: false,
});

export default i18n;

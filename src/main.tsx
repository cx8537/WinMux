import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { App } from '@/App';
import '@/lib/i18n';
import '@/styles/globals.css';

function getRoot(): HTMLElement {
  const root = document.getElementById('root');
  if (!root) {
    throw new Error('root element #root must exist');
  }
  return root;
}

createRoot(getRoot()).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

type LogFields = Record<string, unknown>;

const isDev = import.meta.env.DEV;

function emit(level: 'info' | 'warn' | 'error', message: string, fields?: LogFields): void {
  if (!isDev) return;
  // Placeholder transport. The production path will forward into the
  // Tauri-side `tracing` infrastructure; until that lands the dev
  // console is the only sink. `console.warn`/`console.error` are
  // permitted by the ESLint rule for the logger module specifically.
  if (level === 'error') {
    console.error(message, fields ?? {});
    return;
  }
  if (level === 'warn') {
    console.warn(message, fields ?? {});
    return;
  }
  console.warn(`[info] ${message}`, fields ?? {});
}

export const logger = {
  info(message: string, fields?: LogFields): void {
    emit('info', message, fields);
  },
  warn(message: string, fields?: LogFields): void {
    emit('warn', message, fields);
  },
  error(message: string, fields?: LogFields): void {
    emit('error', message, fields);
  },
};

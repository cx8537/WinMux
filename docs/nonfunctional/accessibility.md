# Accessibility

> Keyboard access, IME, CJK fonts, contrast, and what's explicitly
> out of scope.

WinMux is a developer tool. Its primary surface is a terminal, which
is text by definition. That makes accessibility both unusually
important (keyboard navigation is the whole point) and unusually
challenging (a screen reader cannot meaningfully narrate a tmux
status line).

This document is honest about what's supported and what isn't.

---

## Keyboard

Every feature is reachable by keyboard. This is non-negotiable.

- Menu items in the tray menu: arrow keys + Enter.
- Modals: Tab order, Esc closes, Enter submits.
- Window switching: prefix bindings (tmux-compatible).
- Pane navigation: prefix + arrows.
- Copy mode: vi or emacs keys (configurable).
- Settings: every field reachable; no mouse-only widgets.

We do not introduce accelerator keys that conflict with common screen
readers (NVDA, JAWS) or Windows shortcuts. The prefix key is
configurable; users who find `Ctrl+B` conflicts with their assistive
technology can change it.

### Focus management

shadcn/ui's primitives (Radix UI under the hood) handle most focus
management correctly. We rely on this:

- Focus moves into a newly opened modal.
- Focus returns to the trigger element when the modal closes.
- Focus indicators are visible (not removed by `outline: none`).

Custom components are reviewed for the same behavior.

### Visible focus indicator

Tailwind's `focus-visible:` utilities provide a clear ring on
keyboard-focused elements. Disabled `outline: none` is forbidden in
the project's CSS.

---

## Screen Readers

**Basic support only.** Full screen-reader narration of a terminal
multiplexer's output is out of scope.

What works:

- ARIA labels on all interactive elements (buttons, links, inputs).
- The menu structure is semantic (proper `<nav>` and ARIA roles via
  shadcn/ui).
- Modal dialogs announce themselves.
- Status bar regions are `role="status"` for important updates.

What does not work:

- Live narration of PTY output. xterm.js renders to a canvas; the
  text isn't in the DOM in a screen-reader-friendly way. Some
  terminals have experimental ARIA modes; we have not implemented
  one.
- The panel grid layout (multiple panes) is not announced as a
  spatial structure.

A user who depends primarily on a screen reader will likely find a
non-tmux terminal a better fit. We do not claim otherwise.

---

## IME (Input Method Editor)

Full IME support is required, since the primary user is Korean.

### Korean

- Standard Windows Korean IME composes Hangul.
- During composition, `PtyInput` is not sent.
- On `compositionend`, the composed text is sent as one frame.
- The prefix key (`Ctrl+B` and other modifiers) is detected at
  `keydown`, **before** IME composition, so prefix interception
  works regardless of IME state.
- If a user presses the prefix during composition, the composition
  is canceled and the AwaitingCommand state is entered.

### Japanese, Chinese

The same compositionstart / compositionupdate / compositionend
mechanism applies. Tested with the Microsoft IME for Japanese and
Pinyin IME for Chinese; both compose correctly into the xterm.js
textarea.

### Tray UI strings

UI labels are short enough that IME interactions in modals (typing a
session name in Korean, for example) work without special handling.
Standard React form input behavior covers this.

---

## CJK Font Rendering

CJK characters must render correctly even when the UI language is
English.

### Font fallback chain

Default in `winmux.toml`:

```toml
font_family = "Cascadia Code, Consolas, D2Coding, 'Noto Sans Mono CJK KR', monospace"
```

Rationale:

- **Cascadia Code** ships with Windows 11; great Latin glyphs and
  programming ligatures.
- **Consolas** is a Windows fallback.
- **D2Coding** is widely installed by Korean developers.
- **Noto Sans Mono CJK KR** is a free Google font that many users
  install.
- The generic `monospace` is the final safety net.

We do **not** bundle fonts. The user installs what they prefer. The
default chain works for most Korean-installed Windows machines out
of the box.

### Wide characters

xterm.js's WebGL renderer handles double-width East Asian characters.
The cursor advances by 2 cells. Background colors fill correctly.
This is tested manually with `echo "안녕하세요"` and the equivalent
in Japanese and Chinese.

---

## Contrast and Color

### Themes

- **Dark theme (M1, default).** Color tokens are chosen so foreground
  text has a WCAG AA contrast ratio (≥ 4.5:1) against the background.
- **Light theme (M2+).** Same target.
- **System theme (M2+).** Follows Windows preference; the WCAG
  targets still apply.

### High-contrast mode

Windows users with "High contrast" enabled get a Windows-themed
visual style. xterm.js can be configured to use system colors via:

```typescript
terminal.options.theme = systemThemeFromMedia();
```

Where `systemThemeFromMedia()` reads CSS media queries (`prefers-contrast:
more`, `forced-colors: active`) and selects appropriate colors.

In forced-colors mode (Windows high contrast), we yield to the
system: terminal background and foreground use system tokens, and
custom UI chrome uses the same.

### Color in the status bar and UI chrome

We use Tailwind palette tokens via CSS variables. Hard-coded hex
values are forbidden in components — they bypass the theme system
and break high-contrast.

### Color in terminal output

The ANSI palette is part of the theme. We do not change colors of
output produced by user programs; that's up to the program. We do
ensure default backgrounds and foregrounds give programs enough
contrast room to render legible output.

---

## Motion

- Animations are short (< 200 ms) and small (modal fade-in, pane
  resize easing).
- `prefers-reduced-motion: reduce` disables all non-essential
  animation. Detected via CSS media query and a Tailwind variant.

---

## Tooltips and Affordances

Every icon button has a tooltip with the same content a sighted user
would expect (and that a screen reader can read). Tooltips are
keyboard-triggerable (on focus, not just hover).

shadcn/ui's Tooltip primitive handles this; we apply it consistently.

---

## Settings UI

Settings → General has an "Accessibility" subsection with:

- "Reduce motion" toggle (overrides the system preference if the user
  wants).
- "Use system colors" (forces high-contrast colors).
- "Increase font size" shortcut.
- A note: "Screen reader narration of terminal output is not
  supported. Other features are keyboard-accessible."

The disclosure prevents users from discovering the limit through
frustration.

---

## Internationalization Overlap

i18n is in [`../spec/10-i18n.md`](../spec/10-i18n.md). Accessibility
overlaps where:

- Translated strings stay clear and unabbreviated; both English and
  Korean text fits screen-reader announcement.
- ARIA labels are also localized.
- Korean text in UI uses proper Hangul rendering with system fonts;
  the same fallback chain applies to UI chrome, not just terminal
  content.

---

## What We Don't Promise

- Full screen-reader workflow. A power user of NVDA / JAWS will find
  the terminal output unreadable to assistive technology. This is a
  fundamental limitation of canvas-based terminal renderers.
- Voice control workflows. Not implemented; not tested.
- Switch access. Not implemented.

For these workflows, mainstream terminals with DOM-based renderers
(e.g., Hyper) may be more accessible, at the cost of features and
performance.

---

## Testing

Automated: limited. Snapshot tests of ARIA attributes on key
components. ESLint `jsx-a11y/*` rules at warn level (set to error for
clear violations like missing `alt` text).

Manual: keyboard-only navigation pass before each release, documented
in [`../ops/manual-test-checklist.md`](../ops/manual-test-checklist.md).
Verify:

- Every menu reachable.
- Every modal closable with Esc.
- Tab order is sensible.
- Focus indicator visible.
- No mouse-only operation.

High-contrast mode toggled on/off and the UI inspected.

---

## Related Docs

- IME details → [`../spec/04-key-handling.md`](../spec/04-key-handling.md)
- i18n → [`../spec/10-i18n.md`](../spec/10-i18n.md)
- GUI structure → [`../spec/07-tray-and-gui.md`](../spec/07-tray-and-gui.md)

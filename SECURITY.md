# Security Policy

## Supported Versions

WinMux is pre-release. Only the latest commit on `main` is supported.
Once 1.0 ships, this policy will list supported minor versions.

## Reporting a Vulnerability

WinMux is a single-author open source project. Please report security
issues through one of these channels:

1. **GitHub Security Advisories** (preferred):
   <https://github.com/cx8537/WinMux/security/advisories/new>
   Private until a fix is ready.

2. **GitHub Issue** with the `security` label, only if the issue is
   already public or affects no real users yet.

Do **not** include exploit details in public issues unless the
vulnerability is already widely known.

## Response Time

Best effort. This is a single-author hobby project — replies may take
days or longer. Critical issues will be prioritized.

## Threat Model

WinMux's threat model and what it does and does not protect against
are documented in detail:

- [`docs/nonfunctional/security.md`](docs/nonfunctional/security.md)

In short:

**In scope:**
- Isolation between different Windows user accounts on the same PC
- Named Pipe impersonation prevention
- Safe handling of untrusted `.tmux.conf` files
- Protection of sensitive data in scrollback and logs

**Out of scope:**
- Attackers with Administrator or SYSTEM privileges
- Attackers with physical access to the user's machine
- Vulnerabilities in Windows itself, in dependencies, or in the user's
  shell
- Side channels (timing, cache, electromagnetic, etc.)

## Disclosure Policy

For confirmed vulnerabilities:

1. Acknowledged within reasonable time after report.
2. Fix developed and tested in a private fork or advisory.
3. Coordinated disclosure: release notes credit the reporter unless
   anonymity is requested.
4. CVE assignment if the issue is severe enough; otherwise advisory
   only.

## Known Limitations

See [`docs/known-issues.md`](docs/known-issues.md) for current
limitations of WinMux that are by design, not bugs.

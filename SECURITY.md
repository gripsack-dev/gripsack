# Security policy

## Supported versions

Only the latest `core-vX.Y.Z` release receives fixes. Gripsack is pre-1.0 and
Linux/WSL x86_64 only.

## Reporting a vulnerability

Report privately through GitHub's
[security advisory form](https://github.com/gripsack-dev/gripsack/security/advisories/new).
Please do not open a public issue.

Include the `grip --version` output, the steps to reproduce, and what an attacker
gains. You will get an acknowledgement within a week.

## Scope

In scope:

- the `grip` binary and the Conda helper;
- the TypeScript SDK;
- the evaluation sandbox, meaning anything a workspace module can do beyond the
  evaluation boundary in `plan/0013-constrained-evaluation.md`;
- the store, generations and rollback;
- the release assets and `install.sh`.

What gripsack proves, tests and assumes is listed at
<https://gripsack.dev/docs/guarantees.html>. A way to break a stated guarantee
is in scope even when it needs a hostile local process.

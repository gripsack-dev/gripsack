# Native hook idempotency examples

The frontend remains TypeScript. These are optional **native Python 3 hook
programs**, with ordinary user filesystem/network authority. A temporary HOME
is not a sandbox for arbitrary native code.

## Production contract

- `GRIPSACK_ACTIVATION_INTENT_ID` is stable across interrupted replay.
- `GRIPSACK_ACTIVATION_ATTEMPT` increases when a started attempt is resumed.
- A new activation, including rollback to the same generation, has new IDs.
- Durable success, failure and supersession suppress replay. A started outcome
  is ambiguous: an external effect may already have happened.
- Replayable interrupted delivery is `at_least_once`; known failure is
  `warn_no_retry`. Neither eventual success nor generic exactly-once effects
  is promised. Post-activation failure never automatically rolls back.
- `grip hooks list --json` reads pending/archived outcomes without running them.
  Legacy records explicitly report unavailable identity until migration.

`grip hooks test`, `grip hooks test --duplicate`, and
`grip hooks test --crash-after-start` use generated private state, fixed
first-party actions and a loopback receiver. They never select live hooks or
accept a user script. The duplicate simulation produces **two append effects**
but **one receiver counter effect**, including a receiver restart. Its output
reports the actual attempts and native enforcement tier; it is not a
physical-power-loss or complete-process-tree claim.

## 1. Idempotent local replacement

`atomic_replace.py` creates and syncs a private temporary file, atomically
replaces the chosen derived output and syncs its parent. Repeating the same
value gives the same bytes. The destination's parent must already exist.

```sh
work=$(mktemp -d)
python3 examples/hooks/atomic_replace.py "$work/derived" --value configured
python3 examples/hooks/atomic_replace.py "$work/derived" --value configured
cat "$work/derived"
```

The caller must own this derived output. This deliberately replaces it; it is
not a substitute for gripsack ownership admission and does not preserve
hardlink topology or protect an unrelated external writer.

## 2. A cooperating local keyed operation

`keyed_increment.py` stores the stable token, operation digest and counter
increment in **one SQLite transaction**. Duplicate delivery with the same
payload returns the existing result. Reusing a token with a different payload
fails. State is private; SQLite runs with full synchronization. Use a dedicated
state directory whose parent already exists.

```sh
# Synthetic token only for this standalone example. In a real hook, use the
# core-injected value; do not replace it or generate one per attempt.
export GRIPSACK_ACTIVATION_INTENT_ID=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
python3 examples/hooks/keyed_increment.py --state-dir "$work/local"
python3 examples/hooks/keyed_increment.py --state-dir "$work/local"
```

The two results have `value: 1`; only the first has `applied: true`. A different
intent increments again. Only the database operation shares this atomicity.
Running an arbitrary command after committing the token would reintroduce a
crash window; no “done file after shell” wrapper is offered here.

## 3. An external request with receiver-side deduplication

Start the demonstration receiver in one terminal:

```sh
python3 examples/hooks/receiver.py --state-dir "$work/remote" --port 8123
```

Then deliver the same request twice:

```sh
export GRIPSACK_ACTIVATION_INTENT_ID=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
python3 examples/hooks/notify.py --endpoint http://127.0.0.1:8123/increment
python3 examples/hooks/notify.py --endpoint http://127.0.0.1:8123/increment
```

The receiver binds only loopback. It commits token and counter together before
acknowledging success. Stop/restart it with the **same state directory**, then
repeat: the value remains 1. The sender makes one bounded request, refuses
redirects and performs no automatic retries. Non-loopback endpoints require
HTTPS. Authentication and authorization for a real service are separate work;
this demonstration is not a production HTTP service.

## Declaring separate local and external actions

Use distinct declarations so local activation and external notification have
separate identity/outcome records. Replace the explicit example paths with
operator-approved absolute paths:

```ts
import { customHook, module } from "@gripsack/core";

export default module("notifications", {
  activate: [
    customHook("python3 /absolute/examples/hooks/keyed_increment.py --state-dir /absolute/private/local"),
    customHook("python3 /absolute/examples/hooks/notify.py --endpoint https://receiver.example/increment"),
  ],
});
```

A shell receipt binds its selected interpreter and shell-body digest, not all
subcommands, Python modules, dynamic libraries or remote behavior. These
examples need Python 3. Use a private, owned state directory; their SQLite
pathname opens assume host integrity rather than claiming native confinement.

For an ambiguous remote outcome, inspect the saved intent and query the
cooperating receiver by that token. Decide explicitly whether to reconcile,
retry with the same token, or perform a service-specific compensation. A fresh
activation has a new token and is not the same reconciliation operation.
Gripsack does not invent compensating actions for arbitrary remote services.

## Observed evidence and limits

The scripts were exercised in the Docker e2e environment: repeated replacement,
duplicate/distinct local tokens, conflicting-payload refusal, and a receiver
process killed/restarted between duplicate requests. Private directory/file
modes were checked. These observations do not certify physical power loss,
network filesystems or a production receiver's implementation.

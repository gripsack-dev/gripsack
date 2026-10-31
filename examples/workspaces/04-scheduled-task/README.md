# One task, manually or on a user timer

`gripsack.ts` declares a downloaded native formatter, one task, a store-scoped
mutation lock, and a daily local-time schedule. The same `format` task runs from
both entry points. A catalog declaration or import never installs anything;
applying `personal` authorizes installation.

Requirements:

- Linux/WSL2 x86_64, with a reachable **systemd 255 user manager**. An installed
  `systemctl` binary alone is insufficient. WSL needs its systemd integration
  and a working user session. Gripsack never changes linger or falls back to
  root/system services.
- The static Linux release `grip` executable. Schedule preparation retains that
  executable in the immutable store. Dynamically linked development executables
  are refused because their ambient loader/library closure is not retained.
- The local download fixture and its supported native runtime, a writable live
  checkout, and any explicitly declared external paths or noninteractive
  credentials. Native fetched and Conda packages use ordinary receipt-bound
  package admission, not an application container or build worker.
- The retained runner path cannot contain quotes, backslashes or control
  characters, which systemd 255 refuses in executable names. Spaces, Unicode,
  percent signs and dollar signs are encoded without shell interpretation.

From this example directory, approve its exact inspected source/policy digests
using the ordinary `grip trust inspect` / `grip trust add` workflow, then:

```sh
grip jobs capability --json
grip update formatter
grip task plan format --json
grip task run format --json
grip plan personal
grip apply personal
grip jobs status --json
```

`status` reports selected JobId/JobRevision, recorded registration outcomes,
actual manager observations, and invocation IDs. Registration failure is not
reported as an active timer merely because the generation committed. Inspect a
specific invocation with `grip task status INVOCATION --json` or read its bounded
log with `grip jobs log INVOCATION --json`.

Saved profile plans bind registration authority: the selected captured revision,
owned unit bytes, load/enablement state, fragment/drop-in paths and effective
calendar. An ordinary task starting, finishing or failing does not invalidate
an otherwise unchanged plan; those runtime observations remain visible in
`jobs status`. Disabling registration, changing units or switching the captured
revision still makes the saved binding stale. Unknown manager state cannot
authorize a saved plan.

## Update, disable, remove and rollback

- Edit task arguments/calendar/source selections, explicitly update pins when
  needed, and apply the profile. All tools and commands are prepared before old
  units are touched. Preparation failure leaves the old selection available.
- `grip jobs disable JOB_ID` makes an ordinary journaled generation transition,
  removes the owned timer/service files, and unregisters the timer. It does not
  kill already-admitted commands. Applying a profile that still selects the
  schedule installs it again.
- Remove `nightly` from the profile's `schedules` and apply to remove it
  declaratively. Removing the whole profile also retires its schedules.
- `grip rollback` restores the retained schedule/configuration selection. It
  does **not** undo formatter, backup, upload or other task side effects.
- Foreign collisions, edits and systemd drop-ins are preserved/refused. Save
  unexpected contents outside the managed destination and restore the exact
  owned precondition before retrying a deliberate lifecycle operation. Do not
  delete journal records or use a global `systemctl` cleanup command.

Registration uses the existing post-commit activation ledger. Interrupted
activation is reconciled by the normal lifecycle machinery; known failures are
visible terminal outcomes, not automatic retries. After repairing manager
availability or ownership, deliberately disable/reapply (or rollback to a
retained selection) to authorize a new registration attempt. Failed or unknown
registration/removal keeps captured roots conservatively protected; an elapsed
age, missing PID or unrelated successful activation never releases them.

## Timing and execution contract

Daily/weekly times are local calendar times, with **`Persistent=false`** and
native missed-trigger behavior. There is no portable catch-up promise or
exact-second/run-count guarantee. Sleep, session availability, shutdown, clock
changes, DST and a still-running service affect delivery. No root services,
implicit linger, cron syntax, intervals, named timezones or retry loops are
added.

Each event carries the exact captured JobId and JobRevision. The dispatcher
checks the current selection and acquires process retention under the same
lifecycle lock as update/disable. A stale callback refuses to run; an admitted
old invocation can finish with its original retained tools. The common task
runner enforces mutation keys across manual/timer and old/new revisions. Every
occasion reruns commands and postconditions: previous success is never a cache.

Captured declarations, literal Bash bodies, immutable inputs, package receipts,
and the runner survive source edits and core replacement. Timer events never
re-evaluate TypeScript or update source pins. Explicit host inputs and the
checkout remain live, not sandboxed; a changed external executable identity
blocks execution. Scheduled stdin is closed. The total default deadline is
1800 seconds; per-invocation logs are capped at 1 MiB. Failure, cancellation,
skipped-busy and indeterminate cleanup remain distinct outcomes. Unknown process
cleanup keeps its process root and conflict claim; there is no exactly-once or
arbitrary detached-process containment claim. Declared Docker service
prerequisites must already be healthy; a timer never starts services implicitly.

## Disposable real-manager smoke

No runtime evidence is claimed by committing this fixture. The integrated owner
runs:

```sh
pytest -q e2e/test_scheduled_jobs.py e2e/test_task_graphs.py
```

Those tests sandbox HOME and deliberately use an unreachable bus for scheduler
inspection/preparation cases. They never install into a developer's manager.

Prefer a disposable VM. A privileged container shares kernel-global state with
its host: an ordinary systemd boot can run `systemd-binfmt` and erase WSL's
Windows-executable registration. Before booting such a fixture, mask binfmt
service/mount/automount, sysctl, module-loading, time-setting and udev services;
verify the host registration is unchanged before and after. Do not rely on
shutdown hooks being isolated. The plan0055 development record documents the
observed failure and the corrected fixture; this is not permission to alter the
owner's kernel configuration or scheduled jobs.

For real calendar delivery, provision a **fresh disposable VM/container account**
with systemd 255 and a genuine systemd user manager. Its passwd home must
be `/tmp/gripsack-schedule-fixture-NAME`, and it must not be root. Account creation
is external test infrastructure, not a Gripsack registration fallback. Log in as
that account through the VM console, SSH/PAM, or an equivalent real user session;
do not enable linger as a shortcut. Make the static release binary, this fixture
script, and an already installed Deno readable/executable there. In that session:

```sh
id -u > "$HOME/.gripsack-disposable-scheduler"
export GRIPSACK_DENO=/opt/gripsack-fixture/deno
python3 /opt/gripsack-fixture/smoke_scheduled_jobs.py \
  --grip /opt/gripsack-fixture/grip
```

The script checks the passwd account and refuses a mere HOME override, a real
home directory, root, or reused fixture state. It uses structured subprocess
argv, no downloads, no clock changes and only its uniquely owned units. It
packages the static runner as an offline fetched native tool and exercises:

- the same task and postcondition manually and at an actual future calendar minute;
- actual `systemd-analyze --user verify`, explicit missed-trigger policy, and
  private capture permissions;
- retained tools after deleting the acquisition archive and no timer-time TS evaluation;
- stale callbacks, shared manual/timer mutation exclusion, in-flight revision
  update, disable and GC, then successful completion with the old capture;
- rollback, logs, unit drift refusal, declared removal, and collection.

For the combined sharing/scheduler journey, produce a signed relocatable bundle
on a separate producer, exporting the exact supplied static `grip` binary as the
`grip` command. Transfer only the bundle and an independently authenticated
publisher policy into a **new** disposable recipient home:

```sh
python3 /opt/gripsack-fixture/smoke_scheduled_jobs.py \
  --grip /opt/gripsack-fixture/grip \
  --package-bundle "$HOME/incoming-bundle" \
  --package-trust "$HOME/publisher-policy.json"
```

This variant imports through `source.package`, removes the transferred bundle
before use, and executes the same real-manager lifecycle. It checks that local
revocation blocks new acquisition, task and timer invocations without deleting
retained bytes or killing an already admitted run. The bundle must be beneath
that fresh fixture home because its removal is deliberate; the source producer,
store and cache must not be mounted into the recipient.


A failed run leaves its private account evidence for inspection. Tear down the
whole disposable VM/account only after collecting that evidence; never run its
cleanup against the developer's account. This smoke does not qualify sleep,
reboot, logout, DST or arbitrary supervisor-death scenarios by implication.

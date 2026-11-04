# Direct thread-to-process lookup

Date: 2026-11-04. Status: implemented.

Each image-runtime thread stores its owning generational ProcessId. Creation
sets it after canonical process membership attaches and before the thread can
run. Lookup validates both the TaskId and the stored ProcessId through their
respective tables. Failed cleanup retains the link and membership; successful
reaping clears the link after detachment.

This removes a nested process/member scan from ordinary system-call resource
lookup. Final-member counting belongs to reaping, where resource retirement
needs it, rather than every mutable resource view. Canonical membership still
owns process lifecycle accounting; this link does not create another owner.

Boot checks compare links with canonical membership after growth and rollback,
exercise cleanup retry, and reject stale task and process generations after
slot reuse while preserving an unrelated live process. Debug four-CPU and
release Haswell-boundary QEMU runs pass with the existing ABI and storage
checks. This is correctness and code-path evidence, not a measured latency
improvement. Scheduler scans and BSP policy serialization remain.

See the [validation record](../validation/2026-11-04-process-links/README.md).

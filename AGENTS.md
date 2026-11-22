# ferrion-hardware

FerrionOS's hardware support: per-machine profiles under `profiles/` and the Linux driver host under `driver-host/` (ADR 0046 in the FerrionOS repository). Before working here, read FerrionOS's `AGENTS.md` and its ADR 0046; every rule there applies here: no comments in FerrionOS's own code (license notices and the source notice of brought-in code stay), TigerStyle bounds (ADR 0024), no agent-written unit tests (ADR 0025: proof is a VM check in FerrionOS's `scripts/vm-test.py`, a differential test or a deterministic simulation), measured targets in commit messages (ADR 0026), the naming rule, and the security rules.

Rules of this repository:

- `driver-host/`: Linux's files are byte-exact copies of the pinned 6.18.y release, under `driver-host/linux/` with their paths. A changed line is a patch file with its reason, never an edit in place. FerrionOS's own code (shim, front end, back ends) is MIT and carries no GPL file's code; code taken from FreeBSD's LinuxKPI or OpenBSD keeps its notice and names its source and commit. An interface the shim does not provide ends the host with its name; nothing missing answers success.
- `profiles/<vendor>/<model>/`: one machine per folder. A "works" entry in a profile's README cites the check and the run that showed it; anything not shown says so. A profile names a machine, so a machine's owner decides whether its profile is published: the maintainer's own machines have no public profile until the maintainer says so, and no profile, README or commit here names them.
- Commits: author and committer `CallMeOuta`, no attribution lines, "Category: Description" subjects of at most 72 columns, the numbers before and after in the body.

# Contributing to ferrion-hardware

Every contribution to this repository is offered under the GNU General Public License, version 2 only (`LICENSE`). A contributed commit carries its author's own name and e-mail; the maintainer merges and pushes. The pin in [FerrionOS](https://github.com/CallMeOuta/FerrionOS) moves only to a commit whose drivers pass FerrionOS's checks.

## Rules

- Every new file under `drivers/` starts with `SPDX-License-Identifier: GPL-2.0-only` in the file's comment syntax. Run `python3 scripts/license-check.py check origin/main` before opening a pull request.
- FerrionOS's own code carries no comments beyond license notices, source notices of brought-in code, and tool directives.
- Follow TigerStyle bounds (FerrionOS ADR 0024): every loop, queue, buffer and retry has a named upper bound with a defined error at it, and invariants are asserted where relied on.
- Agents write no unit tests (FerrionOS ADR 0025): prove changes with VM checks in FerrionOS's `scripts/vm-test.py`, a differential test or a deterministic simulation, red before and green after.
- Commit messages use `Category: Description` subjects of at most 72 columns, no attribution or `Signed-off-by` trailers, and state the checks and measured numbers before and after in the body (FerrionOS ADR 0026).

## Type-check a driver alone

With the nightly toolchain in `drivers/rust-toolchain.toml` (and `rust-src` installed):

```sh
cd drivers
cargo check -Zbuild-std=std,panic_abort --target x86_64-unknown-redox -p nvmed
```

Cargo fetches the seven FerrionOS crates at the revision pinned in `drivers/Cargo.toml`. This type-checks only; it does not link or prove runtime behavior.

## Build and boot in FerrionOS

```sh
git clone https://github.com/CallMeOuta/FerrionOS.git && cd FerrionOS
make f.drivers && make lc.drivers
git -C recipes/core/drivers/source remote add mine <your fork>
git -C recipes/core/drivers/source switch -c my-change
CI=1 COOKBOOK_LOGS=true make r.drivers,initfs && make image
python3 scripts/vm-test.py boot shells --config desktop --timeout 120
```

## Checks by driver area

Run from a FerrionOS checkout and paste the results in the pull request and commit body:

| Change to | Checks in `scripts/vm-test.py` |
|---|---|
| Any driver, `pcid`, `pcid-spawner`, `hwd`, `acpid`, shared libraries | `boot`, `shells`, `lockdown` |
| `nvmed` and other storage drivers | `nvme-queue` (with `--config firefox`), `disk-keys`, `disk-install` |
| `vesad`, `virtio-gpud`, `driver-graphics`, `fbcond`, `fbbootlogd` | `boot` (default and `--make gpu=virtio`), `graphics-bounds`, `gpu-heads` |
| `e1000d`, `virtio-netd`, `driver-network` | `net-basics`, `net-hardening`, `net-perf` |
| `ihdad` and other sound drivers | `audio-ring`, `audio-baseline` |
| Hardware with no VM device | A profile's works entry citing its run (`python3 scripts/profile.py check`) |

<!-- SPDX-License-Identifier: GPL-2.0-only -->

# Hardware profiles

Format and bounds: `python3 scripts/profile.py schema`. Check: `python3 scripts/profile.py check --drivers DIR --checks FILE`. Driver start files are handwritten and checked against the device table.

Generated catalog: [index.toml](index.toml). Regenerate with `python3 scripts/profile.py index` and `python3 scripts/profile.py readme`.

| Profile | Kind | Architectures |
|---|---|---|
| [QEMU q35](qemu/q35/README.md) | vm | x86_64 |
| [QEMU q35 with virtio-gpu and Venus](qemu/q35-venus/README.md) | vm | x86_64 |
| [QEMU q35 with virtio devices](qemu/q35-virtio/README.md) | vm | x86_64 |

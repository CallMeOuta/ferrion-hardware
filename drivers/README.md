<!-- SPDX-License-Identifier: GPL-2.0-only AND MIT -->
# Drivers

FerrionOS's own hardware and virtual device drivers (`drivers/` in `ferrion-hardware`), licensed under GNU General Public License version 2 only ([`../LICENSE`](../LICENSE)). Drivers derived from the upstream project retain their MIT notice ([`LICENSE-MIT`](LICENSE-MIT), `GPL-2.0-only AND MIT`) at the revision recorded in [`upstream.toml`](upstream.toml).

- [Libraries](#libraries)
- [Services](#services)
- [Hardware interfaces](#hardware-interfaces)
- [Devices](#devices)
  - [CPU](#cpu)
  - [Controllers](#controllers)
  - [Storage](#storage)
  - [Graphics](#graphics)
  - [Input](#input)
  - [Sound](#sound)
  - [Networking](#networking)
  - [Virtualization](#virtualization)
- [System interfaces](#system-interfaces)
- [Driver design and licensing](#driver-design-and-licensing)
- [Building and checking](#building-and-checking)

## Libraries

- `amlserde` - Serialization and deserialization of the ACPI AML symbol table
- `common` - Shared driver helpers (DMA, logging, scatter-gather lists, timeouts)
- `executor` - Asynchronous executor for interrupt and queue-based hardware interfaces
- [`graphics/console-draw`](graphics/console-draw/) - Shared terminal drawing library
- [`graphics/driver-graphics`](graphics/driver-graphics/) - Shared display and GPU graphics driver library
- [`net/driver-network`](net/driver-network/) - Shared networking driver library
- [`storage/driver-block`](storage/driver-block/) - Shared block storage driver library
- [`storage/partitionlib`](storage/partitionlib/) - MBR and GPT partition table library
- `virtio-core` - Shared VirtIO transport and queue library

## Services

- [`graphics/drawbench`](graphics/drawbench/) - Graphics drawing benchmark utility
- [`graphics/fbbootlogd`](graphics/fbbootlogd/) - Boot log drawing daemon
- [`graphics/fbcond`](graphics/fbcond/) - Framebuffer console daemon
- `hwd` - Hardware discovery daemon for ACPI and DeviceTree
- `pcid-spawner` - PCI device driver spawner daemon
- [`storage/lived`](storage/lived/) - Live disk daemon
- [`storage/nbdd`](storage/nbdd/) - Network/live block daemon
- `redoxerd` - Daemon exchanging terminal I/O between host and QEMU for `redoxer`

## Hardware interfaces

- `acpid` - ACPI interface driver
- `pcid` - PCI and PCI Express bus driver

## Devices

### CPU

- `rtcd` - x86 Real Time Clock driver

### Controllers

- [`usb/xhcid`](usb/xhcid/) - xHCI USB controller driver

### Storage

- [`storage/ahcid`](storage/ahcid/) - AHCI (SATA) driver
- [`storage/bcm2835-sdhcid`](storage/bcm2835-sdhcid/) - BCM2835 storage controller driver
- [`storage/ided`](storage/ided/) - PATA (IDE) driver
- [`storage/nvmed`](storage/nvmed/) - NVMe driver
- [`storage/usbscsid`](storage/usbscsid/) - USB SCSI mass-storage driver
- [`storage/virtio-blkd`](storage/virtio-blkd/) - VirtIO block device driver
- [`storage/virtio-fsd`](storage/virtio-fsd/) - VirtIO shared filesystem driver

### Graphics

- [`graphics/ihdgd`](graphics/ihdgd/) - Intel graphics driver
- [`graphics/vesad`](graphics/vesad/) - VESA framebuffer driver
- [`graphics/virtio-gpud`](graphics/virtio-gpud/) - VirtIO-GPU driver

### Input

- [`input/ps2d`](input/ps2d/) - PS/2 keyboard and mouse driver
- [`input/usbhidd`](input/usbhidd/) - USB HID driver
- [`usb/usbhubd`](usb/usbhubd/) - USB hub driver
- [`usb/usbctl`](usb/usbctl/) - USB control utility

### Sound

- [`audio/ac97d`](audio/ac97d/) - AC'97 audio driver
- [`audio/ihdad`](audio/ihdad/) - Intel HD Audio driver
- [`audio/sb16d`](audio/sb16d/) - Sound Blaster 16 audio driver

### Networking

- [`net/e1000d`](net/e1000d/) - Intel Gigabit Ethernet driver
- [`net/rtl8139d`](net/rtl8139d/) and [`net/rtl8168d`](net/rtl8168d/) - Realtek Ethernet drivers
- [`net/virtio-netd`](net/virtio-netd/) - VirtIO network device driver

### Virtualization

- `vboxd` - VirtualBox guest driver

## System interfaces

### System calls

- `iopl` - Sets the x86 I/O privilege level. The FerrionOS kernel runs in ring 0 and userspace drivers run in ring 3; changing IOPL requires root privileges and applies per process. Processes with `IOPL=3` can access x86 I/O ports directly.

### Schemes

- `/scheme/memory/physical` - Maps physical memory frames into driver-accessible virtual pages with selectable caching attributes:
  - `/scheme/memory/physical` - Default memory type (writeback)
  - `/scheme/memory/physical@wb` - Writeback cached memory
  - `/scheme/memory/physical@uc` - Uncacheable memory
  - `/scheme/memory/physical@wc` - Write-combining memory
- `/scheme/irq` - Delivers hardware interrupt events, typically monitored via `/scheme/event`.

## Driver design and licensing

A device driver on FerrionOS is a userspace daemon that accesses hardware through schemes and system calls rather than in-kernel APIs.

New drivers and modifications in `drivers/` are licensed under `GPL-2.0-only` (every new file starts with `SPDX-License-Identifier: GPL-2.0-only`, verified by `python3 scripts/license-check.py check origin/main`). Prefer public hardware datasheets; be mindful of vendor Non-Disclosure Agreements that restrict open-source distribution. When referring to existing open-source drivers, keep required source and license notices:

- [FreeBSD drivers](https://github.com/freebsd/freebsd-src/tree/main/sys/dev)
- [NetBSD drivers](https://github.com/NetBSD/src/tree/trunk/sys/dev)
- [OpenBSD drivers](https://github.com/openbsd/src/tree/master/sys/dev)

Drivers typically use `redox-scheme` and `redox_event` alongside the shared libraries in this workspace (`common`, `executor`, `driver-block`, `driver-graphics`, `driver-network`, `virtio-core`).

## Building and checking

See [`../CONTRIBUTING.md`](../CONTRIBUTING.md) for standalone type-checking (`cargo check -Zbuild-std=std,panic_abort --target x86_64-unknown-redox -p <driver>`), building and booting inside a FerrionOS checkout (`CI=1 COOKBOOK_LOGS=true make r.drivers,initfs`), and the required VM checks in FerrionOS's `scripts/vm-test.py`.

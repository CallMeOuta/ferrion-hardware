# ferrion-hardware

Every driver of FerrionOS, and its support for the machines it runs on, physical and virtual alike, in three parts.

- **Drivers** (`drivers/`): FerrionOS's own drivers for physical devices and for the virtual devices of hypervisors and clouds: storage, network, graphics, sound, input, USB, PCI and ACPI. Each is a user-space process that offers its device as a FerrionOS scheme. They arrive here from FerrionOS's base system in its monorepo cutover (FerrionOS's ADR 0047), so that drivers have a repository of their own that anyone can contribute to.
- **The driver host** (`driver-host/`): Linux's own drivers, for devices FerrionOS has no driver of its own for, compiled unchanged against a shim that provides the kernel interfaces they call, run as one user-space process per device and offered to programs as a FerrionOS scheme. Programs, Mesa and the compositor talk to it exactly as they talk to FerrionOS's own drivers. The decision and its reasons are FerrionOS's ADR 0046.
- **Profiles** (`profiles/<vendor>/<model>/`): one per machine or virtual machine platform, from a laptop to QEMU/KVM, VirtualBox, VMware, Hyper-V and the cloud instance types FerrionOS servers run on. A profile picks each device's driver (one of FerrionOS's own, or a Linux driver through the driver host), the firmware packages, and the machine's quirks, and its README says what works on that machine, as shown by FerrionOS's checks. The idea comes from nixos-hardware's per-machine profiles.

The rest of the system (kernel, C library, system services, file system) lives in the [FerrionOS repository](https://github.com/CallMeOuta/FerrionOS), which builds this repository through a recipe pinned by commit.

## Status

FerrionOS's own drivers move in with its monorepo cutover. The driver host's first slice is Linux's virtio-gpu driver inside FerrionOS in a virtual machine, compared pixel for pixel with FerrionOS's own `virtio-gpud` and with a Linux guest on the same device; real-hardware drivers follow. [Hardware profiles](profiles/README.md) lists the QEMU profiles and their generated what-works tables; `python3 scripts/profile.py schema` defines their format.

## Licenses

Everything here is licensed under the GNU General Public License, version 2 only (`LICENSE`), the license of Linux's drivers: FerrionOS's own drivers, the driver host and its shim, and the profiles. Anyone may use, change and share them, and whoever ships a changed driver ships its source under the same license, so no one can turn a FerrionOS driver into a closed one.

Code that arrives under another license keeps its notice beside the GPL, as the GPL allows: the drivers FerrionOS took over from upstream carry their MIT notices, Linux's files carry theirs, and code taken from FreeBSD's or OpenBSD's layers carries its BSD notice and names its source. Both license texts are in `LICENSES/`.

## Only open drivers touch hardware

The license keeps FerrionOS's drivers open, but it cannot stop someone writing a closed driver from scratch. So FerrionOS lets a program touch hardware only if it is a driver built from this repository's public source and signed with FerrionOS's release key. A machine's owner can still allow one specific other driver by an explicit choice, which FerrionOS records and shows, the way an owner enrolls a key for Secure Boot. This is decided and not built yet (FerrionOS's ADR 0047).

On real hardware a driver can read and write any memory its device can reach, so FerrionOS also confines every device behind an IOMMU before its driver runs on real hardware.

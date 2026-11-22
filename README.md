# ferrion-hardware

Every driver of FerrionOS, and its support for the machines it runs on, physical and virtual alike, in three parts.

- **Drivers** (`drivers/`): FerrionOS's own drivers for physical devices and for the virtual devices of hypervisors and clouds: storage, network, graphics, sound, input, USB, PCI and ACPI. Each is a user-space process that offers its device as a FerrionOS scheme. They arrive here from FerrionOS's base system in its monorepo cutover (FerrionOS's ADR 0047), so that drivers have a repository of their own that anyone can contribute to.
- **The driver host** (`driver-host/`): Linux's own drivers, for devices FerrionOS has no driver of its own for, compiled unchanged against a shim that provides the kernel interfaces they call, run as one user-space process per device and offered to programs as a FerrionOS scheme. Programs, Mesa and the compositor talk to it exactly as they talk to FerrionOS's own drivers. The decision and its reasons are FerrionOS's ADR 0046.
- **Profiles** (`profiles/<vendor>/<model>/`): one per machine or virtual machine platform, from a laptop to QEMU/KVM, VirtualBox, VMware, Hyper-V and the cloud instance types FerrionOS servers run on. A profile picks each device's driver (one of FerrionOS's own, or a Linux driver through the driver host), the firmware packages, and the machine's quirks, and its README says what works on that machine, as shown by FerrionOS's checks. The idea comes from nixos-hardware's per-machine profiles.

The rest of the system (kernel, C library, system services, file system) lives in the [FerrionOS repository](https://github.com/CallMeOuta/FerrionOS), which builds this repository through a recipe pinned by commit.

## Status

Nothing runs from here yet. FerrionOS's own drivers move in with FerrionOS's monorepo cutover. The driver host's first slice is Linux's virtio-gpu driver inside FerrionOS in a virtual machine, compared pixel for pixel with FerrionOS's own `virtio-gpud` and with a Linux guest on the same device; the AMD GPU driver for real hardware follows, then `nouveau` with Mesa's NVK for NVIDIA. There are no profiles yet.

## Licenses

Each file keeps its own notice. The repository's default license is the MIT license (`LICENSE`); one folder differs.

- `drivers/` holds FerrionOS's own drivers under the MIT license, with the notices they carry from upstream.
- `driver-host/` builds a program that links Linux's GPL-2.0 files, so that program as a whole is licensed under the GNU General Public License, version 2 only (`driver-host/LICENSE`). Linux's files carry their own notices; FerrionOS's own files in it are under the MIT license, as is code taken from the BSD layers with its original notice.
- `profiles/` is FerrionOS's own work under the MIT license.

Both license texts are also in `LICENSES/`.

On real hardware a driver in the host can read and write any memory its device can reach, so FerrionOS runs it only behind an IOMMU that confines the device.

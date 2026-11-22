# ferrion-hardware

FerrionOS's support for the machines it runs on, physical and virtual alike, in two parts.

- **Profiles** (`profiles/<vendor>/<model>/`): one per machine or virtual machine platform, from a laptop to QEMU/KVM, VirtualBox, VMware, Hyper-V and the cloud instance types FerrionOS servers run on. A profile picks each device's driver (one of FerrionOS's own, or a Linux driver through the driver host), the firmware packages, and the machine's quirks, and its README says what works on that machine, as shown by FerrionOS's checks. The idea comes from nixos-hardware's per-machine profiles.
- **The driver host** (`driver-host/`): Linux's own drivers, for physical devices and for the virtual devices of hypervisors and clouds alike, compiled unchanged against a shim that provides the kernel interfaces they call, run as one user-space process per device and offered to programs as a FerrionOS scheme. Programs, Mesa and the compositor talk to it exactly as they talk to FerrionOS's native drivers. The decision and its reasons are FerrionOS's ADR 0046.

FerrionOS's own drivers stay in the [FerrionOS repository](https://github.com/CallMeOuta/FerrionOS) with the rest of the system; FerrionOS builds this repository through a recipe pinned by commit.

## Status

Nothing runs yet. The driver host's first slice is Linux's virtio-gpu driver inside FerrionOS in a virtual machine, compared pixel for pixel with FerrionOS's own `virtio-gpud` and with a Linux guest on the same device. The AMD GPU driver for real hardware follows, then `nouveau` with Mesa's NVK for NVIDIA. There are no profiles yet.

## Licenses

Each file keeps its own notice.

- `driver-host/` builds a program that links Linux's GPL-2.0 files, so that program as a whole is licensed under the GNU General Public License, version 2 only (`LICENSE`). Linux's files carry their own notices; FerrionOS's own files in it are under the MIT license (`LICENSES/MIT.txt`), as is code taken from the BSD layers with its original notice.
- `profiles/` is FerrionOS's own work under the MIT license (`LICENSES/MIT.txt`).

On real hardware a driver in the host can read and write any memory its device can reach, so FerrionOS runs it only behind an IOMMU that confines the device.

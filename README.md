# ferrion-driver-host

FerrionOS's driver host: Linux's own GPU drivers, compiled unchanged against a shim that provides the kernel interfaces they call, run as one user-space process per GPU and offered to programs as a FerrionOS scheme. Programs, Mesa and the compositor talk to it exactly as they talk to FerrionOS's native GPU driver.

The decision and its reasons are FerrionOS's ADR 0046 in the [FerrionOS repository](https://github.com/CallMeOuta/FerrionOS); FerrionOS builds this repository through a recipe pinned by commit.

## Status

Nothing runs yet. The first slice is Linux's virtio-gpu driver inside FerrionOS in a virtual machine, compared pixel for pixel with FerrionOS's own `virtio-gpud` and with a Linux guest on the same device. The AMD GPU driver for real hardware follows, then `nouveau` with Mesa's NVK for NVIDIA.

## What is in it

- Linux's driver files, from one long-term branch (6.18.y), byte for byte as released. A line that has to change is recorded with its reason.
- The shim, the front end that serves the scheme, and the back ends: FerrionOS's own code, written from FreeBSD's LinuxKPI and OpenBSD's DRM layer as references.

## Licenses

The program as a whole is licensed under the GNU General Public License, version 2 only (`LICENSE`), because it links Linux's GPL-2.0 files. Each file keeps its own notice: Linux's files carry theirs, and FerrionOS's own files are under the MIT license (`LICENSES/MIT.txt`), as is code taken from the BSD layers with its original notice.

On real hardware a GPU driver here can read and write any memory its device can reach, so FerrionOS runs it only behind an IOMMU that confines the device.

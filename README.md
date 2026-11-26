# xmip-core-transport-profinet

PROFINET transport: IEC 61158 real-time class 1 over raw Ethernet — DCP identify and set, cyclic frames with a FrameID and the APDU status trailer; a Stream rides in the IO data across as many cycles as it takes. A technology of [xmip-core-transport](https://github.com/IlleNilsson/xmip-core-transport).

A send target is read by `net::Target` in [xmip-core-library-net](https://github.com/IlleNilsson/xmip-core-library-net), the one reading of a URI every technology calls: scheme, authority, path and decoded query. Until 2026-09-28 it was read through the transport capability's `socket::target`, which split it on its first slash and left the query in the path.

## Acknowledgement

Acceptance is at-most-once here. PROFINET's cyclic IO data is process data,
sent again every cycle and acknowledged by nobody, so a run of cycles is off
the wire as it is read and nobody is left to tell how the receive cycle ended.
Each Stream arrives whole, its cycles assembled.

## The declaration

What a Location of this technology is configured with is two `const`s in its `declaration` module, `src/declaration.rs`, beside the code, which `Configured` reads (ADR-0064, amendment 2026-11-26): `declaration::SETTINGS`, `controller`, `device`, `frame_id` and `timeout`, and `SCHEMES`, `profinet`. The first and last real-time class 1 `FrameID`, which `frame_id` is held between, and the wait `timeout` defaults to, `RT_CLASS_1_FIRST`, `RT_CLASS_1_LAST` and `DEFAULT_TIMEOUT`, are there too, and the code reads them from there, where they were `cyclic`'s and the crate root's until then. It is the same on every operating system, those without the kernel's raw socket included. Built with its `declaration` feature alone (`--no-default-features --features declaration`), the crate is those and nothing more: its code and every crate the code is built on — the ethernet technology it rides on among them — are behind the default feature. So the library the surfaces load carries it without the code, through the gathering crate xmip-core-declaration (open problem 48), and the language server and the desktop suggest and check a PROFINET Location's settings though no node in their process loaded the technology. `test/Declaration.Test.ps1` in the estate holds that the declaration builds nothing the transport capability does not.

## Toolchain

`rust-toolchain.toml` pins the toolchain for the whole estate. Do not change it
here.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.

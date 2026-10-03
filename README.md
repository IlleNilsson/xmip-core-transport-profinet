# xmip-core-transport-profinet

PROFINET transport: IEC 61158 real-time class 1 over raw Ethernet — DCP identify and set, cyclic frames with a FrameID and the APDU status trailer; a Stream rides in the IO data across as many cycles as it takes. A technology of [xmip-core-transport](https://github.com/IlleNilsson/xmip-core-transport).

A send target is read by `net::Target` in [xmip-core-library-net](https://github.com/IlleNilsson/xmip-core-library-net), the one reading of a URI every technology calls: scheme, authority, path and decoded query. Until 2026-09-28 it was read through the transport capability's `socket::target`, which split it on its first slash and left the query in the path.

## Acknowledgement

Acceptance is at-most-once here. PROFINET's cyclic IO data is process data,
sent again every cycle and acknowledged by nobody, so a run of cycles is off
the wire as it is read and nobody is left to tell how the receive cycle ended.
Each Stream arrives whole, its cycles assembled.

## Toolchain

`rust-toolchain.toml` pins the toolchain for the whole estate. Do not change it
here.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.

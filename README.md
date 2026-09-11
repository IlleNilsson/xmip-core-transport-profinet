# xmip-core-transport-profinet

PROFINET transport: IEC 61158 real-time class 1 over raw Ethernet — DCP identify and set, cyclic frames with a FrameID and the APDU status trailer; a Stream rides in the IO data across as many cycles as it takes. A technology of [xmip-core-transport](https://github.com/IlleNilsson/xmip-core-transport).

## Toolchain

`rust-toolchain.toml` pins the toolchain for the whole estate. Do not change it
here.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.

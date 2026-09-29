# Third-party Rust dependencies

CFMD vendors its external Rust dependency closure under `vendor/` for reproducible/offline builds. Direct cryptographic dependencies are `aes-gcm-siv` 0.12.1, `hkdf` 0.13.0, `getrandom` 0.4.3, `ed25519-dalek` 3.0.0, `sha2` 0.11.0 and `zeroize` 1.9.0; the remaining entries below are their transitive build/runtime closure.

| crate | version | declared license |
|---|---:|---|
| `aead` | `0.6.1` | `MIT OR Apache-2.0` |
| `aes` | `0.9.3` | `MIT OR Apache-2.0` |
| `aes-gcm-siv` | `0.12.1` | `MIT OR Apache-2.0` |
| `block-buffer` | `0.12.1` | `MIT OR Apache-2.0` |
| `cfg-if` | `1.0.5` | `MIT OR Apache-2.0` |
| `cipher` | `0.5.2` | `MIT OR Apache-2.0` |
| `cmov` | `0.5.4` | `Apache-2.0 OR MIT` |
| `cpubits` | `0.1.1` | `MIT OR Apache-2.0` |
| `cpufeatures` | `0.3.1` | `MIT OR Apache-2.0` |
| `crypto-common` | `0.2.2` | `MIT OR Apache-2.0` |
| `ctr` | `0.10.1` | `MIT OR Apache-2.0` |
| `ctutils` | `0.4.2` | `Apache-2.0 OR MIT` |
| `curve25519-dalek` | `5.0.0` | `BSD-3-Clause` |
| `curve25519-dalek-derive` | `0.1.1` | `MIT/Apache-2.0` |
| `digest` | `0.11.3` | `MIT OR Apache-2.0` |
| `ed25519` | `3.0.0` | `Apache-2.0 OR MIT` |
| `ed25519-dalek` | `3.0.0` | `BSD-3-Clause` |
| `fiat-crypto` | `0.3.0` | `MIT OR Apache-2.0 OR BSD-1-Clause` |
| `getrandom` | `0.4.3` | `MIT OR Apache-2.0` |
| `hkdf` | `0.13.0` | `MIT OR Apache-2.0` |
| `hmac` | `0.13.0` | `MIT OR Apache-2.0` |
| `hybrid-array` | `0.4.15` | `MIT OR Apache-2.0` |
| `inout` | `0.2.2` | `MIT OR Apache-2.0` |
| `libc` | `0.2.189` | `MIT OR Apache-2.0` |
| `polyval` | `0.7.3` | `Apache-2.0 OR MIT` |
| `proc-macro2` | `1.0.107` | `MIT OR Apache-2.0` |
| `quote` | `1.0.47` | `MIT OR Apache-2.0` |
| `r-efi` | `6.0.0` | `MIT OR Apache-2.0 OR LGPL-2.1-or-later` |
| `rustc_version` | `0.4.1` | `MIT OR Apache-2.0` |
| `semver` | `1.0.28` | `MIT OR Apache-2.0` |
| `sha2` | `0.11.0` | `MIT OR Apache-2.0` |
| `signature` | `3.0.0` | `Apache-2.0 OR MIT` |
| `subtle` | `2.6.1` | `BSD-3-Clause` |
| `syn` | `2.0.119` | `MIT OR Apache-2.0` |
| `typenum` | `1.20.1` | `MIT OR Apache-2.0` |
| `unicode-ident` | `1.0.26` | `(MIT OR Apache-2.0) AND Unicode-3.0` |
| `universal-hash` | `0.6.1` | `MIT OR Apache-2.0` |
| `zeroize` | `1.9.0` | `Apache-2.0 OR MIT` |

This table is informational and derived from vendored `Cargo.toml` metadata. Distribution/relicensing decisions should inspect the complete upstream license files shipped in each vendored crate.

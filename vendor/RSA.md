# RSA 0.10.0 compatibility

Itda keeps the current IronRDP protocol stack and patches three of its dependencies.
Picky and SSPI are copied from the crates.io release archives below. winscard 0.3.3
already has a local zlib-rs patch documented in [winscard/ITDA.md](winscard/ITDA.md).

| Archive | SHA-256 |
| --- | --- |
| `picky-7.0.0-rc.25.crate` | `c1ae9cd78eb1d61be4790713d28368cf71c844218fcd91542768805423f02666` |
| `sspi-0.21.3.crate` | `5c3bd2a45e7e24fd72eba100ced3fd847e05fe4657d7f067eb06d1a894f0d4e5` |

- All three manifests select stable RSA 0.10.0. Picky and SSPI also select stable
  PKCS#1 0.8 to match the encoding dependency required by RSA 0.10.0.
- Picky enables the RSA crate's PKCS#1 v1.5 feature. Its private-key conversion
  borrows the RSA prime values to build the existing PKCS#1 representation.

The stable RSA release addresses [RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071.html).
Use the published upstream crates once their dependency constraints select a patched
RSA version. Run the unfiltered audit and verify RDP TLS and NLA on both platforms
when updating these dependencies.

# RSA 0.10.0 compatibility

Itda keeps the current IronRDP protocol stack and patches three of its dependencies.
Picky 7.0.0-rc.25 and SSPI 0.21.3 are from crates.io. winscard 0.3.3 already has
a local zlib-rs patch documented in [winscard/ITDA.md](winscard/ITDA.md).

- All three manifests select stable RSA 0.10.0. Picky and SSPI also select stable
  PKCS#1 0.8 to match the encoding dependency required by RSA 0.10.0.
- Picky enables the RSA crate's PKCS#1 v1.5 feature. Its private-key conversion
  borrows the RSA prime values to build the existing PKCS#1 representation.

The stable RSA release addresses [RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071.html).
Use the published upstream crates once their dependency constraints select a patched
RSA version. Run the unfiltered audit and verify RDP TLS and NLA on both platforms
when updating these dependencies.

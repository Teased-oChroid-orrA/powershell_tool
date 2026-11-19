# Third-party notices

This file ships next to every release executable (`app-tui.exe`), together with the license texts in `licenses/`.

## Always

The Rust crates this program is built from keep their own licenses (MIT, Apache-2.0, BSD and similar); `cargo tree` and
`Cargo.lock` list them. Nothing in the default build is under a copyleft license.

## Native chart triangulation

The native CAD path uses **spade 2.15.1**, licensed MIT OR Apache-2.0
(<https://github.com/Stoeoef/spade>). This distribution elects its MIT license;
`licenses/spade-LICENSE-MIT.txt` retains its copyright and permission notice.
This 2D triangulation dependency is not a CAD kernel.

The native diagnostic STEP association path uses **sha2 0.10.9**
(<https://github.com/RustCrypto/hashes>), MIT OR Apache-2.0. This distribution
elects MIT; `licenses/sha2-LICENSE-MIT.txt` retains the upstream notice.

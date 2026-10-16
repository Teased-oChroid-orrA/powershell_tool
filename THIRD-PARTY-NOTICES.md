# Third-party notices

This file ships next to every release executable (`app-tui.exe`), together with the license texts in `licenses/`.

## Always

The Rust crates this program is built from keep their own licenses (MIT, Apache-2.0, BSD and similar); `cargo tree` and
`Cargo.lock` list them. Nothing in the default build is under a copyleft license.

## Only in a build with the `occt` feature (STEP import and solid editing)

A build made with `--features occt` links **Open CASCADE Technology (OCCT)** 8.0.1, statically, through the `cadrum` crate (MIT,
a binding that ships a prebuilt OCCT). OCCT is licensed under the **GNU Lesser General Public License version 2.1 with the Open
CASCADE exception 1.0**.

This product makes use of or is based on facilities provided by the Open CASCADE Technology software
(<https://dev.opencascade.org>). That sentence is the "prominent notice in supporting documentation" the exception asks for.

Obligations for distributing such a build (checked by the release workflow, which refuses to publish an `occt` build without the
files below):

1. `licenses/OpenCASCADE-LICENSE_LGPL_21.txt` and `licenses/OpenCASCADE-OCCT_LGPL_EXCEPTION.txt` ship with the executable.
2. The corresponding source of the OCCT version linked (OCCT 8.0.1, <https://github.com/Open-Cascade-SAS/OCCT>) and the means to
   relink the program with a modified OCCT are available to recipients: the program's source is in this repository, and
   `cargo build --release -p app-tui --features occt` (with `--features occt-source` for a build of OCCT from source) rebuilds it.
3. This notice stays with the product.

Whether this satisfies the LGPL for the way the program is distributed (static linking, an offline executable) is a legal
question this repository cannot settle: it needs sign-off from whoever distributes the build. See `docs/adr/ADR-019-solid-kernel-opencascade.md`.

## Native chart triangulation

The native diagnostic CAD path uses **spade 2.15.1**, licensed MIT OR Apache-2.0
(<https://github.com/Stoeoef/spade>). This distribution elects its MIT license;
`licenses/spade-LICENSE-MIT.txt` retains its copyright and permission notice.
This 2D triangulation dependency does not replace OpenCASCADE's CAD functionality.

The native diagnostic STEP association path uses **sha2 0.10.9**
(<https://github.com/RustCrypto/hashes>), MIT OR Apache-2.0. This distribution
elects MIT; `licenses/sha2-LICENSE-MIT.txt` retains the upstream notice.

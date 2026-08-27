//! Offline-only asset loading for the app's Blitz/dioxus-native renderer.
//!
//! This app is launched with a hand-rolled `launch::run` (see `main.rs`)
//! instead of `dioxus_native::launch_cfg`, and until the Bushing Workbench
//! tool needed `<img src="data:...">` for its pre-rendered LaTeX
//! derivation formulas (`bushing_workbench.rs`'s `formula_img_src`), that
//! hand-rolled launcher passed `net_provider: None` - Blitz then falls
//! back to `blitz_traits::net::DummyNetProvider`, whose `fetch` is a
//! true no-op (verified by reading `blitz-traits-0.2.0/src/net.rs`
//! directly), so `<img>` never rendered anything at all.
//!
//! `blitz-shell`'s `data-uri` feature ships exactly the right amount of
//! provider for that gap: `DataUriNetProvider` resolves `data:` URIs
//! locally (base64 decode, no I/O) and returns an explicit
//! `"UnsupportedScheme"` error for every other scheme - not
//! `blitz-net::Provider`, which would also pull in a real HTTP client
//! (`reqwest`) this app has no legitimate use for and does not want
//! silently available to any future stray `href`/`src`.
//!
//! Self-contained on purpose - to remove data-URI image support entirely:
//! delete this file, drop `mod net_provider;` and the `net_provider(...)`
//! call in `main.rs`'s `launch::run`, and remove blitz-shell's `data-uri`
//! feature from `app/Cargo.toml`.

use std::sync::Arc;

use blitz_dom::net::Resource;
use blitz_shell::{BlitzShellEvent, BlitzShellNetCallback, DataUriNetProvider};
use blitz_traits::net::NetProvider;
use winit::event_loop::EventLoopProxy;

/// Builds the `NetProvider` passed into `DocumentConfig::net_provider`.
/// `proxy` is the same event loop proxy `launch::run` already creates for
/// `DioxusNativeApplication` - `EventLoopProxy` is cheap to clone, so
/// callers just pass a second clone/`create_proxy()` call here.
pub fn data_uri_only(proxy: EventLoopProxy<BlitzShellEvent>) -> Arc<dyn NetProvider<Resource>> {
    DataUriNetProvider::shared(BlitzShellNetCallback::shared(proxy))
}

//! Design System Epic, Phase 1: foundation modules (typography, spacing,
//! radii, shadows) + locally-bundled fonts. Adapted from the user-supplied
//! `NATIVE_PREMIUM_UI_SYSTEM_EPIC.md` (`UI-EPIC-001`) and scoped down to
//! this specific 3-tool app per the approved planning artifact - see that
//! artifact's "Design system" tier for what's deferred and why.
//!
//! Color tokens are NOT duplicated here - `crate::theme::Tokens` already
//! owns that (ported from the approved mockup's CSS custom properties
//! before this epic existed) and every call site already depends on it.
//! Re-deriving a second color-token module would create two sources of
//! truth for the same palette; this module covers only what didn't exist
//! yet: type scale, spacing scale, radius scale, and elevation.

pub mod components;
pub mod icons;
pub mod radii;
pub mod shadows;
pub mod spacing;
pub mod typography;

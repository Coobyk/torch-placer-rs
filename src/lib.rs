//! Torch Placer's own crate. The client mixins under `src/mixins/` are
//! compiled into copies of the game functions they patch; the shared state
//! module is injected by `[native.module]`.
#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl mod_api::Hooks for Mod {}

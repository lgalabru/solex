//! A tiny "program" exercising every solex macro. The same source is compiled
//! once per channel; assertions key off `solex::active_channel!()`.

// Bare `#[channel(...)]` works anywhere once imported; a `feature` spelling
// would collide with rustc's built-in `#![feature]` attribute.
use solex::{ProgramError, channel};

solex::declare_id!();

pub const CHANNEL: &str = solex::active_channel!();
pub const HAS_STABLE: bool = solex::ships!(stable);
pub const HAS_NIGHTLY: bool = solex::ships!(nightly);
pub const HAS_CANARY: bool = solex::ships!("canary");

/// Ships everywhere (every channel includes `stable`).
#[channel(stable)]
pub fn stable_only() -> Result<u8, ProgramError> {
    Ok(1)
}

/// Present on nightly and canary only. Inherent items shadow trait items, so
/// `Probe::NIGHTLY` reads the inherent const when the gated impl is compiled in
/// and falls back to the trait default when solex removed it.
pub struct Probe;
pub trait ProbeFallback {
    const NIGHTLY: bool = false;
}
impl ProbeFallback for Probe {}
#[channel(nightly)]
impl Probe {
    pub const NIGHTLY: bool = true;
}
pub const NIGHTLY_ITEM_PRESENT: bool = Probe::NIGHTLY;

/// Gated at runtime: fails until epoch 850 on nightly, unconditional on canary.
#[channel([nightly, 850], canary)]
pub fn epoch_gated() -> Result<u8, ProgramError> {
    Ok(2)
}

/// Methods inside impl blocks are gated too.
pub struct Counter(pub u64);

impl Counter {
    #[channel(stable)]
    pub fn bump(&mut self) -> Result<(), ProgramError> {
        self.0 += 1;
        Ok(())
    }

    #[channel([stable, 1_000_000])]
    pub fn far_future(&mut self) -> Result<(), ProgramError> {
        self.0 = u64::MAX;
        Ok(())
    }
}

/// Non-Anchor `#[solex::program]`: features are applied to module items and the
/// module is emitted as-is.
#[solex::program]
pub mod program {
    use super::*;

    #[channel(stable)]
    pub fn ship_everywhere() -> Result<u8, ProgramError> {
        Ok(10)
    }

    #[channel(canary)]
    pub fn canary_only() -> Result<u8, ProgramError> {
        Ok(30)
    }

    #[channel(nightly)]
    pub fn nightly_only() -> Result<u8, ProgramError> {
        Ok(20)
    }
}

/// Per-channel constant from the manifest.
pub const TREASURY: &str = solex::var!("treasury");

/// Hand-rolled instruction enum in the style of Pinocchio programs: variants
/// cannot carry attribute macros, so `#[solex::gated]` resolves the markers.
#[solex::gated]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Instruction {
    Open = 1,
    #[channel(nightly)]
    OpenWithHooks = 10,
}

#[solex::gated]
impl Instruction {
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            1 => Some(Self::Open),
            #[channel(nightly)]
            10 => Some(Self::OpenWithHooks),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Open => "open",
            #[channel(nightly)]
            Self::OpenWithHooks => "open_with_hooks",
        }
    }
}

/// Statements and nested items inside a gated fn body.
#[solex::gated]
#[allow(unused_mut)] // `n` is only mutated on channels shipping `nightly`
pub fn dispatch_count() -> u8 {
    let mut n = 1u8;
    #[channel(nightly)]
    let nightly_extra = 1u8;
    #[channel(nightly)]
    {
        n += nightly_extra;
    }
    n
}

/// A named feature: the string labels the item, the channel says where.
#[channel(nightly, "hooks")]
pub fn hooks_feature() -> u8 {
    7
}

/// Probe usable on every channel: `Some` only where the feature compiled in.
#[solex::gated]
pub fn hooks_feature_value() -> Option<u8> {
    #[allow(unused_mut)]
    let mut v = None;
    #[channel(nightly, "hooks")]
    {
        v = Some(hooks_feature());
    }
    v
}

//! # solex — SOlana EXpress
//!
//! Ship one codebase to several release channels. Each channel in
//! `Solex.yaml` has its own program id and a set of feature tags; items marked
//! `#[channel(tag)]` compile only into channels that ship that tag, and
//! `#[channel([tag, epoch])]` additionally refuses to run before the cluster
//! reaches `epoch`.
//!
//! ```ignore
//! use anchor_lang::prelude::*;
//!
//! solex::declare_id!();              // id of the channel being built
//!
//! #[solex::program]
//! pub mod counter {
//!     use super::*;
//!
//!     #[channel(stable)]
//!     pub fn increment(ctx: Context<Increment>) -> Result<()> { ... }
//!
//!     #[channel([nightly, 850])]
//!     pub fn decrement(ctx: Context<Decrement>) -> Result<()> { ... }
//! }
//! ```

#![cfg_attr(not(any(feature = "anchor", feature = "native")), allow(unused))]

#[cfg(not(any(feature = "anchor", feature = "native")))]
compile_error!("solex: enable the `anchor` (default) or `native` feature");

pub use solex_macros::{active_channel, channel, declare_id, program, ships};

#[cfg(feature = "anchor")]
pub use anchor_lang;

#[cfg(feature = "anchor")]
mod sys {
    pub use anchor_lang::{
        prelude::SolanaSysvar as Sysvar,
        solana_program::{clock::Clock, msg, program_error::ProgramError, pubkey::Pubkey},
    };
}

#[cfg(all(feature = "native", not(feature = "anchor")))]
mod sys {
    pub use solana_clock::Clock;
    pub use solana_msg::msg;
    pub use solana_program_error::ProgramError;
    pub use solana_pubkey::Pubkey;
    pub use solana_sysvar::Sysvar;
}

#[cfg(any(feature = "anchor", feature = "native"))]
pub use sys::{ProgramError, Pubkey};

/// Error codes returned by Solex runtime gates, as `ProgramError::Custom`.
pub mod error {
    /// Base of the Solex error range (`"SX"` in ASCII), far above Anchor's
    /// 6000+ user range so codes never collide.
    pub const BASE: u32 = 0x5358_0000;
    /// The instruction exists on this channel but its activation epoch has
    /// not been reached yet.
    pub const FEATURE_NOT_ACTIVE: u32 = BASE;
}

#[cfg(any(feature = "anchor", feature = "native"))]
mod runtime {
    use super::{
        error,
        sys::{Clock, ProgramError, Sysvar, msg},
    };

    /// Current cluster epoch from the Clock sysvar.
    pub fn current_epoch() -> Result<u64, ProgramError> {
        Ok(Clock::get()?.epoch)
    }

    /// Whether the cluster has reached `epoch`.
    pub fn epoch_reached(epoch: u64) -> Result<bool, ProgramError> {
        Ok(current_epoch()? >= epoch)
    }

    /// Fail with [`error::FEATURE_NOT_ACTIVE`] until `epoch` is reached. This
    /// is what `#[channel([tag, epoch])]` injects at the top of a handler.
    pub fn require_epoch(epoch: u64) -> Result<(), ProgramError> {
        let current = current_epoch()?;
        if current < epoch {
            msg!(
                "solex: feature inactive until epoch {} (now {})",
                epoch,
                current
            );
            return Err(ProgramError::Custom(error::FEATURE_NOT_ACTIVE));
        }
        Ok(())
    }
}

#[cfg(any(feature = "anchor", feature = "native"))]
pub use runtime::{current_epoch, epoch_reached};

/// Used by generated code; not part of the public API.
#[doc(hidden)]
#[cfg(any(feature = "anchor", feature = "native"))]
pub mod __private {
    pub use super::runtime::require_epoch;

    // `#[solex::program]` and `solex::declare_id!` hand off to these after the
    // channel logic ran; the cargo features of *this* crate pick the framework.
    #[cfg(feature = "anchor")]
    pub use anchor_lang::{declare_id as framework_declare_id, program as framework_program};
    #[cfg(all(feature = "native", not(feature = "anchor")))]
    pub use solex_macros::{
        native_declare_id as framework_declare_id, passthrough_program as framework_program,
    };
}

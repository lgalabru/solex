//! # solex — SOlana EXpress
//!
//! Ship one codebase to several release channels. Each channel in the
//! manifest has its own program id and a set of tags; items marked
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
//!
//! Frameworks are selected by cargo feature: `anchor` (default), `pinocchio`
//! or `native`. Without `anchor` the crate is `no_std`.

#![cfg_attr(not(feature = "anchor"), no_std)]

#[cfg(not(any(feature = "anchor", feature = "pinocchio", feature = "native")))]
compile_error!("solex: enable one of the `anchor` (default), `pinocchio` or `native` features");

pub use solex_macros::{
    activation, active_channel, active_cluster, channel, declare_id, gated, program, ships, var,
};

#[cfg(feature = "anchor")]
pub use anchor_lang;

#[cfg(feature = "anchor")]
mod sys {
    pub use anchor_lang::{
        prelude::SolanaSysvar as Sysvar,
        solana_program::{clock::Clock, program_error::ProgramError, pubkey::Pubkey},
    };
    pub fn log_inactive(epoch: u64, current: u64) {
        anchor_lang::solana_program::msg!(
            "solex: feature inactive until epoch {} (now {})",
            epoch,
            current
        );
    }
}

#[cfg(all(feature = "pinocchio", not(feature = "anchor")))]
mod sys {
    pub use pinocchio::{
        Address as Pubkey,
        error::ProgramError,
        sysvars::{Sysvar, clock::Clock},
    };
    pub fn log_inactive(_epoch: u64, _current: u64) {
        // pinocchio core ships no formatting logger; the custom error code
        // identifies the gate, and `pinocchio-log` users can wrap `epoch_reached`.
    }
}

#[cfg(all(
    feature = "native",
    not(feature = "anchor"),
    not(feature = "pinocchio")
))]
mod sys {
    pub use solana_clock::Clock;
    pub use solana_program_error::ProgramError;
    pub use solana_pubkey::Pubkey;
    pub use solana_sysvar::Sysvar;
    pub fn log_inactive(epoch: u64, current: u64) {
        solana_msg::msg!(
            "solex: feature inactive until epoch {} (now {})",
            epoch,
            current
        );
    }
}

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

mod runtime {
    use super::{
        error,
        sys::{Clock, ProgramError, Sysvar, log_inactive},
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
            log_inactive(epoch, current);
            return Err(ProgramError::Custom(error::FEATURE_NOT_ACTIVE));
        }
        Ok(())
    }
}

pub use runtime::{current_epoch, epoch_reached};

/// Used by generated code; not part of the public API.
#[doc(hidden)]
pub mod __private {
    pub use super::runtime::require_epoch;

    // `#[solex::program]` and `solex::declare_id!` hand off to these after the
    // channel logic ran; the cargo features of *this* crate pick the framework.
    #[cfg(feature = "anchor")]
    pub use anchor_lang::{declare_id as framework_declare_id, program as framework_program};
    #[cfg(all(feature = "pinocchio", not(feature = "anchor")))]
    pub use solana_address::declare_id as framework_declare_id;
    #[cfg(all(feature = "pinocchio", not(feature = "anchor")))]
    pub use solex_macros::passthrough_program as framework_program;
    #[cfg(all(
        feature = "native",
        not(feature = "anchor"),
        not(feature = "pinocchio")
    ))]
    pub use solex_macros::{
        native_declare_id as framework_declare_id, passthrough_program as framework_program,
    };
}

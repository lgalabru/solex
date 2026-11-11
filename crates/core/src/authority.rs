//! Who signs an upgrade. `upgrade_authority` in the manifest is a keypair
//! file or a bare address; the chain says who the authority of a deployed
//! program really is. Together they decide whether this machine can sign or
//! the signature has to come from somewhere else.

use std::path::{Path, PathBuf};

use crate::{
    CoreError, Project, Result, keys, program_metadata,
    solana::{self, Pubkey, Rpc},
};

/// `upgrade_authority` as written in the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Declared {
    /// A keypair file this machine signs with.
    Keypair(PathBuf),
    /// An address whose key is held elsewhere. Pins the authority: a deployed
    /// program with another one is refused.
    Address(String),
}

/// Read a manifest value: an existing file is a keypair, as is anything that
/// does not parse as an address.
pub fn declared(project: &Project, raw: &Path) -> Declared {
    let path = project.resolve_path(raw);
    let address = raw
        .to_str()
        .filter(|s| keys::validate_pubkey(s).is_ok() && !path.is_file());
    match address {
        Some(a) => Declared::Address(a.to_string()),
        None => Declared::Keypair(path),
    }
}

/// The keypair file a manifest value names, if it names one.
pub fn declared_keypair(project: &Project, raw: &Path) -> Option<PathBuf> {
    match declared(project, raw) {
        Declared::Keypair(p) => Some(p),
        Declared::Address(_) => None,
    }
}

/// What the cluster holds for a program id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnChain {
    NotDeployed,
    /// Deployed with no upgrade authority; it can never change.
    Immutable,
    Upgradeable(Pubkey),
}

pub fn on_chain(rpc: &Rpc, program: &Pubkey) -> Result<OnChain> {
    if rpc.account(program)?.is_none() {
        return Ok(OnChain::NotDeployed);
    }
    Ok(match program_metadata::upgrade_authority(rpc, program)? {
        Some(a) => OnChain::Upgradeable(a),
        None => OnChain::Immutable,
    })
}

/// What kind of address an authority without a local keypair is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// On the ed25519 curve: someone holds its key and signs the upgrade
    /// transaction itself.
    Key,
    /// Off the curve: a program-derived address. `owner` is the program
    /// owning its account, when that account exists and says more than the
    /// System Program does.
    Derived { owner: Option<String> },
}

pub fn is_on_curve(address: &Pubkey) -> bool {
    curve25519_dalek::edwards::CompressedEdwardsY(*address)
        .decompress()
        .is_some()
}

pub fn kind(rpc: &Rpc, address: &Pubkey) -> Result<Kind> {
    if is_on_curve(address) {
        return Ok(Kind::Key);
    }
    let owner = rpc
        .account(address)?
        .map(|a| a.owner)
        .filter(|o| *o != solana::SYSTEM_PROGRAM)
        .map(|o| solana::b58(&o));
    Ok(Kind::Derived { owner })
}

/// Who signs the upgrade of one program on one cluster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signer {
    /// This keypair file; `address` is its public key.
    Local { path: PathBuf, address: String },
    /// No keypair here for `address`. `deployed` is false when the program
    /// does not exist yet and `address` is only what the manifest declares.
    Remote {
        address: String,
        kind: Kind,
        deployed: bool,
    },
}

impl Signer {
    pub fn address(&self) -> &str {
        match self {
            Signer::Local { address, .. } | Signer::Remote { address, .. } => address,
        }
    }

    /// One line for a human: what the address is and how it signs.
    pub fn describe(&self) -> String {
        match self {
            Signer::Local { path, address } => format!("{address} ({})", path.display()),
            Signer::Remote {
                address,
                kind: Kind::Key,
                ..
            } => format!("{address} (no local keypair; signs the transaction offline)"),
            Signer::Remote {
                address,
                kind: Kind::Derived { owner: Some(owner) },
                ..
            } => format!("{address} (program-derived, owned by {owner})"),
            Signer::Remote {
                address,
                kind: Kind::Derived { owner: None },
                ..
            } => format!("{address} (program-derived)"),
        }
    }
}

/// Decide the signer from what is declared, the keypair the Solana CLI would
/// use (`local`), and the chain.
///
/// The chain wins for a deployed program; a declaration that disagrees with
/// it is an error rather than something to follow or override.
pub fn resolve(
    rpc: &Rpc,
    program_id: &str,
    declared: Option<&Declared>,
    local: Option<&Path>,
) -> Result<Signer> {
    let program = solana::pubkey(program_id)?;
    let chain = on_chain(rpc, &program)?;
    let local = match (declared, local) {
        (Some(Declared::Keypair(p)), _) => Some(p.as_path()),
        (Some(Declared::Address(_)), _) => None,
        (None, l) => l,
    };
    let local = local
        .filter(|p| p.is_file())
        .map(|p| Ok::<_, CoreError>((p.to_path_buf(), keys::pubkey_of(p)?)))
        .transpose()?;
    decide(program_id, declared, local, chain, |a| kind(rpc, a))
}

fn decide(
    program_id: &str,
    declared: Option<&Declared>,
    local: Option<(PathBuf, String)>,
    chain: OnChain,
    kind: impl Fn(&Pubkey) -> Result<Kind>,
) -> Result<Signer> {
    let remote = |address: String, deployed: bool| -> Result<Signer> {
        let kind = kind(&solana::pubkey(&address)?)?;
        Ok(Signer::Remote {
            address,
            kind,
            deployed,
        })
    };
    match chain {
        OnChain::Immutable => Err(CoreError::message(format!(
            "{program_id} is immutable (no upgrade authority); it cannot be upgraded"
        ))),
        OnChain::NotDeployed => match (declared, local) {
            (Some(Declared::Address(a)), _) => remote(a.clone(), false),
            (_, Some((path, address))) => Ok(Signer::Local { path, address }),
            (Some(Declared::Keypair(p)), None) => Err(CoreError::message(format!(
                "upgrade authority keypair {} does not exist",
                p.display()
            ))),
            (None, None) => Err(CoreError::message(
                "no upgrade authority: set `upgrade_authority` in the manifest or pass \
                 --upgrade-authority",
            )),
        },
        OnChain::Upgradeable(actual) => {
            let actual = solana::b58(&actual);
            match (declared, local) {
                (Some(Declared::Address(a)), _) if *a != actual => {
                    Err(CoreError::message(format!(
                        "the manifest declares upgrade authority {a}, but {program_id} is \
                         controlled by {actual}\n  Nothing was sent. Fix `upgrade_authority` if \
                         the authority was handed over on purpose."
                    )))
                }
                (Some(Declared::Address(_)), _) => remote(actual, true),
                (_, Some((path, address))) if address == actual => {
                    Ok(Signer::Local { path, address })
                }
                // A declared keypair that is not the authority is a mistake;
                // an undeclared one just means the key lives elsewhere.
                (Some(Declared::Keypair(_)), Some((path, address))) => {
                    Err(CoreError::message(format!(
                        "{} is the keypair of {address}, but {program_id} is controlled by \
                         {actual}\n  Nothing was sent. Point `upgrade_authority` at the right \
                         keypair, or at the address `{actual}` if its key is held elsewhere.",
                        path.display()
                    )))
                }
                (Some(Declared::Keypair(p)), None) => Err(CoreError::message(format!(
                    "upgrade authority keypair {} does not exist\n  {program_id} is controlled \
                     by {actual}.",
                    p.display()
                ))),
                (None, _) => remote(actual, true),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::Keypair;

    const PROGRAM: &str = "CHNLxBtXw7VPqd5GMcbtVoEbn4ehyaSoiHj56Ao5sWkJ";

    fn key() -> (PathBuf, String, Pubkey) {
        let kp = Keypair::generate();
        (PathBuf::from("auth.json"), kp.pubkey(), kp.pubkey_bytes())
    }

    fn run(
        declared: Option<&Declared>,
        local: Option<(PathBuf, String)>,
        chain: OnChain,
    ) -> Result<Signer> {
        decide(PROGRAM, declared, local, chain, |a| {
            Ok(if is_on_curve(a) {
                Kind::Key
            } else {
                Kind::Derived { owner: None }
            })
        })
    }

    #[test]
    fn a_matching_local_keypair_signs() {
        let (path, address, bytes) = key();
        let local = Some((path.clone(), address.clone()));
        let want = Signer::Local { path, address };
        assert_eq!(
            run(None, local.clone(), OnChain::Upgradeable(bytes)).unwrap(),
            want
        );
        assert_eq!(run(None, local, OnChain::NotDeployed).unwrap(), want);
    }

    #[test]
    fn the_chain_is_followed_when_nothing_is_declared() {
        let (path, address, _) = key();
        let (_, other, other_bytes) = key();
        let got = run(
            None,
            Some((path, address)),
            OnChain::Upgradeable(other_bytes),
        )
        .unwrap();
        assert_eq!(
            got,
            Signer::Remote {
                address: other,
                kind: Kind::Key,
                deployed: true
            }
        );
    }

    #[test]
    fn a_declared_keypair_that_is_not_the_authority_is_refused() {
        let (path, address, _) = key();
        let (_, _, other_bytes) = key();
        let declared = Declared::Keypair(path.clone());
        let err = run(
            Some(&declared),
            Some((path, address)),
            OnChain::Upgradeable(other_bytes),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("is controlled by"), "{err}");
    }

    #[test]
    fn a_declared_address_pins_the_authority() {
        let (_, address, bytes) = key();
        let (_, _, other_bytes) = key();
        let declared = Declared::Address(address.clone());
        let want = |deployed| Signer::Remote {
            address: address.clone(),
            kind: Kind::Key,
            deployed,
        };
        assert_eq!(
            run(Some(&declared), None, OnChain::Upgradeable(bytes)).unwrap(),
            want(true)
        );
        assert_eq!(
            run(Some(&declared), None, OnChain::NotDeployed).unwrap(),
            want(false)
        );
        let err = run(Some(&declared), None, OnChain::Upgradeable(other_bytes))
            .unwrap_err()
            .to_string();
        assert!(err.contains("declares upgrade authority"), "{err}");
    }

    #[test]
    fn immutable_programs_and_program_derived_authorities() {
        assert!(run(None, None, OnChain::Immutable).is_err());
        let (pda, _) = solana::find_program_address(&[b"vault"], &[7; 32]);
        assert!(!is_on_curve(&pda));
        let got = run(None, None, OnChain::Upgradeable(pda)).unwrap();
        assert_eq!(
            got,
            Signer::Remote {
                address: solana::b58(&pda),
                kind: Kind::Derived { owner: None },
                deployed: true
            }
        );
    }

    #[test]
    fn manifest_values_are_addresses_unless_a_file_is_there() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Solex.toml"),
            format!("[channels.dev]\nprogram_id = \"{PROGRAM}\"\n"),
        )
        .unwrap();
        let project = Project::load(&dir.path().join("Solex.toml")).unwrap();
        let (_, address, _) = key();
        assert_eq!(
            declared(&project, Path::new(&address)),
            Declared::Address(address.clone())
        );
        assert!(matches!(
            declared(&project, Path::new("keys/auth.json")),
            Declared::Keypair(_)
        ));
        // A file named like an address is still a file.
        std::fs::write(dir.path().join(&address), "[]").unwrap();
        assert!(matches!(
            declared(&project, Path::new(&address)),
            Declared::Keypair(_)
        ));
    }
}

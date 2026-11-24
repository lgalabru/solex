//! Who signs an upgrade. `upgrade_authority` in the manifest is a keypair
//! file, a bare address or a Squads multisig; the chain says who the
//! authority of a deployed program really is. Together they decide whether
//! this machine can sign, the signature has to come from somewhere else, or
//! the program is still waiting to be handed over.

use std::path::{Path, PathBuf};

use crate::{
    CoreError, Project, Result, keys, program_metadata,
    solana::{self, Pubkey, Rpc},
    squads,
};

/// `upgrade_authority` as written in the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Declared {
    /// A keypair file this machine signs with.
    Keypair(PathBuf),
    /// An address whose key is held elsewhere. Pins the authority: a deployed
    /// program with another one is refused.
    Address(String),
    /// `squads:<multisig>`: the vault of a Squads multisig. Pins the
    /// authority like an address does, and can be verified on the cluster.
    Squads(squads::Reference),
}

impl Declared {
    /// The address this declaration pins the authority to, when it names
    /// one rather than a keypair file.
    pub fn pinned(&self) -> Result<Option<String>> {
        Ok(match self {
            Declared::Keypair(_) => None,
            Declared::Address(a) => Some(a.clone()),
            Declared::Squads(r) => Some(solana::b58(&r.vault()?)),
        })
    }
}

/// Read a manifest value: `squads:<multisig>` is a multisig, an existing
/// file is a keypair, as is anything that does not parse as an address.
pub fn declared(project: &Project, raw: &Path) -> Result<Declared> {
    if let Some(rest) = raw.to_str().and_then(|s| s.strip_prefix(squads::SCHEME)) {
        return Ok(Declared::Squads(squads::Reference::parse(rest)?));
    }
    let path = project.resolve_path(raw);
    let address = raw
        .to_str()
        .filter(|s| keys::validate_pubkey(s).is_ok() && !path.is_file());
    Ok(match address {
        Some(a) => Declared::Address(a.to_string()),
        None => Declared::Keypair(path),
    })
}

/// The keypair file a manifest value names, if it names one.
pub fn declared_keypair(project: &Project, raw: &Path) -> Result<Option<PathBuf>> {
    Ok(match declared(project, raw)? {
        Declared::Keypair(p) => Some(p),
        Declared::Address(_) | Declared::Squads(_) => None,
    })
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
    /// The vault of this Squads multisig, as declared in the manifest and
    /// read from the cluster.
    Squads(squads::Multisig),
}

impl Kind {
    fn describe(&self) -> String {
        match self {
            Kind::Key => "no local keypair; signs the transaction offline".to_string(),
            Kind::Derived { owner: Some(owner) } => format!("program-derived, owned by {owner}"),
            Kind::Derived { owner: None } => "program-derived".to_string(),
            Kind::Squads(m) => format!(
                "vault of Squads multisig {}, {}",
                solana::b58(&m.address),
                m.describe()
            ),
        }
    }
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
    /// This keypair controls the program (or will deploy it first, when
    /// `deployed` is false), but the manifest declares `to`: the authority
    /// has not been handed over yet.
    Outgoing {
        path: PathBuf,
        address: String,
        to: String,
        kind: Kind,
        deployed: bool,
    },
}

impl Signer {
    pub fn address(&self) -> &str {
        match self {
            Signer::Local { address, .. }
            | Signer::Remote { address, .. }
            | Signer::Outgoing { address, .. } => address,
        }
    }

    /// One line for a human: what the address is and how it signs.
    pub fn describe(&self) -> String {
        match self {
            Signer::Local { path, address } => format!("{address} ({})", path.display()),
            Signer::Remote { address, kind, .. } => format!("{address} ({})", kind.describe()),
            Signer::Outgoing {
                path, address, to, ..
            } => format!("{address} ({}), to be handed over to {to}", path.display()),
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
        (_, l) => l,
    };
    let local = local
        .filter(|p| p.is_file())
        .map(|p| Ok::<_, CoreError>((p.to_path_buf(), keys::pubkey_of(p)?)))
        .transpose()?;
    // Read on every deploy, not only on a handover: a multisig that is not
    // on this cluster is a mistake worth catching before anything is sent.
    let multisig = match declared {
        Some(Declared::Squads(r)) => Some((
            r.vault()?,
            squads::fetch(rpc, &solana::pubkey(&r.multisig)?)?,
        )),
        _ => None,
    };
    decide(program_id, declared, local, chain, |a| match &multisig {
        Some((vault, m)) if vault == a => Ok(Kind::Squads(m.clone())),
        _ => kind(rpc, a),
    })
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
    let pinned = declared.map(Declared::pinned).transpose()?.flatten();
    match chain {
        OnChain::Immutable => Err(CoreError::message(format!(
            "{program_id} is immutable (no upgrade authority); it cannot be upgraded"
        ))),
        OnChain::NotDeployed => match (declared, local) {
            // A vault cannot sign a first deploy: a local key deploys, then
            // hands the program over.
            (Some(Declared::Squads(_)), Some((path, address))) => {
                let to = pinned.expect("a multisig pins its vault");
                let kind = kind(&solana::pubkey(&to)?)?;
                Ok(Signer::Outgoing {
                    path,
                    address,
                    to,
                    kind,
                    deployed: false,
                })
            }
            (Some(Declared::Squads(_)), None) => Err(CoreError::message(
                "no keypair to deploy with first: a multisig vault cannot sign a first deploy\n  \
                 Pass --keypair; the program is handed to the multisig afterwards.",
            )),
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
            match (pinned, declared, local) {
                (Some(to), _, _) if to == actual => remote(actual, true),
                // The declared authority is not in control yet, and the key
                // that is sits on this machine: a handover is pending.
                (Some(to), _, Some((path, address))) if address == actual => {
                    let kind = kind(&solana::pubkey(&to)?)?;
                    Ok(Signer::Outgoing {
                        path,
                        address,
                        to,
                        kind,
                        deployed: true,
                    })
                }
                (Some(to), _, _) => Err(CoreError::message(format!(
                    "the manifest declares upgrade authority {to}, but {program_id} is \
                     controlled by {actual}\n  Nothing was sent. Fix `upgrade_authority` if \
                     the authority was handed over on purpose."
                ))),
                (None, _, Some((path, address))) if address == actual => {
                    Ok(Signer::Local { path, address })
                }
                // A declared keypair that is not the authority is a mistake;
                // an undeclared one just means the key lives elsewhere.
                (None, Some(Declared::Keypair(_)), Some((path, address))) => {
                    Err(CoreError::message(format!(
                        "{} is the keypair of {address}, but {program_id} is controlled by \
                         {actual}\n  Nothing was sent. Point `upgrade_authority` at the right \
                         keypair, or at the address `{actual}` if its key is held elsewhere.",
                        path.display()
                    )))
                }
                (None, Some(Declared::Keypair(p)), None) => Err(CoreError::message(format!(
                    "upgrade authority keypair {} does not exist\n  {program_id} is controlled \
                     by {actual}.",
                    p.display()
                ))),
                (None, _, _) => remote(actual, true),
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
            declared(&project, Path::new(&address)).unwrap(),
            Declared::Address(address.clone())
        );
        assert!(matches!(
            declared(&project, Path::new("keys/auth.json")).unwrap(),
            Declared::Keypair(_)
        ));
        let multisig = declared(&project, Path::new(&format!("squads:{address}/2"))).unwrap();
        assert!(matches!(&multisig, Declared::Squads(r) if r.vault_index == 2));
        assert!(declared(&project, Path::new("squads:nope")).is_err());
        // A file named like an address is still a file.
        std::fs::write(dir.path().join(&address), "[]").unwrap();
        assert!(matches!(
            declared(&project, Path::new(&address)).unwrap(),
            Declared::Keypair(_)
        ));
    }

    #[test]
    fn a_pinned_authority_still_held_by_a_local_key_is_outgoing() {
        let (path, address, bytes) = key();
        let (_, other, _) = key();
        let (_, _, stranger) = key();
        let pin = Declared::Address(other.clone());
        let local = Some((path.clone(), address.clone()));
        let got = run(Some(&pin), local.clone(), OnChain::Upgradeable(bytes)).unwrap();
        assert_eq!(
            got,
            Signer::Outgoing {
                path: path.clone(),
                address: address.clone(),
                to: other,
                kind: Kind::Key,
                deployed: true
            }
        );
        // Held by a key that is not here: still a mismatch, not a handover.
        let err = run(Some(&pin), local.clone(), OnChain::Upgradeable(stranger))
            .unwrap_err()
            .to_string();
        assert!(err.contains("declares upgrade authority"), "{err}");

        // A multisig cannot sign a first deploy, so a local key goes first.
        let reference = squads::Reference::parse(&address).unwrap();
        let vault = solana::b58(&reference.vault().unwrap());
        let multisig = Declared::Squads(reference);
        let got = run(Some(&multisig), local, OnChain::NotDeployed).unwrap();
        assert!(
            matches!(&got, Signer::Outgoing { to, deployed: false, .. } if *to == vault),
            "{got:?}"
        );
        assert!(run(Some(&multisig), None, OnChain::NotDeployed).is_err());
    }
}

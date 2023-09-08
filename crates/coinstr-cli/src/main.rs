// Copyright (c) 2022-2023 Coinstr
// Distributed under the MIT software license

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use clap::Parser;
use cli::{AddCommand, ConfigCommand, ConnectCommand, SetCommand};
use coinstr_sdk::config::Config;
use coinstr_sdk::core::bips::bip39::Mnemonic;
use coinstr_sdk::core::bitcoin::Network;
use coinstr_sdk::core::signer::{Signer, SignerType};
use coinstr_sdk::core::types::Priority;
use coinstr_sdk::core::{Amount, CompletedProposal, FeeRate, Keychain, Result};
use coinstr_sdk::db::model::{GetPolicy, GetProposal};
use coinstr_sdk::nostr::Metadata;
use coinstr_sdk::types::Label;
use coinstr_sdk::util::format;
use coinstr_sdk::{logger, Coinstr};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

mod cli;
mod util;

use crate::cli::batch::BatchCommand;
use crate::cli::{
    io, Cli, CliCommand, Command, DeleteCommand, GetCommand, ProofCommand, SettingCommand,
    ShareCommand,
};

fn base_path() -> Result<PathBuf> {
    let path = dirs::home_dir()
        .expect("Imposible to get the HOME dir")
        .join(".coinstr");
    std::fs::create_dir_all(path.as_path())?;
    Ok(path)
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("{e}");
    }
}

async fn run() -> Result<()> {
    let args = Cli::parse();
    let network: Network = args.network.into();
    let base_path: PathBuf = base_path()?;

    logger::init(base_path.clone(), network, false)?;

    match args.command {
        CliCommand::Generate {
            name,
            word_count,
            passphrase,
        } => {
            let password_from_env: Option<String> = io::get_password_from_env();
            let confirm_password_from_env: Option<String> = password_from_env.clone();

            let coinstr = Coinstr::generate(
                base_path,
                name,
                || {
                    if let Some(password) = password_from_env {
                        Ok(password)
                    } else {
                        io::get_password()
                    }
                },
                || {
                    if let Some(password) = confirm_password_from_env {
                        Ok(password)
                    } else {
                        io::get_confirmation_password()
                    }
                },
                word_count.into(),
                || {
                    if let Some(passphrase) = passphrase {
                        Ok(Some(passphrase))
                    } else if io::ask("Do you want to use a passphrase?")? {
                        Ok(Some(io::get_input("Passphrase")?))
                    } else {
                        Ok(None)
                    }
                },
                network,
            )
            .await?;
            let keychain: Keychain = coinstr.keychain();

            println!("\n!!! WRITE DOWN YOUR MNEMONIC !!!");
            println!("\n################################################################\n");
            println!("{}", keychain.seed.mnemonic());
            println!("\n################################################################\n");

            Ok(())
        }
        CliCommand::Restore { name } => {
            Coinstr::restore(
                base_path,
                name,
                io::get_password,
                io::get_confirmation_password,
                || Ok(Mnemonic::from_str(&io::get_input("Mnemonic")?)?),
                || {
                    if io::ask("Do you want to use a passphrase?")? {
                        Ok(Some(io::get_input("Passphrase")?))
                    } else {
                        Ok(None)
                    }
                },
                network,
            )
            .await?;
            Ok(())
        }
        CliCommand::Open { name } => {
            let coinstr = Coinstr::open(base_path, name, io::get_password, network).await?;

            let rl = &mut DefaultEditor::new()?;

            loop {
                let readline = rl.readline("coinstr> ");
                match readline {
                    Ok(line) => {
                        let _ = rl.add_history_entry(line.as_str());
                        let mut vec: Vec<String> = cli::parser::split(&line)?;
                        vec.insert(0, String::new());
                        match Command::try_parse_from(vec) {
                            Ok(command) => {
                                if let Err(e) = handle_command(command, &coinstr).await {
                                    eprintln!("Error: {e}");
                                }
                            }
                            Err(e) => {
                                eprintln!("{e}");
                            }
                        }
                        continue;
                    }
                    Err(ReadlineError::Interrupted) => {
                        // Ctrl-C
                        continue;
                    }
                    Err(ReadlineError::Eof) => break,
                    Err(e) => {
                        eprintln!("Error: {e}");
                        break;
                    }
                }
            }

            coinstr.shutdown().await?;

            Ok(())
        }
        CliCommand::Batch { name, path } => {
            let coinstr = Coinstr::open(base_path, name, io::get_password, network).await?;

            let file = File::open(path)?;
            let reader = BufReader::new(file);

            for line in reader.lines().flatten() {
                let mut vec: Vec<String> = cli::parser::split(&line)?;
                vec.insert(0, String::new());
                println!("{line}");
                match BatchCommand::try_parse_from(vec) {
                    Ok(command) => {
                        if let Err(e) = handle_command(command.into(), &coinstr).await {
                            eprintln!("Error: {e}");
                        }
                    }
                    Err(e) => {
                        eprintln!("{e}");
                    }
                }
            }

            println!("Shutting down...");
            coinstr.shutdown().await?;

            Ok(())
        }
        CliCommand::List => {
            let names: Vec<String> = Coinstr::list_keychains(base_path, network)?;
            for (index, name) in names.iter().enumerate() {
                println!("{}. {name}", index + 1);
            }
            Ok(())
        }
        CliCommand::Config { command } => match command {
            ConfigCommand::View => {
                let config = Config::try_from_file(base_path, network)?;
                println!("{}", config.as_pretty_json().await?);
                Ok(())
            }
            ConfigCommand::Set {
                electrum_server,
                proxy,
                block_explorer,
            } => {
                let config = Config::try_from_file(base_path, network)?;

                if let Some(endpoint) = electrum_server {
                    config.set_electrum_endpoint(Some(endpoint)).await;
                }

                if let Some(proxy) = proxy {
                    config.set_proxy(Some(proxy)).await;
                }

                if let Some(block_explorer) = block_explorer {
                    config.set_block_explorer(Some(block_explorer)).await;
                }

                config.save().await?;

                Ok(())
            }
            ConfigCommand::Unset {
                electrum_server,
                proxy,
                block_explorer,
            } => {
                let config = Config::try_from_file(base_path, network)?;

                if electrum_server {
                    config.set_electrum_endpoint::<String>(None).await;
                }

                if proxy {
                    config.set_proxy(None).await;
                }

                if block_explorer {
                    config.set_block_explorer(None).await;
                }

                config.save().await?;

                Ok(())
            }
        },
    }
}

async fn handle_command(command: Command, coinstr: &Coinstr) -> Result<()> {
    match command {
        Command::Inspect => {
            let keychain = coinstr.keychain();
            util::print_secrets(keychain, coinstr.network())
        }
        Command::Spend {
            policy_id,
            to_address,
            amount,
            description,
            target_blocks,
        } => {
            let GetProposal { proposal_id, .. } = coinstr
                .spend(
                    policy_id,
                    to_address,
                    Amount::Custom(amount),
                    description,
                    FeeRate::Priority(Priority::Custom(target_blocks)),
                    None,
                    None,
                    false,
                )
                .await?;
            println!("Spending proposal {proposal_id} sent");
            Ok(())
        }
        Command::SpendAll {
            policy_id,
            to_address,
            description,
            target_blocks,
        } => {
            let GetProposal { proposal_id, .. } = coinstr
                .spend(
                    policy_id,
                    to_address,
                    Amount::Max,
                    description,
                    FeeRate::Priority(Priority::Custom(target_blocks)),
                    None,
                    None,
                    false,
                )
                .await?;
            println!("Spending proposal {proposal_id} sent");
            Ok(())
        }
        Command::Approve { proposal_id } => {
            let (event_id, _) = coinstr.approve(proposal_id).await?;
            println!("Proposal {proposal_id} approved: {event_id}");
            Ok(())
        }
        Command::Finalize { proposal_id } => {
            let completed_proposal: CompletedProposal = coinstr.finalize(proposal_id).await?;

            match completed_proposal {
                CompletedProposal::Spending { tx, .. } => {
                    let txid = tx.txid();

                    println!("Transaction {txid} broadcasted");

                    match coinstr.network() {
                        Network::Bitcoin => {
                            println!("\nExplorer: https://blockstream.info/tx/{txid} \n")
                        }
                        Network::Testnet => {
                            println!("\nExplorer: https://blockstream.info/testnet/tx/{txid} \n")
                        }
                        _ => (),
                    };
                }
                CompletedProposal::ProofOfReserve { .. } => println!("Proof of Reserve finalized"),
            };

            Ok(())
        }
        Command::Rebroadcast => {
            coinstr.rebroadcast_all_events().await?;
            Ok(())
        }
        Command::Proof { command } => match command {
            ProofCommand::New { policy_id, message } => {
                let (proposal_id, ..) = coinstr.new_proof_proposal(policy_id, message).await?;
                println!("Proof of Reserve proposal {proposal_id} sent");
                Ok(())
            }
            ProofCommand::Verify { proposal_id } => {
                let spendable = coinstr.verify_proof_by_id(proposal_id).await?;
                println!(
                    "Valid Proof - Spendable amount: {} sat",
                    format::number(spendable)
                );
                Ok(())
            }
        },
        Command::Connect { command } => match command {
            ConnectCommand::New { uri } => {
                coinstr.new_nostr_connect_session(uri).await?;
                Ok(())
            }
            ConnectCommand::Disconnect { app_public_key } => {
                coinstr
                    .disconnect_nostr_connect_session(app_public_key)
                    .await?;
                Ok(())
            }
            ConnectCommand::Sessions => {
                let sessions = coinstr.get_nostr_connect_sessions()?;
                util::print_sessions(sessions);
                Ok(())
            }
            ConnectCommand::Requests { approved } => {
                let requests = coinstr.get_nostr_connect_requests(approved)?;
                util::print_requests(requests)?;
                Ok(())
            }
            ConnectCommand::Approve { request_id } => {
                coinstr.approve_nostr_connect_request(request_id).await?;
                Ok(())
            }
            ConnectCommand::Autoapprove {
                app_public_key,
                seconds,
            } => {
                coinstr
                    .auto_approve_nostr_connect_requests(
                        app_public_key,
                        Duration::from_secs(seconds),
                    )
                    .await;
                Ok(())
            }
            ConnectCommand::Authorizations => {
                let authorizations = coinstr.get_nostr_connect_pre_authorizations().await;
                util::print_authorizations(authorizations);
                Ok(())
            }
            ConnectCommand::Revoke { app_public_key } => {
                coinstr
                    .revoke_nostr_connect_auto_approve(app_public_key)
                    .await;
                Ok(())
            }
        },
        Command::Add { command } => match command {
            AddCommand::Relay { url, proxy } => {
                coinstr.add_relay(url, proxy).await?;
                Ok(())
            }
            AddCommand::Contact { public_key } => {
                coinstr.add_contact(public_key).await?;
                Ok(())
            }
            AddCommand::Policy {
                name,
                description,
                descriptor,
                nostr_pubkeys,
            } => {
                let policy_id = coinstr
                    .save_policy(name, description, descriptor, nostr_pubkeys)
                    .await?;
                println!("Policy saved: {policy_id}");
                Ok(())
            }
            AddCommand::CoinstrSigner {
                share_with_contacts,
            } => {
                let signer_id = coinstr.save_coinstr_signer().await?;
                if share_with_contacts {
                    for public_key in coinstr.get_contacts()?.into_keys() {
                        coinstr.share_signer(signer_id, public_key).await?;
                    }
                }
                Ok(())
            }
            AddCommand::Signer {
                name,
                fingerprint,
                descriptor,
                share_with_contacts,
            } => {
                let signer = Signer::new(name, None, fingerprint, descriptor, SignerType::AirGap)?;
                let signer_id = coinstr.save_signer(signer).await?;
                if share_with_contacts {
                    for public_key in coinstr.get_contacts()?.into_keys() {
                        coinstr.share_signer(signer_id, public_key).await?;
                    }
                }
                Ok(())
            }
        },
        Command::Get { command } => match command {
            GetCommand::Contacts => {
                let contacts = coinstr.get_contacts()?;
                util::print_contacts(contacts);
                Ok(())
            }
            GetCommand::Policies => {
                let policies = coinstr.get_policies().await?;
                util::print_policies(policies);
                Ok(())
            }
            GetCommand::Policy { policy_id, export } => {
                // Get policy
                let GetPolicy { policy, .. } = coinstr.get_policy_by_id(policy_id)?;

                // Print result
                if export {
                    println!("\n{}\n", policy.descriptor);
                    Ok(())
                } else {
                    let item = policy.satisfiable_item(coinstr.network())?;
                    let balance = coinstr.get_balance(policy_id).await;
                    let address = coinstr.get_last_unused_address(policy_id).await?;
                    let txs = coinstr.get_txs(policy_id, true).await.unwrap_or_default();
                    let utxos = coinstr.get_utxos(policy_id).await.unwrap_or_default();
                    util::print_policy(policy, policy_id, item, balance, address, txs, utxos);
                    Ok(())
                }
            }
            GetCommand::Proposals { completed } => {
                if completed {
                    let proposals = coinstr.get_completed_proposals()?;
                    util::print_completed_proposals(proposals);
                } else {
                    let proposals = coinstr.get_proposals().await?;
                    util::print_proposals(proposals);
                }
                Ok(())
            }
            GetCommand::Proposal { proposal_id } => {
                let proposal = coinstr.get_proposal_by_id(proposal_id)?;
                util::print_proposal(proposal);
                Ok(())
            }
            GetCommand::Signers => {
                let signers = coinstr.get_signers()?;
                util::print_signers(signers);
                Ok(())
            }
            GetCommand::Relays => {
                let relays = coinstr.relays().await;
                util::print_relays(relays).await;
                Ok(())
            }
            GetCommand::Addresses { policy_id } => {
                let addresses = coinstr.get_addresses(policy_id).await?;
                let balances = coinstr.get_addresses_balances(policy_id).await?;
                util::print_addresses(addresses, balances);
                Ok(())
            }
        },
        Command::Set { command } => match command {
            SetCommand::Metadata {
                name,
                display_name,
                nip05,
                empty,
            } => {
                let mut metadata = Metadata::new();
                metadata.name = name;
                metadata.display_name = display_name;
                metadata.nip05 = nip05;

                if metadata != Metadata::default() || empty {
                    coinstr.set_metadata(metadata).await?;
                } else {
                    println!("No metadata passed with args! If you want to set empty metadata, use --empty flag");
                }

                Ok(())
            }
            SetCommand::Label {
                policy_id,
                data,
                text,
            } => {
                let label = Label::new(data, text);
                let event_id = coinstr.save_label(policy_id, label).await?;
                println!("Label saved at event {event_id}");
                Ok(())
            }
        },
        Command::Share { command } => match command {
            ShareCommand::Signer {
                signer_id,
                public_key,
            } => {
                let shared_signer_id = coinstr.share_signer(signer_id, public_key).await?;
                println!(
                    "Signer {} shared with {}",
                    coinstr_sdk::util::cut_event_id(signer_id),
                    coinstr_sdk::util::cut_public_key(public_key)
                );
                println!("Shared Signer ID: {shared_signer_id}");
                Ok(())
            }
        },
        Command::Delete { command } => match command {
            DeleteCommand::Relay { url } => {
                coinstr.remove_relay(url).await?;
                Ok(())
            }
            DeleteCommand::Policy { policy_id } => {
                Ok(coinstr.delete_policy_by_id(policy_id).await?)
            }
            DeleteCommand::Proposal {
                proposal_id,
                completed,
            } => {
                if completed {
                    Ok(coinstr.delete_completed_proposal_by_id(proposal_id).await?)
                } else {
                    Ok(coinstr.delete_proposal_by_id(proposal_id).await?)
                }
            }
            DeleteCommand::Approval { approval_id } => {
                coinstr.revoke_approval(approval_id).await?;
                Ok(())
            }
            DeleteCommand::Signer { signer_id } => {
                Ok(coinstr.delete_signer_by_id(signer_id).await?)
            }
            DeleteCommand::SharedSigner { shared_signer_id } => {
                Ok(coinstr.revoke_shared_signer(shared_signer_id).await?)
            }
            DeleteCommand::Cache => Ok(coinstr.clear_cache().await?),
        },
        Command::Setting { command } => match command {
            SettingCommand::Rename { new_name } => Ok(coinstr.rename(new_name)?),
            SettingCommand::ChangePassword => Ok(coinstr.change_password(
                io::get_password,
                io::get_new_password,
                io::get_confirmation_password,
            )?),
        },
        Command::Exit => std::process::exit(0x01),
    }
}

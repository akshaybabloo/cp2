use crate::config::{self, RemoteConfig};
use std::io::{self, Write};

/// Dispatches `cp2 config <subcommand> [args...]`.
pub(crate) async fn run(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("create") => match args.get(1) {
            Some(name) => create(name),
            None => {
                eprintln!("Usage: cp2 config create <name>");
                std::process::exit(1);
            }
        },
        Some("list") => list(),
        Some("delete") => match args.get(1) {
            Some(name) => delete(name),
            None => {
                eprintln!("Usage: cp2 config delete <name>");
                std::process::exit(1);
            }
        },
        _ => {
            eprintln!(
                "Usage:\n  cp2 config create <name>   Create a new remote configuration\n  cp2 config list            List all configured remotes\n  cp2 config delete <name>   Remove a remote configuration"
            );
            std::process::exit(1);
        }
    }
}

fn prompt(label: &str, default: &str) -> String {
    if default.is_empty() {
        print!("{}: ", label);
    } else {
        print!("{} [{}]: ", label, default);
    }
    io::stdout().flush().ok();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap_or(0);
    let trimmed = input.trim().to_string();
    if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed
    }
}

fn create(name: &str) {
    println!("Creating remote \"{}\"", name);
    println!("Only S3-compatible remotes are supported.\n");

    let provider = prompt("Provider (e.g. AWS, Minio, DigitalOcean)", "AWS");
    let access_key_id = prompt("Access key ID", "");
    let secret_access_key = rpassword::prompt_password("Secret access key: ").unwrap_or_default();
    let region = prompt("Region", "us-east-1");
    let endpoint = prompt("Endpoint URL (leave blank for AWS S3)", "");

    let mut cfg = config::load_config().unwrap_or_default();

    cfg.insert(
        name.to_string(),
        RemoteConfig {
            remote_type: "s3".to_string(),
            provider: Some(provider),
            access_key_id: Some(access_key_id),
            secret_access_key: Some(secret_access_key),
            region: Some(region),
            endpoint: if endpoint.is_empty() {
                None
            } else {
                Some(endpoint)
            },
        },
    );

    match config::save_config(&cfg) {
        Ok(_) => println!(
            "\nRemote \"{}\" saved to {}",
            name,
            config::config_path().display()
        ),
        Err(e) => {
            eprintln!("Failed to save configuration: {}", e);
            std::process::exit(1);
        }
    }
}

fn list() {
    let cfg = match config::load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    };

    if cfg.is_empty() {
        println!("No remotes configured. Use `cp2 config create <name>` to add one.");
        return;
    }

    let mut names: Vec<&String> = cfg.keys().collect();
    names.sort();
    for name in names {
        let r = &cfg[name];
        let provider = r.provider.as_deref().unwrap_or("unknown");
        let region = r.region.as_deref().unwrap_or("unknown");
        println!(
            "{:<20} type={} provider={} region={}",
            name, r.remote_type, provider, region
        );
    }
}

fn delete(name: &str) {
    let mut cfg = match config::load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    };

    if cfg.remove(name).is_none() {
        eprintln!("Remote \"{}\" not found.", name);
        std::process::exit(1);
    }

    match config::save_config(&cfg) {
        Ok(_) => println!("Remote \"{}\" deleted.", name),
        Err(e) => {
            eprintln!("Failed to save configuration: {}", e);
            std::process::exit(1);
        }
    }
}

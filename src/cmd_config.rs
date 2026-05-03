use crate::cli::ConfigAction;
use crate::config::{self, RemoteConfig};
use std::io::{self, Write};

/// Dispatches a parsed `cp2 config <action>` command.
pub(crate) fn run(action: ConfigAction) {
    match action {
        ConfigAction::Create { name } => create(&name),
        ConfigAction::List => list(),
        ConfigAction::Delete { name } => delete(&name),
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
    match io::stdin().read_line(&mut input) {
        Ok(0) => {
            eprintln!("\nInput stream closed.");
            std::process::exit(1);
        }
        Ok(_) => {}
        Err(e) => {
            eprintln!("Failed to read input: {}", e);
            std::process::exit(1);
        }
    }
    let trimmed = input.trim().to_string();
    if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed
    }
}

/// Prompts until the user enters a non-empty value.
fn prompt_required(label: &str) -> String {
    loop {
        let value = prompt(label, "");
        if !value.is_empty() {
            return value;
        }
        eprintln!("This field is required.");
    }
}

fn create(name: &str) {
    println!("Creating remote \"{}\"", name);
    println!("Only S3-compatible remotes are supported.\n");

    let provider = prompt("Provider (e.g. AWS, Minio, DigitalOcean)", "AWS");
    let access_key_id = prompt_required("Access key ID");
    let secret_access_key = loop {
        let value = match rpassword::prompt_password("Secret access key: ") {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Failed to read secret access key: {}", e);
                std::process::exit(1);
            }
        };
        if !value.is_empty() {
            break value;
        }
        eprintln!("This field is required.");
    };
    let region = prompt("Region", "us-east-1");
    let endpoint = prompt("Endpoint URL (leave blank for AWS S3)", "");

    let mut cfg = match config::load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    };

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
        Ok(_) => {
            let path = config::config_path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "(unknown)".to_string());
            println!("\nRemote \"{}\" saved to {}", name, path);
        }
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
    let name_width = names.iter().map(|n| n.len()).max().unwrap_or(0);
    for name in names {
        let r = &cfg[name];
        let provider = r.provider.as_deref().unwrap_or("unknown");
        let region = r.region.as_deref().unwrap_or("unknown");
        println!(
            "{:<width$} type={} provider={} region={}",
            name,
            r.remote_type,
            provider,
            region,
            width = name_width,
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

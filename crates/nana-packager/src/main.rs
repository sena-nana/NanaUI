use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use nana_package::{ContentKey, PackReader, PublisherKey, ReadStats, TrustPolicy};
use nana_packager::layout::TargetPlatform;
use nana_packager::package::{PackageOptions, package};
use nana_packager::secrets::SecretSources;
use nana_packager::validate::{Status, ValidateOptions, validate};

const USAGE: &str = "\
nana-packager: package a Nana application (Issue #226, docs/packaging.md)

USAGE:
  nana-packager package --config nana-package.toml --exe PATH --out DIR
                [--target TRIPLE] [--baseline PREVIOUS_OUT] [--cache DIR]
                [--profile dist] [--build-id ID] [--compact]
                [--content-key-file NAME=PATH]... [--signing-key-file PATH]
                [--allow-in-tree-key]
  nana-packager validate APP_ROOT [--trust-key ed25519:HEX] [--run] [--tamper-suite]
                [--allow-resigned] [--tamper-drop-env NAME]...
                [--env KEY=VALUE]...
  nana-packager keygen content|publisher --out PATH
  nana-packager inspect PACK [--content-key-file NAME=PATH]
  nana-packager delta OLD NEW
  nana-packager macos-app --exe PATH --name NAME --identifier ID --out DIR
                [--icon ICNS] [--no-strip]
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("package") => cmd_package(&args[1..]),
        Some("validate") => cmd_validate(&args[1..]),
        Some("keygen") => cmd_keygen(&args[1..]),
        Some("inspect") => cmd_inspect(&args[1..]),
        Some("delta") => cmd_delta(&args[1..]),
        Some("macos-app") => cmd_macos_app(&args[1..]),
        Some("--help" | "-h" | "help") | None => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("nana-packager: {error}");
            ExitCode::FAILURE
        }
    }
}

/// `--flag value` pairs, repeated flags, bare switches and positionals.
struct Args {
    values: BTreeMap<String, Vec<String>>,
    switches: Vec<String>,
    positional: Vec<String>,
}

impl Args {
    fn parse(args: &[String], switches: &[&str]) -> Result<Self, String> {
        let mut parsed = Self {
            values: BTreeMap::new(),
            switches: Vec::new(),
            positional: Vec::new(),
        };
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            if switches.contains(&arg.as_str()) {
                parsed.switches.push(arg.clone());
            } else if arg.starts_with("--") {
                let value = iter
                    .next()
                    .ok_or_else(|| format!("{arg} requires a value"))?;
                parsed
                    .values
                    .entry(arg.clone())
                    .or_default()
                    .push(value.clone());
            } else {
                parsed.positional.push(arg.clone());
            }
        }
        Ok(parsed)
    }

    fn one(&self, flag: &str) -> Option<&str> {
        self.values
            .get(flag)
            .and_then(|v| v.last())
            .map(String::as_str)
    }

    fn required(&self, flag: &str) -> Result<&str, String> {
        self.one(flag).ok_or_else(|| format!("{flag} is required"))
    }

    fn all(&self, flag: &str) -> &[String] {
        self.values.get(flag).map_or(&[], Vec::as_slice)
    }

    fn has(&self, switch: &str) -> bool {
        self.switches.iter().any(|s| s == switch)
    }

    fn key_files(&self) -> Result<BTreeMap<String, PathBuf>, String> {
        self.all("--content-key-file")
            .iter()
            .map(|pair| {
                pair.split_once('=')
                    .map(|(name, path)| (name.to_owned(), PathBuf::from(path)))
                    .ok_or_else(|| format!("--content-key-file expects NAME=PATH, got `{pair}`"))
            })
            .collect()
    }
}

fn cmd_package(args: &[String]) -> Result<ExitCode, String> {
    let args = Args::parse(args, &["--compact", "--allow-in-tree-key"])?;
    let options = PackageOptions {
        config: PathBuf::from(args.required("--config")?),
        executable: PathBuf::from(args.required("--exe")?),
        target: args
            .one("--target")
            .map(str::to_owned)
            .unwrap_or_else(TargetPlatform::host_triple),
        out: PathBuf::from(args.required("--out")?),
        baseline: args.one("--baseline").map(PathBuf::from),
        cache: args.one("--cache").map(PathBuf::from),
        profile: args.one("--profile").unwrap_or("dist").to_owned(),
        build_id: args
            .one("--build-id")
            .map(str::to_owned)
            .or_else(|| std::env::var("NANA_BUILD_ID").ok()),
        compact: args.has("--compact"),
        secrets: SecretSources {
            content_key_files: args.key_files()?,
            signing_key_file: args.one("--signing-key-file").map(PathBuf::from),
            allow_in_tree: args.has("--allow-in-tree-key"),
        },
    };
    let report = package(&options)?;
    for warning in report
        .warnings
        .iter()
        .chain(report.packs.iter().flat_map(|p| &p.warnings))
    {
        eprintln!("warning: {warning}");
    }
    for pack in &report.packs {
        println!(
            "pack {}: {} entries, {} bytes (encoded {}, reused {}, cache {}/{})",
            pack.name,
            pack.entries,
            pack.pack_bytes,
            pack.encoded_bytes,
            pack.reused_bytes,
            pack.cache_hits,
            pack.cache_hits + pack.cache_misses
        );
    }
    for (name, delta) in &report.deltas {
        println!(
            "delta {name}: {}/{} chunks changed, {} bytes to download",
            delta.changed_chunks, delta.total_chunks, delta.steam_delta_bytes
        );
    }
    println!(
        "{} {} → {} ({}, {})",
        report.application,
        report.version,
        options.out.join(&report.app_root).display(),
        report.backend,
        if report.signed {
            "publisher-signed"
        } else {
            "unsigned"
        }
    );
    for signing in &report.platform_signing {
        println!("platform signing {}: {:?}", signing.adapter, signing.status);
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_validate(args: &[String]) -> Result<ExitCode, String> {
    let args = Args::parse(
        args,
        &["--run", "--tamper-suite", "--json", "--allow-resigned"],
    )?;
    let app_root = args
        .positional
        .first()
        .ok_or("validate needs the application root")?;
    let trust = match args.one("--trust-key") {
        Some(text) => {
            Some(PublisherKey::from_text(text).ok_or("--trust-key must be ed25519:<64 hex>")?)
        }
        None => None,
    };
    let run_env = args
        .all("--env")
        .iter()
        .map(|pair| {
            pair.split_once('=')
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .ok_or_else(|| format!("--env expects KEY=VALUE, got `{pair}`"))
        })
        .collect::<Result<_, _>>()?;
    let report = validate(&ValidateOptions {
        app_root: PathBuf::from(app_root),
        trust,
        run: args.has("--run"),
        tamper_suite: args.has("--tamper-suite"),
        run_env,
        allow_resigned: args.has("--allow-resigned"),
        tamper_drop_env: args.all("--tamper-drop-env").to_vec(),
    })?;
    if args.has("--json") {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).expect("report serializes")
        );
    } else {
        for check in &report.checks {
            let (label, detail) = match &check.status {
                Status::Pass => ("pass", None),
                Status::Warn(d) => ("warn", Some(d)),
                Status::Fail(d) => ("FAIL", Some(d)),
                Status::NotExecuted(d) => ("not executed", Some(d)),
            };
            match detail {
                Some(detail) => println!("{label:>12}  {}: {detail}", check.name),
                None => println!("{label:>12}  {}", check.name),
            }
        }
    }
    Ok(if report.failed() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn cmd_keygen(args: &[String]) -> Result<ExitCode, String> {
    let args = Args::parse(args, &[])?;
    let kind = args
        .positional
        .first()
        .ok_or("keygen needs `content` or `publisher`")?;
    let out = PathBuf::from(args.required("--out")?);
    if out.exists() {
        return Err(format!(
            "{} exists; refusing to overwrite a key",
            out.display()
        ));
    }
    let mut seed = zeroize::Zeroizing::new([0u8; 32]);
    getrandom::fill(&mut seed[..]).map_err(|error| format!("system RNG failed: {error}"))?;
    write_secret(&out, &nana_package::to_hex(seed.as_ref()))?;
    match kind.as_str() {
        "content" => println!("content key written to {}", out.display()),
        "publisher" => {
            let signing = ed25519_dalek::SigningKey::from_bytes(&seed);
            let public = PublisherKey::from_bytes(signing.verifying_key().as_bytes())
                .expect("derived key is valid");
            println!("publisher signing key written to {}", out.display());
            println!(
                "public key (put in signing.publisher.public_key): {}",
                public.to_text()
            );
        }
        other => {
            let _ = std::fs::remove_file(&out);
            return Err(format!("unknown key kind `{other}`"));
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn write_secret(path: &PathBuf, text: &str) -> Result<(), String> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("cannot create {}: {error}", path.display()))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.write_all(b"\n"))
        .map_err(|error| format!("cannot write {}: {error}", path.display()))
}

fn cmd_inspect(args: &[String]) -> Result<ExitCode, String> {
    let args = Args::parse(args, &[])?;
    let path = PathBuf::from(args.positional.first().ok_or("inspect needs a pack")?);
    // The generation is unknown here: match keys by id alone.
    struct AnyGeneration(Vec<(nana_package::KeyId, ContentKey)>);
    impl nana_package::KeyProvider for AnyGeneration {
        fn content_key(
            &self,
            request: &nana_package::KeyRequest<'_>,
        ) -> Result<ContentKey, nana_package::KeyError> {
            self.0
                .iter()
                .find(|(id, _)| *id == request.key_id)
                .map(|(_, key)| key.clone())
                .ok_or(nana_package::KeyError)
        }
    }
    let mut keys = AnyGeneration(Vec::new());
    for (name, file) in args.key_files()? {
        let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
        let key = ContentKey::from_hex(&text).ok_or("content key must be 64 hex digits")?;
        keys.0.push((nana_package::KeyId::from_name(&name), key));
    }
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("pack");
    let reader = PackReader::open(&path, name, &keys, &TrustPolicy::AllowUnsigned, None)
        .map_err(|error| error.to_string())?;
    let header = reader.header();
    println!(
        "{name}: class {}, {} entries, block {} B, {}, {}",
        reader.class().as_str(),
        reader.entry_count(),
        header.block_size,
        if header.encrypted() {
            "encrypted"
        } else {
            "plain"
        },
        if header.signed() {
            "signed"
        } else {
            "unsigned"
        }
    );
    for key in reader.keys().map_err(|e| e.to_string())? {
        let info = reader.entry(&key).map_err(|e| e.to_string())?;
        let ok = reader
            .read(&key, u64::MAX, &mut ReadStats::default())
            .is_ok();
        println!(
            "  {key}  {} B  {} blocks  {}",
            info.plain_len,
            info.block_count,
            if ok { "ok" } else { "CORRUPT" }
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_delta(args: &[String]) -> Result<ExitCode, String> {
    let args = Args::parse(args, &[])?;
    let [old, new] = args.positional.as_slice() else {
        return Err("delta needs OLD and NEW".into());
    };
    let read = |p: &String| std::fs::read(p).map_err(|e| format!("{p}: {e}"));
    let delta =
        nana_packager::delta::delta(&read(old)?, &read(new)?, nana_packager::delta::STEAM_CHUNK);
    println!(
        "{}",
        serde_json::to_string_pretty(&delta).expect("delta serializes")
    );
    Ok(ExitCode::SUCCESS)
}

/// The former `nana-package-app` command: a bare macOS bundle from an
/// executable, without resources or a manifest.
fn cmd_macos_app(args: &[String]) -> Result<ExitCode, String> {
    let args = Args::parse(args, &["--no-strip"])?;
    let path = nana_packager::macos::bare_app(&nana_packager::macos::BareApp {
        exe: PathBuf::from(args.required("--exe")?),
        name: args.required("--name")?.to_owned(),
        identifier: args.required("--identifier")?.to_owned(),
        out: PathBuf::from(args.required("--out")?),
        icon: args.one("--icon").map(PathBuf::from),
        strip: !args.has("--no-strip"),
    })?;
    println!("{}", path.display());
    Ok(ExitCode::SUCCESS)
}
